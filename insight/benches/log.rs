use std::hash::{Hash, Hasher};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use datafusion::arrow::ipc::writer::StreamWriter;
use datafusion::arrow::record_batch::RecordBatch;
use futures_util::SinkExt;
use insight::error::{ErrorKind, Result};
use tokio::net::TcpStream;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

#[path = "log/control.rs"]
mod control;
#[path = "log/data.rs"]
mod data;
#[path = "log/output.rs"]
mod output;
#[path = "log/server.rs"]
mod server;

use data::Samples;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

const DEFAULT_BYTES_PER_INDEX: u64 = 5 * 1024 * 1024 * 1024;
const BATCH_BYTES: usize = 2 * 1024 * 1024;
const RETENTION: jiff::SignedDuration = jiff::SignedDuration::from_hours(24);
const WRITER_FLUSH_INTERVAL: jiff::SignedDuration = jiff::SignedDuration::from_millis(500);

#[derive(Debug)]
struct Options {
    bytes_per_index: u64,
    row_group_bytes: usize,
    index_count: usize,
    query: bool,
    keep_files: bool,
    seed: u64,
}

impl Options {
    fn load() -> Result<Self> {
        let bytes_per_index =
            env("INSIGHT_BENCH_BYTES_PER_INDEX")?.unwrap_or(DEFAULT_BYTES_PER_INDEX);
        if bytes_per_index < 512 {
            return Err(ErrorKind::Config(
                "INSIGHT_BENCH_BYTES_PER_INDEX must be at least 512".to_owned(),
            )
            .into());
        }
        let row_group_bytes: usize =
            env("INSIGHT_BENCH_ROW_GROUP_BYTES")?.unwrap_or(8 * 1024 * 1024);
        if row_group_bytes == 0 || row_group_bytes.checked_mul(8).is_none() {
            return Err(ErrorKind::Config(format!(
                "INSIGHT_BENCH_ROW_GROUP_BYTES must be within 1..={}",
                usize::MAX / 8
            ))
            .into());
        }
        Ok(Self {
            bytes_per_index,
            row_group_bytes,
            index_count: env("INSIGHT_BENCH_INDEX_COUNT")?
                .map_or_else(std::thread::available_parallelism, Ok)?
                .get(),
            query: env("INSIGHT_BENCH_QUERY")?.unwrap_or(true),
            keep_files: env("INSIGHT_BENCH_KEEP_FILES")?.unwrap_or(false),
            seed: env("INSIGHT_BENCH_SEED")?.unwrap_or_else(random_seed),
        })
    }
}

#[derive(Debug)]
struct Report {
    bytes: u64,
    parquet_bytes: u64,
    rows: u64,
    ingest: Duration,
    drain: Duration,
    merge_wait: Duration,
    query: Option<Duration>,
    files: Option<std::path::PathBuf>,
    server_pid: u32,
    server_peak_rss_bytes: Option<u64>,
}

fn encode_ingest_frame(index_id: &str, batch: &RecordBatch) -> Result<Bytes> {
    let mut output = Vec::with_capacity(BATCH_BYTES * 2);
    insight_wire::frame::write_header(&mut output, index_id);
    {
        let mut writer = StreamWriter::try_new(&mut output, &batch.schema())?;
        writer.write(batch)?;
        writer.finish()?;
    }
    Ok(output.into())
}

async fn send_index(
    address: std::net::SocketAddr,
    index_id: String,
    samples: Arc<Samples>,
) -> Result<()> {
    let stream = TcpStream::connect(address).await?;
    stream.set_nodelay(true)?;
    let codec = LengthDelimitedCodec::builder()
        .length_field_length(4)
        .max_frame_length(insight_wire::frame::MAX_LEN)
        .new_codec();
    let mut transport = Framed::new(stream, codec);
    for batch in &samples.batches {
        transport
            .send(encode_ingest_frame(&index_id, batch)?)
            .await?;
    }
    SinkExt::<Bytes>::close(&mut transport).await?;
    Ok(())
}

