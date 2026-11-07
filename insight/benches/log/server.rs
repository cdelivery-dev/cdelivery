use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use datafusion::arrow::array::TimestampNanosecondArray;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::{Expr, col, contains, lit, lower};
use insight::config::Config;
use insight::error::{ErrorKind, Result};
use insight::ingest::Ingest;
use insight::storage::store::Store;
use insight_schema::log;
use insight_wire::ObjectStoreConfig;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::control::{self, Message, RunSpec, ServerReport};

fn query_filter(words: &[String; 3]) -> Expr {
    let text = lower(col(log::LOG));
    words
        .iter()
        .map(|word| contains(text.clone(), lit(word)))
        .reduce(Expr::and)
        .unwrap_or_else(|| lit(true))
}

#[derive(Debug, Default)]
struct SearchStats {
    rows: u64,
    oldest: Option<i64>,
}

fn summarize(stats: &mut SearchStats, batch: &RecordBatch) -> Result<()> {
    let timestamps = batch
        .column_by_name(insight_schema::TIMESTAMP)
        .and_then(|column| column.as_any().downcast_ref::<TimestampNanosecondArray>())
        .ok_or_else(|| {
            ErrorKind::Invariant("query result contained no timestamp column".to_owned())
        })?;
    for timestamp in timestamps.values() {
        if stats.oldest.is_some_and(|value| *timestamp > value) {
            return Err(ErrorKind::Invariant(
                "query result was not sorted by descending timestamp".to_owned(),
            )
            .into());
        }
        stats.oldest = Some(*timestamp);
    }
    stats.rows += batch.num_rows() as u64;
    Ok(())
}

async fn query_index(store: &Store, index_id: &str, spec: &RunSpec) -> Result<Duration> {
    let started = Instant::now();
    let stats = store
        .search(
            index_id,
            (i64::MIN, i64::MAX),
            Some(query_filter(&spec.query_words)),
            0,
            None,
            |stats, batch| summarize(stats, &batch),
        )
        .await?;
    let rows = stats.rows;
    if rows != spec.query_rows {
        return Err(ErrorKind::Invariant(format!(
            "query of {index_id} matched {rows} rows, expected {}",
            spec.query_rows
        ))
        .into());
    }
    if stats.oldest.is_none() {
        return Err(ErrorKind::Invariant("query result was empty".to_owned()).into());
    }
    let elapsed = started.elapsed();
    let trace_id = &spec.trace_id;
    let trace_matches = store
        .search(
            index_id,
            (i64::MIN, i64::MAX),
            Some(contains(lower(col(log::LOG)), lit(trace_id))),
            0,
            None,
            |rows: &mut u64, batch| {
                *rows += batch.num_rows() as u64;
                Ok(())
            },
        )
        .await?;
    if trace_matches != spec.trace_rows {
        return Err(ErrorKind::Invariant(format!(
            "trace {trace_id} query of {index_id} matched {trace_matches} rows, expected {}",
            spec.trace_rows
        ))
        .into());
    }
    Ok(elapsed)
}