fn env<T: FromStr>(name: &str) -> Result<Option<T>> {
    match std::env::var(name) {
        Ok(value) => value
            .parse()
            .map(Some)
            .map_err(|_| ErrorKind::Config(format!("{name} has invalid value {value}")).into()),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(ErrorKind::Config(error.to_string()).into()),
    }
}

fn random_seed() -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    uuid::Uuid::new_v4().hash(&mut hasher);
    hasher.finish()
}

fn indexes(count: usize) -> Vec<String> {
    (1..=count)
        .map(|index| format!("bench{index}/log"))
        .collect()
}

async fn send_all(
    address: std::net::SocketAddr,
    index_count: usize,
    samples: &Arc<Samples>,
) -> Result<()> {
    let mut tasks = tokio::task::JoinSet::new();
    for index in indexes(index_count) {
        tasks.spawn(send_index(address, index, Arc::clone(samples)));
    }
    while let Some(joined) = tasks.join_next().await {
        joined.map_err(|error| ErrorKind::Invariant(error.to_string()))??;
    }
    Ok(())
}

async fn run(
    options: &Options,
    corpus: &data::Corpus,
    child: &mut control::Server,
) -> Result<Report> {
    let samples = Arc::new(corpus.replay(options.bytes_per_index)?);
    control::send(
        &mut child.writer,
        &control::Message::RunSpec(control::RunSpec {
            rows_per_index: samples.rows,
            query_words: samples.query_words.clone(),
            query_rows: samples.query_rows,
            trace_id: samples.trace_id.clone(),
            trace_rows: samples.trace_rows,
        }),
    )
    .await?;
    output::step(2, options.query, "Ingesting indexes");
    let started = Instant::now();
    tokio::select! {
        result = send_all(child.address, options.index_count, &samples) => result?,
        message = control::receive(&mut child.reader) => {
            return Err(control::unexpected(&message?));
        }
    }
    let send_elapsed = started.elapsed();
    control::send(&mut child.writer, &control::Message::Drain).await?;
    let message = control::receive(&mut child.reader).await?;
    if !matches!(message, control::Message::Stored) {
        return Err(control::unexpected(&message));
    }
    let ingest = started.elapsed();
    output::step(3, options.query, "Waiting for merges");
    let message = control::receive(&mut child.reader).await?;
    let control::Message::Report(server) = message else {
        return Err(control::unexpected(&message));
    };
    child.wait().await?;
    let bytes = samples
        .raw_bytes
        .checked_mul(options.index_count as u64)
        .ok_or_else(|| ErrorKind::Invariant("benchmark byte count overflowed".to_owned()))?;
    let rows = samples
        .rows
        .checked_mul(options.index_count as u64)
        .ok_or_else(|| ErrorKind::Invariant("benchmark row count overflowed".to_owned()))?;
    Ok(Report {
        bytes,
        parquet_bytes: server.parquet_bytes,
        rows,
        ingest,
        drain: ingest.saturating_sub(send_elapsed),
        merge_wait: server.merge_wait,
        query: server.query,
        files: server.files,
        server_pid: child.pid,
        server_peak_rss_bytes: server.peak_rss_bytes,
    })
}

async fn controller(options: &Options) -> Result<()> {
    let mut child = control::Server::start().await?;
    output::step(1, options.query, "Preparing data");
    let corpus = data::generate(options.bytes_per_index, options.seed)?;
    let report = tokio::time::timeout(control::RUN_TIMEOUT, run(options, &corpus, &mut child))
        .await
        .map_err(|_| {
            ErrorKind::Invariant("benchmark controller exceeded 900 seconds".to_owned())
        })??;
    output::report(options, &corpus, &report)?;
    Ok(())
}

fn main() -> Result<()> {
    logging::init_with_default_level(tracing::Level::WARN)?;
    let options = Options::load()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = if std::env::args().nth(1).as_deref() == Some(control::SERVER_ARGUMENT) {
        let address = std::env::args()
            .nth(2)
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| ErrorKind::Config("invalid benchmark control address".to_owned()))?;
        runtime.block_on(server::run(&options, address))
    } else {
        runtime.block_on(controller(&options))
    };
    runtime.shutdown_timeout(Duration::from_secs(5));
    if result.is_err() {
        output::failure(options.seed);
    }
    result
}