async fn wait_until_stored(store: &Store, indexes: &[String], expected_rows: u64) -> Result<()> {
    let patience = super::WRITER_FLUSH_INTERVAL.unsigned_abs() + Duration::from_secs(15);
    let deadline = tokio::time::Instant::now() + patience;
    loop {
        if indexes
            .iter()
            .all(|index| store.benchmark_stats(index).0 == expected_rows)
        {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            let pending = indexes
                .iter()
                .map(|index| format!("{index}={}/{expected_rows}", store.benchmark_stats(index).0))
                .collect::<Vec<_>>()
                .join(",");
            return Err(ErrorKind::Invariant(format!(
                "ingested rows did not become queryable within {} seconds: {pending}",
                patience.as_secs_f64()
            ))
            .into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_until_merged(
    store: &Store,
    indexes: &[String],
    maintenance: &tokio::task::JoinHandle<()>,
) -> Result<Duration> {
    let started = Instant::now();
    loop {
        if maintenance.is_finished() {
            return Err(ErrorKind::Invariant(
                "storage runtime exited before merges completed".to_owned(),
            )
            .into());
        }
        let pending = indexes
            .iter()
            .filter(|index| store.benchmark_merge_pending(index))
            .map(String::as_str)
            .collect::<Vec<_>>();
        if pending.is_empty() {
            return Ok(started.elapsed());
        }
        if started.elapsed() >= Duration::from_secs(300) {
            return Err(ErrorKind::Invariant(format!(
                "merges did not complete within 300 seconds: {}",
                pending.join(",")
            ))
            .into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

struct Runtime {
    cancel: CancellationToken,
    ingest: tokio::task::JoinHandle<()>,
    maintenance: tokio::task::JoinHandle<()>,
}

impl Runtime {
    async fn stop(&mut self) -> Result<()> {
        self.cancel.cancel();
        let (ingest, maintenance) = tokio::join!(&mut self.ingest, &mut self.maintenance);
        ingest
            .and(maintenance)
            .map_err(|error| ErrorKind::Invariant(error.to_string()).into())
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.ingest.abort();
        self.maintenance.abort();
    }
}

async fn next(commands: &mut mpsc::Receiver<Message>) -> Result<Message> {
    commands
        .recv()
        .await
        .ok_or_else(|| ErrorKind::Invariant("benchmark controller disconnected".to_owned()).into())
}

async fn measure(
    options: &super::Options,
    root: &Path,
    writer: &mut control::Writer,
    commands: &mut mpsc::Receiver<Message>,
) -> Result<ServerReport> {
    let mut indexes = super::indexes(options.index_count);
    let config = Config {
        storage: ObjectStoreConfig::Local,
        node_id: "benchmark-node".to_owned(),
        retention: super::RETENTION,
        writer_flush_interval: super::WRITER_FLUSH_INTERVAL,
        row_group_max_bytes: options.row_group_bytes,
        ingest_memory_budget: 4 * 1024 * 1024 * 1024,
        query_memory_budget: 2 * 1024 * 1024 * 1024,
        search_partitions: 2,
        aggregation_partitions: 8,
        single_instance: false,
    };
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let address = listener.local_addr()?;
    let (store, storage_runtime) = Store::start(&config, root).await?;
    let ingest = Arc::new(Ingest::new(
        Arc::clone(&store),
        config.writer_flush_interval,
        config.ingest_memory_budget,
    ));
    let cancel = CancellationToken::new();
    let mut runtime = Runtime {
        ingest: tokio::spawn(
            Arc::clone(&ingest).listen(listener, cancel.clone().cancelled_owned()),
        ),
        maintenance: tokio::spawn(storage_runtime.run(cancel.clone())),
        cancel,
    };
    control::send(
        writer,
        &Message::Ready {
            address,
            pid: std::process::id(),
        },
    )
    .await?;
    let message = next(commands).await?;
    let Message::RunSpec(spec) = message else {
        return Err(control::unexpected(&message));
    };
    let message = next(commands).await?;
    if !matches!(message, Message::Drain) {
        return Err(control::unexpected(&message));
    }
    wait_until_stored(&store, &indexes, spec.rows_per_index).await?;
    control::send(writer, &Message::Stored).await?;
    let merge_wait = wait_until_merged(&store, &indexes, &runtime.maintenance).await?;
    runtime.stop().await?;
    let parquet_bytes = indexes
        .iter()
        .map(|index| store.benchmark_stats(index).1)
        .sum();
    let mut query = options.query.then_some(Duration::ZERO);
    if let Some(elapsed) = &mut query {
        super::output::step(4, true, "Querying indexes");
        indexes.sort_unstable();
        for index in &indexes {
            *elapsed += query_index(&store, index, &spec).await?;
        }
    }
    Ok(ServerReport {
        parquet_bytes,
        merge_wait,
        query,
        files: options.keep_files.then(|| root.to_path_buf()),
        peak_rss_bytes: None,
    })
}

#[cfg(target_os = "linux")]
fn peak_rss_bytes() -> Result<Option<u64>> {
    let status = std::fs::read_to_string("/proc/self/status")?;
    let line = status
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))
        .ok_or_else(|| ErrorKind::Invariant("/proc/self/status contained no VmHWM".to_owned()))?;
    let mut fields = line.split_whitespace();
    let kib = fields
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|_| fields.next() == Some("kB"))
        .ok_or_else(|| ErrorKind::Invariant(format!("invalid VmHWM: {line}")))?;
    kib.checked_mul(1024)
        .map(Some)
        .ok_or_else(|| ErrorKind::Invariant("VmHWM bytes overflowed".to_owned()).into())
}

#[cfg(not(target_os = "linux"))]
#[expect(clippy::unnecessary_wraps)]
fn peak_rss_bytes() -> Result<Option<u64>> {
    Ok(None)
}

pub(crate) async fn run(options: &super::Options, address: SocketAddr) -> Result<()> {
    let (mut writer, mut reader) = control::channel(TcpStream::connect(address).await?)?;
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("insight-log-bench")
        .join(uuid::Uuid::new_v4().to_string());
    let (sender, mut commands) = mpsc::channel(2);
    let monitor = async {
        loop {
            let message = control::receive(&mut reader).await?;
            sender.send(message).await.map_err(|_| {
                ErrorKind::Invariant("benchmark command receiver stopped".to_owned())
            })?;
        }
    };
    let result = tokio::select! {
        biased;
        result = measure(options, &root, &mut writer, &mut commands) => result,
        result = monitor => result,
        () = tokio::time::sleep(control::RUN_TIMEOUT) => {
            Err(ErrorKind::Invariant("benchmark server exceeded 900 seconds".to_owned()).into())
        }
    };
    let cleanup = if options.keep_files {
        if result.is_err() {
            super::output::artifact(&root);
        }
        Ok(())
    } else {
        match std::fs::remove_dir_all(&root) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    };
    let mut report = result?;
    cleanup?;
    report.peak_rss_bytes = peak_rss_bytes()?;
    control::send(&mut writer, &Message::Report(report)).await
}
