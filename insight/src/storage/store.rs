use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::num::NonZeroUsize;
use std::sync::Arc;

use datafusion::arrow::array::RecordBatch;
use datafusion::arrow::buffer::Buffer;
use datafusion::arrow::datatypes::DataType;
use datafusion::catalog::MemTable;
use datafusion::common::{cast::as_string_view_array, internal_err};
use datafusion::execution::memory_pool::{FairSpillPool, TrackConsumersPool};
use datafusion::execution::object_store::ObjectStoreUrl;
use datafusion::execution::runtime_env::{RuntimeEnv, RuntimeEnvBuilder};
use datafusion::logical_expr::{ColumnarValue, Volatility, create_udf};
use datafusion::prelude::*;
use futures_util::StreamExt;
use insight_wire::ObjectStoreConfig;
use object_store::buffered::BufWriter;
use object_store::path::Path;
use object_store::{ObjectStoreExt, PutPayload, WriteMultipart};
use parquet::arrow::AsyncArrowWriter;
use parquet::basic::{Compression, Encoding, ZstdLevel};
use parquet::file::properties::{EnabledStatistics, WriterProperties, WriterPropertiesBuilder};
use tokio_util::sync::CancellationToken;

use super::meta::{self, Chunk, Index, MERGE_L0_BATCH, MERGE_L1_TARGET, Site};
use super::{
    PARQUET_WRITER_SORT_FIELD, SORT_FIELD, SORT_TO_INDICES_OPTIONS, current_schema, query_sort,
};
use crate::config;
use crate::error::{ErrorKind, Result};
use insight_schema::{IndexKind, NODE_ID, TIMESTAMP, access, event, log, message, metric};

const SYNCER_CHANNEL_CAPACITY: usize = 1024;
const MAX_SYNC_WORKER: usize = 8;

const SPILL_DIR: &str = "cdelivery-spill";
const TEMP_PATH_SUFFIX: &str = ".temp";
const ROLL_TEMP_SUFFIX: &str = ".roll.temp";
const MERGED_TEMP_SUFFIX: &str = ".merged.temp";
const MERGE_MAX_FANIN: usize = MERGE_L1_TARGET + MERGE_L0_BATCH;
#[expect(clippy::cast_possible_truncation)]
const ROLL_MAX_TIME_SPAN: i64 = jiff::SignedDuration::from_hours(24).as_nanos() as i64;
const PARQUET_BATCH_SIZE: usize = 8 * 1024;
pub const PARQUET_MAX_ROW_GROUP_ROWS: usize = 1_000_000;
const ACCESS_BLOOM_NDV: u64 = 64;
const UPLOAD_CHUNK_SIZE: usize = 32 * 1024 * 1024;

fn rotate_required(rows: u64, size: u64, time_span_nanos: i64, max_bytes: usize) -> bool {
    rows >= PARQUET_MAX_ROW_GROUP_ROWS as u64
        || size >= max_bytes as u64
        || time_span_nanos >= ROLL_MAX_TIME_SPAN
}

fn select_reusable_roll(roll: Option<Chunk>, max_bytes: usize) -> Option<Chunk> {
    roll.filter(|r| {
        !rotate_required(
            r.stats().rows,
            r.stats().size,
            r.time_span_nanos(),
            max_bytes,
        )
    })
}

async fn list_chunks(
    os: &Arc<dyn object_store::ObjectStore>,
    prefix: Option<&Path>,
    mut stray: impl FnMut(&Path),
) -> Result<Vec<(String, Chunk)>> {
    let mut list_stream = os.list(prefix);
    let mut chunks = Vec::new();
    while let Some(entry) = list_stream.next().await {
        let entry = entry?;
        if entry.location.as_ref().split('/').next() == Some(SPILL_DIR) {
            continue;
        }
        match Chunk::parse(&entry.location, entry.size) {
            Some(parsed) => chunks.push(parsed),
            None => stray(&entry.location),
        }
    }
    Ok(chunks)
}

fn warn_stray(site: &'static str) -> impl FnMut(&Path) {
    move |path| {
        tracing::warn!(
            "unrecognized {site} object {} left for manual inspection",
            path.as_ref()
        );
    }
}

async fn delete_all(
    os: &Arc<dyn object_store::ObjectStore>,
    paths: Vec<Path>,
    what: &str,
    mut deleted: impl FnMut(String),
) {
    let mut results =
        os.delete_stream(futures_util::stream::iter(paths.into_iter().map(Ok)).boxed());
    while let Some(res) = results.next().await {
        match res {
            Ok(path) => deleted(path.as_ref().to_owned()),
            Err(object_store::Error::NotFound { path, .. }) => deleted(path),
            Err(e) => tracing::error!(error = ?e, "failed to delete {what}"),
        }
    }
}

async fn sweep_temp_files(local: &Arc<dyn object_store::ObjectStore>) {
    let mut list_stream = local.list(None);
    let mut temps = Vec::new();
    while let Some(entry) = list_stream.next().await {
        match entry {
            Ok(entry) if entry.location.as_ref().split('/').next() == Some(SPILL_DIR) => {}
            Ok(entry) if entry.location.as_ref().ends_with(TEMP_PATH_SUFFIX) => {
                temps.push(entry.location);
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(error = ?e, "failed to list local storage while sweeping temp files");
                return;
            }
        }
    }
    delete_all(local, temps, "leftover temp file", |path| {
        tracing::info!("swept leftover temp file {path}");
    })
    .await;
}

type StaleEntry = (String, uuid::Uuid, Path);
type WrittenFile = (u64, usize, Option<(i64, i64)>);

async fn delete_stale(
    os: &Arc<dyn object_store::ObjectStore>,
    stales: Vec<StaleEntry>,
    site: &str,
) -> Vec<StaleEntry> {
    let paths = stales.iter().map(|(_, _, path)| path.clone()).collect();
    let mut remaining = stales;
    let mut acks = Vec::with_capacity(remaining.len());
    delete_all(os, paths, &format!("stale {site} file"), |deleted| {
        if let Some(pos) = remaining
            .iter()
            .position(|(_, _, path)| deleted.ends_with(path.as_ref()))
        {
            acks.push(remaining.swap_remove(pos));
        }
    })
    .await;
    acks
}

fn base_writer(row_group_max_bytes: usize) -> WriterPropertiesBuilder {
    WriterProperties::builder()
        .set_max_row_group_row_count(Some(PARQUET_MAX_ROW_GROUP_ROWS))
        .set_max_row_group_bytes(Some(row_group_max_bytes))
        .set_compression(Compression::ZSTD(ZstdLevel::default()))
        .set_sorting_columns(Some(vec![PARQUET_WRITER_SORT_FIELD]))
        .set_bloom_filter_enabled(false)
        .set_dictionary_enabled(true)
        .set_column_dictionary_enabled(TIMESTAMP.into(), false)
        .set_column_encoding(TIMESTAMP.into(), Encoding::DELTA_BINARY_PACKED)
        .set_data_page_size_limit(1024 * 1024)
}

fn writer_properties(kind: Option<IndexKind>, row_group_max_bytes: usize) -> WriterProperties {
    let (delta_columns, text_column): (&[&str], Option<&str>) = match kind {
        Some(IndexKind::HostMetric) => (
            &[
                metric::NODE_UPTIME_MS,
                metric::CPU_USER_TICKS,
                metric::CPU_SYSTEM_TICKS,
                metric::CPU_IDLE_TICKS,
                metric::CPU_IOWAIT_TICKS,
                metric::CPU_STEAL_TICKS,
                metric::PSI_CPU_SOME_USEC_TOTAL,
                metric::PSI_MEMORY_SOME_USEC_TOTAL,
                metric::PSI_MEMORY_FULL_USEC_TOTAL,
                metric::PSI_IO_SOME_USEC_TOTAL,
                metric::PSI_IO_FULL_USEC_TOTAL,
                metric::NETWORK_RECEIVE_BYTES_TOTAL,
                metric::NETWORK_TRANSMIT_BYTES_TOTAL,
            ],
            None,
        ),
        Some(IndexKind::ContainerMetric) => (
            &[
                metric::NODE_UPTIME_MS,
                metric::CPU_USAGE_USEC_TOTAL,
                metric::PSI_CPU_SOME_USEC_TOTAL,
                metric::NETWORK_RECEIVE_BYTES_TOTAL,
                metric::NETWORK_TRANSMIT_BYTES_TOTAL,
            ],
            None,
        ),
        Some(IndexKind::Log) => (&[], Some(log::LOG)),
        Some(IndexKind::Event) => (&[], Some(event::DETAIL)),
        Some(IndexKind::Message) => (&[], Some(message::TEXT)),
        Some(IndexKind::PingressAccess) => return access_writer_properties(row_group_max_bytes),
        Some(IndexKind::FilesystemMetric) | None => (&[], None),
    };
    let mut builder =
        base_writer(row_group_max_bytes).set_statistics_enabled(EnabledStatistics::Page);
    for &column in delta_columns {
        builder = builder
            .set_column_dictionary_enabled(column.into(), false)
            .set_column_encoding(column.into(), Encoding::DELTA_BINARY_PACKED);
    }
    if let Some(column) = text_column {
        builder = builder.set_column_statistics_enabled(column.into(), EnabledStatistics::None);
    }
    builder.build()
}

fn access_writer_properties(row_group_max_bytes: usize) -> WriterProperties {
    let mut builder =
        base_writer(row_group_max_bytes).set_statistics_enabled(EnabledStatistics::Chunk);
    for column in [access::DOMAIN, access::SERVICE_ID] {
        builder = builder
            .set_column_bloom_filter_enabled(column.into(), true)
            .set_column_bloom_filter_max_ndv(column.into(), ACCESS_BLOOM_NDV);
    }
    for column in [
        access::UNMATCHED_HOST,
        access::CLIENT_IP,
        access::PATH,
        access::DURATION_US,
        access::UPSTREAM_TTFB_US,
        access::REQUEST_BODY_BYTES,
        access::RESPONSE_BODY_BYTES,
    ] {
        builder = builder.set_column_dictionary_enabled(column.into(), false);
    }
    for column in [
        NODE_ID,
        access::UNMATCHED_HOST,
        access::CLIENT_IP,
        access::PATH,
    ] {
        builder = builder.set_column_statistics_enabled(column.into(), EnabledStatistics::None);
    }
    for column in [TIMESTAMP, access::STATUS] {
        builder = builder.set_column_statistics_enabled(column.into(), EnabledStatistics::Page);
    }
    builder.build()
}

fn session_config(target_partitions: usize) -> SessionConfig {
    SessionConfig::new()
        .with_batch_size(PARQUET_BATCH_SIZE)
        .with_target_partitions(target_partitions)
        .set_bool("datafusion.optimizer.repartition_file_scans", false)
        .set_bool("datafusion.optimizer.prefer_existing_sort", true)
}

fn window_filter(time_range_nanos: (i64, i64), filter: Option<Expr>) -> Expr {
    let (start_nanos, end_nanos) = time_range_nanos;
    let window = col(TIMESTAMP)
        .gt_eq(lit_timestamp_nano(start_nanos))
        .and(col(TIMESTAMP).lt(lit_timestamp_nano(end_nanos)));
    match filter {
        Some(f) => window.and(f),
        None => window,
    }
}

fn read_options(schema: &datafusion::arrow::datatypes::Schema) -> ParquetReadOptions<'_> {
    ParquetReadOptions::default()
        .file_sort_order(vec![vec![SORT_FIELD.clone()]])
        .schema(schema)
}

fn needs_resort(batch: &RecordBatch) -> Option<&datafusion::arrow::array::ArrayRef> {
    use datafusion::arrow::array::Array;
    let ts = batch.column_by_name(TIMESTAMP)?;
    let sorted_desc = ts
        .as_any()
        .downcast_ref::<datafusion::arrow::array::TimestampNanosecondArray>()
        .is_some_and(|ts| ts.null_count() == 0 && ts.values().is_sorted_by(|a, b| a >= b));
    (!sorted_desc).then_some(ts)
}

fn flush_partitions(batches: &[RecordBatch]) -> Result<Vec<Vec<RecordBatch>>> {
    use datafusion::arrow::compute::{sort_to_indices, take};
    batches
        .iter()
        .map(|batch| {
            let Some(ts) = needs_resort(batch) else {
                return Ok(vec![batch.clone()]);
            };
            let indices = sort_to_indices(ts, Some(SORT_TO_INDICES_OPTIONS), None)?;
            let columns = batch
                .columns()
                .iter()
                .map(|c| take(c.as_ref(), &indices, None))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(vec![RecordBatch::try_new(batch.schema(), columns)?])
        })
        .collect()
}

fn batch_time_bounds(batch: &RecordBatch) -> Option<(i64, i64)> {
    let ts = batch
        .column_by_name(TIMESTAMP)?
        .as_any()
        .downcast_ref::<datafusion::arrow::array::TimestampNanosecondArray>()?;
    let min = datafusion::arrow::compute::min(ts)?;
    let max = datafusion::arrow::compute::max(ts)?;
    Some((min, max))
}

fn merge_candidate(
    best: &mut HashMap<(String, uuid::Uuid), (Chunk, bool)>,
    chunk: Chunk,
    in_local: bool,
) {
    match best.entry((chunk.node_id().to_owned(), chunk.id())) {
        std::collections::hash_map::Entry::Occupied(mut occupied) => {
            let (existing, existing_local) = occupied.get();
            let replace = chunk.generation_key() > existing.generation_key()
                || (chunk.generation_key() == existing.generation_key()
                    && in_local
                    && !existing_local);
            if replace {
                occupied.insert((chunk, in_local));
            }
        }
        std::collections::hash_map::Entry::Vacant(vacant) => {
            vacant.insert((chunk, in_local));
        }
    }
}

struct SyncingGuard<'a> {
    set: &'a scc::HashSet<Path>,
    path: &'a Path,
}

impl Drop for SyncingGuard<'_> {
    fn drop(&mut self) {
        self.set.remove_sync(self.path);
    }
}

#[derive(Debug)]
struct TargetStorage {
    os: Arc<dyn object_store::ObjectStore>,
    uri_prefix: ObjectStoreUrl,
    syncer: [tokio::sync::mpsc::Sender<Path>; MAX_SYNC_WORKER],
}

impl TargetStorage {
    fn uri(&self, path: &Path) -> String {
        format!(
            "{}/{}",
            self.uri_prefix.as_str().trim_end_matches('/'),
            path.as_ref().trim_start_matches('/')
        )
    }

    fn trigger_sync(
        &self,
        index_id: &str,
        path: &Path,
    ) -> std::result::Result<(), tokio::sync::mpsc::error::TrySendError<Path>> {
        let idx = index_id
            .bytes()
            .fold(0usize, |acc, b| acc.wrapping_add(b as usize))
            % MAX_SYNC_WORKER;
        #[expect(clippy::indexing_slicing)]
        self.syncer[idx].try_send(path.clone())
    }
}

type RemoteTarget = (Arc<dyn object_store::ObjectStore>, ObjectStoreUrl);

fn build_target(storage: &ObjectStoreConfig) -> Result<Option<RemoteTarget>> {
    let target: RemoteTarget = match storage {
        ObjectStoreConfig::Local => return Ok(None),
        ObjectStoreConfig::S3 {
            region,
            bucket_name,
            access_key_id,
            secret_access_key,
        } => (
            Arc::new(
                object_store::aws::AmazonS3Builder::new()
                    .with_region(region)
                    .with_bucket_name(bucket_name)
                    .with_access_key_id(access_key_id)
                    .with_secret_access_key(secret_access_key.reveal())
                    .build()?,
            ),
            ObjectStoreUrl::parse(format!("s3://{bucket_name}/"))?,
        ),
        ObjectStoreConfig::S3Compatible {
            endpoint,
            region,
            bucket_name,
            access_key_id,
            secret_access_key,
        } => (
            Arc::new(
                object_store::aws::AmazonS3Builder::new()
                    .with_endpoint(endpoint)
                    .with_region(region)
                    .with_bucket_name(bucket_name)
                    .with_access_key_id(access_key_id)
                    .with_secret_access_key(secret_access_key.reveal())
                    .with_allow_http(true)
                    .build()?,
            ),
            ObjectStoreUrl::parse(format!("s3://{bucket_name}/"))?,
        ),
        ObjectStoreConfig::Gcs {
            bucket_name,
            credentials,
        } => (
            Arc::new(
                object_store::gcp::GoogleCloudStorageBuilder::new()
                    .with_bucket_name(bucket_name)
                    .with_service_account_key(credentials.reveal())
                    .build()?,
            ),
            ObjectStoreUrl::parse(format!("gs://{bucket_name}/"))?,
        ),
        ObjectStoreConfig::Azure {
            container_name,
            account_name,
            account_key,
        } => (
            Arc::new(
                object_store::azure::MicrosoftAzureBuilder::new()
                    .with_container_name(container_name)
                    .with_account(account_name)
                    .with_access_key(account_key.reveal())
                    .build()?,
            ),
            ObjectStoreUrl::parse(format!("azure://{container_name}/"))?,
        ),
    };
    Ok(Some(target))
}

#[derive(Debug)]
pub struct Store {
    local: Arc<dyn object_store::ObjectStore>,
    local_root: String,
    node_id: String,
    retention_nanos: i64,
    row_group_max_bytes: usize,
    query_runtime: Arc<RuntimeEnv>,
    search_partitions: usize,
    aggregation_partitions: usize,
    indices: scc::HashMap<String, Index>,
    target: Option<TargetStorage>,
    syncing: scc::HashSet<Path>,
    flush_locks: scc::HashMap<String, Arc<tokio::sync::Mutex<()>>>,
    pending: scc::HashMap<String, Vec<RecordBatch>>,
    single_instance: bool,
    pub(crate) cpu: crate::cpu::Cache,
}

#[derive(Debug)]
pub struct Runtime {
    store: Arc<Store>,
    receivers: Vec<tokio::sync::mpsc::Receiver<Path>>,
    recover_remote: bool,
}

fn validate_sorting_columns() -> Result<()> {
    for kind in <IndexKind as strum::VariantArray>::VARIANTS.iter().copied() {
        if kind
            .schema()
            .fields()
            .first()
            .is_none_or(|f| f.name() != TIMESTAMP)
        {
            return Err(ErrorKind::Invariant(format!(
                "{kind:?} schema must lead with {TIMESTAMP} to match the parquet sorting column"
            ))
            .into());
        }
    }
    Ok(())
}

impl Store {
    pub(crate) fn row_group_max_bytes(&self) -> usize {
        self.row_group_max_bytes
    }

    pub async fn start(
        config: &config::Config,
        root: &std::path::Path,
    ) -> Result<(Arc<Self>, Runtime)> {
        validate_sorting_columns()?;
        #[expect(clippy::cast_possible_truncation)]
        let retention_nanos = config.retention.as_nanos() as i64;
        std::fs::create_dir_all(root)?;
        let canonical = root.canonicalize()?;
        let local: Arc<dyn object_store::ObjectStore> = Arc::new(
            object_store::local::LocalFileSystem::new_with_prefix(&canonical)?
                .with_automatic_cleanup(true),
        );
        let spill_dir = canonical.join(SPILL_DIR);
        if let Err(error) = std::fs::remove_dir_all(&spill_dir)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(error = ?error, "spill directory cleanup failed");
        }
        let query_runtime = RuntimeEnvBuilder::new()
            .with_memory_pool(Arc::new(TrackConsumersPool::new(
                FairSpillPool::new(config.query_memory_budget),
                NonZeroUsize::new(5).unwrap_or(NonZeroUsize::MIN),
            )))
            .with_temp_file_path(spill_dir)
            .with_max_temp_directory_size(config::QUERY_SPILL_MAX_BYTES)
            .build_arc()?;
        let mut receivers = Vec::new();
        let target = build_target(&config.storage)?.map(|(os, uri_prefix)| {
            let syncer = std::array::from_fn(|_| {
                let (tx, rx) = tokio::sync::mpsc::channel(SYNCER_CHANNEL_CAPACITY);
                receivers.push(rx);
                tx
            });
            TargetStorage {
                os,
                uri_prefix,
                syncer,
            }
        });
        let store = Arc::new(Self {
            local,
            local_root: canonical.to_string_lossy().into_owned(),
            node_id: config.node_id.clone(),
            retention_nanos,
            row_group_max_bytes: config.row_group_max_bytes,
            query_runtime,
            search_partitions: config.search_partitions,
            aggregation_partitions: config.aggregation_partitions,
            indices: scc::HashMap::new(),
            target,
            syncing: scc::HashSet::new(),
            flush_locks: scc::HashMap::new(),
            pending: scc::HashMap::new(),
            single_instance: config.single_instance,
            cpu: crate::cpu::Cache::default(),
        });
        sweep_temp_files(&store.local).await;
        store.rebuild_local().await?;
        let recover_remote = if store.target.is_some() {
            let startup =
                tokio::time::timeout(config::REMOTE_STARTUP_TIMEOUT.unsigned_abs(), async {
                    store.seed_remote_ledger().await?;
                    store.check_and_pull().await
                })
                .await
                .unwrap_or_else(|source| Err(ErrorKind::RemoteStartupTimeout(source).into()));
            match startup {
                Ok(()) => {
                    store.resync_unpushed();
                    false
                }
                Err(e) => {
                    tracing::error!(
                        error = ?e,
                        "remote storage unavailable at startup, retaining local writes and retrying in background"
                    );
                    true
                }
            }
        } else {
            false
        };
        Ok((
            Arc::clone(&store),
            Runtime {
                store,
                receivers,
                recover_remote,
            },
        ))
    }

    async fn recover_remote_ledger(self: Arc<Self>, cancel: CancellationToken) {
        let mut delay = config::REMOTE_RECOVERY_RETRY_DELAY.unsigned_abs();
        loop {
            tokio::select! {
                () = cancel.cancelled() => return,
                () = tokio::time::sleep(delay) => {}
            }
            let result = tokio::select! {
                () = cancel.cancelled() => return,
                result = self.seed_remote_ledger() => result,
            };
            match result {
                Ok(()) => {
                    let pull = tokio::select! {
                        () = cancel.cancelled() => return,
                        result = self.check_and_pull() => result,
                    };
                    if let Err(e) = pull {
                        tracing::error!(error = ?e, "failed to pull chunks from remote storage");
                    }
                    self.resync_unpushed();
                    tracing::info!("remote ledger seeded after degraded startup");
                    return;
                }
                Err(e) => {
                    tracing::error!(error = ?e, "remote ledger seeding failed, will retry");
                    delay = (delay * 2).min(config::REMOTE_RECOVERY_RETRY_MAX_DELAY.unsigned_abs());
                }
            }
        }
    }

    #[cfg(feature = "benchmark")]
    pub fn benchmark_stats(&self, index_id: &str) -> (u64, u64) {
        self.indices
            .read_sync(index_id, |_, index| {
                index.groups().flat_map(|group| group.visible()).fold(
                    (0, 0),
                    |(rows, bytes), chunk| {
                        let stats = chunk.stats();
                        (rows + stats.rows, bytes + stats.size)
                    },
                )
            })
            .unwrap_or_default()
    }

    #[cfg(feature = "benchmark")]
    pub fn benchmark_merge_pending(&self, index_id: &str) -> bool {
        self.merge_selection(index_id).is_some()
    }

    fn expiry_cutoff_nanos(&self) -> i64 {
        insight_schema::now_nanos().saturating_sub(self.retention_nanos)
    }

    fn local_uri(&self, path: &Path) -> String {
        format!(
            "file://{}/{}",
            self.local_root,
            path.as_ref().trim_start_matches('/')
        )
    }

    fn flush_lock(&self, index_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.flush_locks
            .entry_sync(index_id.to_owned())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .get()
            .clone()
    }

    async fn write_sorted(
        &self,
        index_id: &str,
        df_sorted: DataFrame,
        schema: Arc<datafusion::arrow::datatypes::Schema>,
        temp_path: &Path,
    ) -> Result<WrittenFile> {
        let properties = writer_properties(IndexKind::of(index_id), self.row_group_max_bytes);
        let result: Result<WrittenFile> = async {
            let mut writer = AsyncArrowWriter::try_new(
                BufWriter::new(self.local.clone(), temp_path.clone()),
                schema,
                Some(properties),
            )?;
            let mut bounds: Option<(i64, i64)> = None;
            let mut stream = df_sorted.execute_stream().await?;
            while let Some(batch_result) = stream.next().await {
                let batch = batch_result?;
                if let Some((batch_min, batch_max)) = batch_time_bounds(&batch) {
                    bounds = Some(bounds.map_or((batch_min, batch_max), |(min, max)| {
                        (min.min(batch_min), max.max(batch_max))
                    }));
                }
                writer.write(&batch).await?;
            }
            #[expect(clippy::cast_sign_loss)]
            let num_rows = writer.finish().await?.file_metadata().num_rows() as u64;
            Ok((num_rows, writer.bytes_written(), bounds))
        }
        .await;
        if result.is_err() {
            let _ = self.local.delete(temp_path).await;
        }
        result
    }

    fn record_chunk(&self, index_id: &str, site: Site, chunk: Chunk) {
        self.indices
            .entry_sync(index_id.to_owned())
            .or_default()
            .record_at(site, chunk);
    }

    async fn rebuild_local(&self) -> Result<()> {
        for (index_id, chunk) in list_chunks(&self.local, None, warn_stray("local")).await? {
            self.record_chunk(&index_id, Site::Local, chunk);
        }
        Ok(())
    }

    async fn seed_remote_ledger(&self) -> Result<()> {
        let Some(target) = &self.target else {
            return Ok(());
        };
        let mut foreign: HashMap<String, _> = HashMap::new();
        for (index_id, chunk) in list_chunks(&target.os, None, warn_stray("remote")).await? {
            if chunk.node_id() == self.node_id {
                self.record_chunk(&index_id, Site::Remote, chunk);
            } else if self
                .indices
                .read_sync(&index_id, |_, idx| idx.group(chunk.node_id()).is_some())
                .unwrap_or(false)
            {
                merge_candidate(foreign.entry(index_id).or_default(), chunk, false);
            }
        }
        for (index_id, best) in foreign {
            for (chunk, _) in best.into_values() {
                self.indices
                    .update_sync(&index_id, move |_, idx| idx.record_at(Site::Remote, chunk));
            }
        }
        Ok(())
    }

    async fn check_and_pull(&self) -> Result<()> {
        if self.target.is_none() {
            return Ok(());
        }
        let mut pulls: Vec<(String, Chunk)> = Vec::new();
        self.indices.iter_sync(|index_id, idx| {
            if let Some(group) = idx.group(&self.node_id) {
                for chunk in group.pull_candidates() {
                    pulls.push((index_id.clone(), chunk.clone()));
                }
            }
            true
        });
        for (index_id, chunk) in pulls {
            tracing::info!(
                "chunk {} for index {index_id} not found in local storage, pulling from remote",
                chunk.path().as_ref(),
            );
            match self.pull(chunk.path()).await {
                Ok(()) => {
                    self.indices
                        .update_sync(&index_id, move |_, idx| idx.record_at(Site::Local, chunk));
                }
                Err(e) => {
                    tracing::error!(error = ?e, "failed to pull chunk for index {index_id}");
                }
            }
        }
        Ok(())
    }

    fn resync_unpushed(&self) {
        if self.target.is_none() {
            return;
        }
        let cutoff = self.expiry_cutoff_nanos();
        let mut pending: Vec<(String, Path)> = Vec::new();
        self.indices.iter_sync(|index_id, idx| {
            for group in idx.groups() {
                for chunk in group.resync_candidates(cutoff) {
                    pending.push((index_id.clone(), chunk.path().clone()));
                }
            }
            true
        });
        for (index_id, path) in pending {
            tracing::info!("re-enqueueing unpushed chunk {} for sync", path.as_ref());
            self.trigger_sync(&index_id, &path);
        }
    }

    pub async fn flush(self: &Arc<Self>, index_id: &str, batches: &[RecordBatch]) -> Result<()> {
        if batches.is_empty() {
            return Ok(());
        }
        let ctx = SessionContext::new_with_config(session_config(2));
        let lock = self.flush_lock(index_id);
        let _flush_guard = lock.lock().await;
        let now = insight_schema::now_nanos();
        let (roll, sealed_at) = self
            .indices
            .read_sync(index_id, |_, idx| {
                idx.group(&self.node_id).map_or((None, now), |g| {
                    (g.roll_candidate(now).cloned(), g.next_sealed(now))
                })
            })
            .unwrap_or((None, now));
        let mut reusable_roll = select_reusable_roll(roll, self.row_group_max_bytes);
        if let Some(chunk) = &reusable_roll
            && let Err(e) = self.ensure_local(chunk.path()).await
        {
            tracing::warn!(
                error = ?e,
                "roll chunk {} unavailable locally, starting a fresh chunk",
                chunk.path().as_ref()
            );
            reusable_roll = None;
        }
        let batch_schema = current_schema(index_id)?;
        let payload_rows: u64 = batches.iter().map(|b| b.num_rows() as u64).sum();
        let temp_path = meta::temp_path(index_id, &self.node_id, ROLL_TEMP_SUFFIX);
        let mem_table = MemTable::try_new(batch_schema.clone(), flush_partitions(batches)?)?
            .with_sort_order(vec![vec![SORT_FIELD.clone()]]);
        let mut df_new = ctx.read_table(Arc::new(mem_table))?;
        if let Some(roll) = &reusable_roll {
            let df_existing = ctx
                .read_parquet(self.local_uri(roll.path()), read_options(&batch_schema))
                .await?;
            df_new = df_new.union(df_existing)?;
        }

        let df_sorted = df_new.sort(vec![SORT_FIELD.clone()])?;
        let (num_rows, p_size, bounds) = self
            .write_sorted(index_id, df_sorted, batch_schema, &temp_path)
            .await?;
        let Some((min_timestamp_nanos, max_timestamp_nanos)) = bounds else {
            let _ = self.local.delete(&temp_path).await;
            return Ok(());
        };
        let expected_rows = reusable_roll.as_ref().map_or(0, |c| c.stats().rows) + payload_rows;
        if num_rows != expected_rows {
            let _ = self.local.delete(&temp_path).await;
            return Err(ErrorKind::Invariant(format!(
                "flush of index {index_id} wrote {num_rows} rows, expected {expected_rows}"
            ))
            .into());
        }

        let stats = meta::ChunkStats {
            sealed: sealed_at,
            min_timestamp_nanos,
            max_timestamp_nanos,
            rows: num_rows,
            size: p_size as u64,
        };
        let chunk = match &reusable_roll {
            Some(roll) => roll.next_generation(index_id, stats),
            None => Chunk::new(index_id, &self.node_id, 0, stats),
        };
        self.local.rename(&temp_path, chunk.path()).await?;
        self.set_pending(index_id, &[]);
        let sync_path = chunk.path().clone();
        self.record_chunk(index_id, Site::Local, chunk);
        self.trigger_sync(index_id, &sync_path);
        Ok(())
    }

    fn trigger_sync(&self, index_id: &str, path: &Path) {
        let Some(target) = &self.target else {
            return;
        };
        if self.syncing.insert_sync(path.clone()).is_err() {
            return;
        }
        if let Err(e) = target.trigger_sync(index_id, path) {
            self.syncing.remove_sync(path);
            tracing::error!(
                error = ?e,
                "sync queue rejected {}, deferring to periodic resync",
                path.as_ref()
            );
        }
    }

    fn merge_selection(&self, index_id: &str) -> Option<meta::MergeSelection> {
        let now = insight_schema::now_nanos();
        self.indices
            .read_sync(index_id, |_, idx| {
                idx.group(&self.node_id).and_then(|group| {
                    let visible = group.visible();
                    let roll = group.roll_candidate(now).map(Chunk::id);
                    meta::select_merge(&visible, roll, self.retention_nanos)
                })
            })
            .flatten()
    }

    async fn merge_once(&self, index_id: &str) -> Result<()> {
        let Some(selection) = self.merge_selection(index_id) else {
            return Ok(());
        };
        let temp_path = meta::temp_path(index_id, &self.node_id, MERGED_TEMP_SUFFIX);

        let ctx = SessionContext::new_with_config(session_config(MERGE_MAX_FANIN));
        let batch_schema = current_schema(index_id)?;
        let min_timestamp_nanos = selection
            .inputs
            .iter()
            .map(|c| c.stats().min_timestamp_nanos)
            .min()
            .unwrap_or_default();
        let max_timestamp_nanos = selection
            .inputs
            .iter()
            .map(|c| c.stats().max_timestamp_nanos)
            .max()
            .unwrap_or_default();
        let sealed = selection
            .inputs
            .iter()
            .map(|c| c.stats().sealed)
            .max()
            .unwrap_or_default();
        let mut uris = Vec::with_capacity(selection.inputs.len());
        for chunk in &selection.inputs {
            self.ensure_local(chunk.path()).await?;
            uris.push(self.local_uri(chunk.path()));
        }
        if uris.is_empty() {
            return Ok(());
        }
        let df = ctx.read_parquet(uris, read_options(&batch_schema)).await?;
        let df_sorted = df.sort(vec![SORT_FIELD.clone()])?;
        let (num_rows, merged_size, _) = self
            .write_sorted(index_id, df_sorted, batch_schema, &temp_path)
            .await?;
        let expected_rows: u64 = selection.inputs.iter().map(|c| c.stats().rows).sum();
        if num_rows != expected_rows {
            let _ = self.local.delete(&temp_path).await;
            tracing::error!(
                "merge row conservation violated for index {index_id}: wrote {num_rows} rows, expected {expected_rows}, aborting merge",
            );
            return Ok(());
        }
        let merged_chunk = Chunk::new(
            index_id,
            &self.node_id,
            selection.output_level,
            meta::ChunkStats {
                sealed,
                min_timestamp_nanos,
                max_timestamp_nanos,
                rows: num_rows,
                size: merged_size as u64,
            },
        );
        self.local.rename(&temp_path, merged_chunk.path()).await?;
        let sync_path = merged_chunk.path().clone();
        self.record_chunk(index_id, Site::Local, merged_chunk);
        self.trigger_sync(index_id, &sync_path);

        Ok(())
    }

    async fn ensure_local(&self, path: &Path) -> Result<()> {
        match self.local.head(path).await {
            Ok(_) => Ok(()),
            Err(object_store::Error::NotFound { .. }) => self.pull(path).await,
            Err(e) => Err(e.into()),
        }
    }

    async fn pull(&self, path: &Path) -> Result<()> {
        let Some(target) = &self.target else {
            return Err(ErrorKind::Invariant(format!(
                "chunk {} is missing locally and no remote storage is configured",
                path.as_ref()
            ))
            .into());
        };
        let f = target.os.get(path).await?;
        let tf = self.local.put_multipart(path).await?;
        let mut write_multipart = WriteMultipart::new_with_chunk_size(tf, UPLOAD_CHUNK_SIZE);
        let mut stream = f.into_stream();
        while let Some(chunk_res) = stream.next().await {
            let chunk = chunk_res?;
            write_multipart.write(&chunk);
        }
        write_multipart.finish().await?;
        Ok(())
    }

    async fn push(&self, path: &Path) -> Result<()> {
        let Some(target) = &self.target else {
            return Ok(());
        };
        let f = match self.local.get(path).await {
            Ok(f) => f,
            Err(object_store::Error::NotFound { .. }) => {
                return Ok(());
            }
            Err(e) => {
                return Err(e.into());
            }
        };

        let file_size = f.meta.size;
        if file_size <= (UPLOAD_CHUNK_SIZE * 3) as u64 {
            let bytes = f.bytes().await?;
            let payload = PutPayload::from_bytes(bytes);
            target.os.put(path, payload).await?;
        } else {
            let mut stream = f.into_stream();
            let upload = target.os.put_multipart(path).await?;
            let mut write_multipart =
                WriteMultipart::new_with_chunk_size(upload, UPLOAD_CHUNK_SIZE);
            while let Some(chunk_res) = stream.next().await {
                let chunk = chunk_res?;
                write_multipart.write(&chunk);
            }
            write_multipart.finish().await?;
        }

        if let Some((index_id, chunk)) = Chunk::parse(path, file_size) {
            self.record_chunk(&index_id, Site::Remote, chunk);
        }

        Ok(())
    }

    pub async fn search<T: Default>(
        &self,
        index_id: &str,
        time_range_nanos: (i64, i64),
        filter: Option<Expr>,
        skip: usize,
        limit: Option<usize>,
        consume: impl Fn(&mut T, RecordBatch) -> Result<()>,
    ) -> Result<T> {
        self.query(
            index_id,
            time_range_nanos,
            filter,
            self.search_partitions,
            |df| df.sort(query_sort(index_id))?.limit(skip, limit),
            consume,
        )
        .await
    }

    pub async fn aggregate(
        &self,
        index_id: &str,
        time_range_nanos: (i64, i64),
        filter: Option<Expr>,
        plan: impl Fn(DataFrame) -> datafusion::error::Result<DataFrame>,
    ) -> Result<Vec<RecordBatch>> {
        self.query(
            index_id,
            time_range_nanos,
            filter,
            self.aggregation_partitions,
            plan,
            |batches: &mut Vec<_>, batch| {
                batches.push(batch);
                Ok(())
            },
        )
        .await
    }

    pub(crate) async fn fold<T: Default>(
        &self,
        index_id: &str,
        range: (i64, i64),
        plan: impl Fn(DataFrame) -> datafusion::error::Result<DataFrame>,
        consume: impl Fn(&mut T, RecordBatch) -> Result<()>,
    ) -> Result<T> {
        self.query(
            index_id,
            range,
            None,
            self.aggregation_partitions,
            plan,
            consume,
        )
        .await
    }

    async fn query<T: Default>(
        &self,
        index_id: &str,
        time_range_nanos: (i64, i64),
        filter: Option<Expr>,
        partitions: usize,
        plan: impl Fn(DataFrame) -> datafusion::error::Result<DataFrame>,
        consume: impl Fn(&mut T, RecordBatch) -> Result<()>,
    ) -> Result<T> {
        let (start_nanos, end_nanos) = time_range_nanos;
        if start_nanos >= end_nanos {
            return Ok(T::default());
        }
        let filter_expr = window_filter(time_range_nanos, filter);
        let candidates = self
            .query_candidates(index_id, start_nanos, end_nanos)
            .await?;
        let pending = self
            .pending
            .read_sync(index_id, |_, batches| batches.clone())
            .unwrap_or_default();
        if pending.is_empty() && candidates.is_empty() {
            return Ok(T::default());
        }
        let ctx = SessionContext::new_with_config_rt(
            SessionConfig::new()
                .with_batch_size(config::QUERY_BATCH_SIZE)
                .with_target_partitions(partitions)
                .set_bool("datafusion.optimizer.prefer_existing_sort", true)
                .set_bool(
                    "datafusion.optimizer.enable_round_robin_repartition",
                    !matches!(IndexKind::of(index_id), Some(IndexKind::Log)),
                )
                .set_usize(
                    "datafusion.execution.sort_pushdown_buffer_capacity",
                    config::QUERY_SORT_BUFFER_BYTES,
                )
                .set_bool("datafusion.execution.parquet.pushdown_filters", true)
                .set_bool("datafusion.execution.parquet.reorder_filters", true),
            Arc::clone(&self.query_runtime),
        );
        let mut df = if pending.is_empty() {
            None
        } else {
            let table = MemTable::try_new(current_schema(index_id)?, vec![pending])?;
            Some(query_input(
                ctx.read_table(Arc::new(table))?,
                index_id,
                filter_expr.clone(),
            )?)
        };
        if !candidates.is_empty() {
            let persisted = self
                .read_candidates(&ctx, index_id, &candidates, &filter_expr)
                .await?;
            df = Some(match df {
                Some(pending) => pending.union(persisted)?,
                None => persisted,
            });
        }
        let df = df.ok_or_else(|| {
            ErrorKind::Invariant(format!("query {index_id} has no readable input"))
        })?;
        let mut stream = plan(df)?.execute_stream().await?;
        let mut output = T::default();
        while let Some(batch) = stream.next().await {
            consume(&mut output, batch?)?;
        }
        Ok(output)
    }

    pub(crate) fn set_pending(&self, index_id: &str, batches: &[RecordBatch]) {
        if !self.single_instance {
            return;
        }
        if batches.is_empty() {
            let _ = self.pending.remove_sync(index_id);
        } else {
            self.pending
                .upsert_sync(index_id.to_owned(), batches.to_vec());
        }
    }

    async fn query_candidates(
        &self,
        index_id: &str,
        start_nanos: i64,
        end_nanos: i64,
    ) -> Result<Vec<(Chunk, bool)>> {
        let mut best: HashMap<(String, uuid::Uuid), (Chunk, bool)> = HashMap::new();
        if let Some(target) = &self.target
            && !self.single_instance
        {
            let prefix = Path::from(index_id);
            for (listed, chunk) in list_chunks(&target.os, Some(&prefix), |_| {}).await? {
                if listed == index_id {
                    merge_candidate(&mut best, chunk, false);
                }
            }
        }
        self.indices.read_sync(index_id, |_, idx| {
            for group in idx.groups() {
                for record in group.records() {
                    if let Some(current) = record.current() {
                        merge_candidate(&mut best, current.clone(), record.current_is_local());
                    }
                }
            }
        });

        let mut by_group: HashMap<String, Vec<(Chunk, bool)>> = HashMap::new();
        for ((node_id, _), entry) in best {
            by_group.entry(node_id).or_default().push(entry);
        }
        let mut candidates = Vec::new();
        for (_, entries) in by_group {
            let visible: HashSet<uuid::Uuid> = meta::shadow_visible(entries.iter().map(|(c, _)| c))
                .into_iter()
                .map(Chunk::id)
                .collect();
            for (chunk, in_local) in entries {
                if visible.contains(&chunk.id()) && chunk.overlaps(start_nanos, end_nanos) {
                    candidates.push((chunk, in_local));
                }
            }
        }
        Ok(candidates)
    }

    async fn read_candidates(
        &self,
        ctx: &SessionContext,
        index_id: &str,
        candidates: &[(Chunk, bool)],
        filter: &Expr,
    ) -> Result<DataFrame> {
        if let Some(target) = &self.target {
            ctx.register_object_store(target.uri_prefix.as_ref(), target.os.clone());
        }
        let read_schema = current_schema(index_id)?;
        let mut local_uris = Vec::new();
        let mut remote_uris = Vec::new();
        for (chunk, in_local) in candidates {
            match &self.target {
                Some(target) if !*in_local => remote_uris.push(target.uri(chunk.path())),
                _ => local_uris.push(self.local_uri(chunk.path())),
            }
        }
        let mut df: Option<DataFrame> = None;
        for uris in [local_uris, remote_uris] {
            if uris.is_empty() {
                continue;
            }
            let part = ctx.read_parquet(uris, read_options(&read_schema)).await?;
            let part = query_input(part, index_id, filter.clone())?;
            df = Some(match df {
                Some(acc) => acc.union(part)?,
                None => part,
            });
        }
        df.ok_or_else(|| {
            ErrorKind::Invariant(format!("no readable chunks for index {index_id}")).into()
        })
    }

    async fn cleanup_index(&self, index_id: &str) {
        let cutoff = self.expiry_cutoff_nanos();
        let mut local_stales: Vec<StaleEntry> = Vec::new();
        let mut remote_stales: Vec<StaleEntry> = Vec::new();
        self.indices.update_sync(index_id, |_, idx| {
            for group in idx.groups_mut() {
                let own = group.node_id() == self.node_id;
                if own {
                    group.retire_shadowed();
                }
                group.retire_covered_local(own);
                group.retire_expired(cutoff);
                let node_id = group.node_id().to_owned();
                let entries = |site| {
                    group
                        .stale_paths(site)
                        .map(|(id, path)| (node_id.clone(), id, path.clone()))
                };
                local_stales.extend(entries(Site::Local));
                remote_stales.extend(entries(Site::Remote));
            }
        });

        let local_acks = delete_stale(&self.local, local_stales, "local").await;
        let remote_acks = match &self.target {
            Some(target) => delete_stale(&target.os, remote_stales, "remote").await,
            None => Vec::new(),
        };
        self.indices.update_sync(index_id, |_, idx| {
            for (site, acks) in [(Site::Local, &local_acks), (Site::Remote, &remote_acks)] {
                for (node_id, id, path) in acks {
                    idx.group_mut(node_id).ack_stale(site, id, path);
                }
            }
            idx.compact();
        });
    }

    async fn sweep_expired_remote(&self) {
        let Some(target) = &self.target else {
            return;
        };
        let cutoff = self.expiry_cutoff_nanos();
        let chunks = match list_chunks(&target.os, None, warn_stray("remote")).await {
            Ok(chunks) => chunks,
            Err(e) => {
                tracing::error!(error = ?e, "failed to list remote storage during expiry sweep");
                return;
            }
        };
        let expired = chunks
            .into_iter()
            .filter(|(_, chunk)| chunk.stats().max_timestamp_nanos < cutoff)
            .map(|(_, chunk)| chunk.path().clone())
            .collect();
        delete_all(&target.os, expired, "expired remote object", |_| {}).await;
    }

    async fn sync_worker(
        self: Arc<Self>,
        mut rx: tokio::sync::mpsc::Receiver<Path>,
        cancel: CancellationToken,
    ) {
        loop {
            let path = tokio::select! {
                biased;
                () = cancel.cancelled() => return,
                path = rx.recv() => path,
            };
            let Some(path) = path else {
                return;
            };
            let _syncing_guard = SyncingGuard {
                set: &self.syncing,
                path: &path,
            };
            let result = tokio::select! {
                biased;
                () = cancel.cancelled() => return,
                result = self.push(&path) => result,
            };
            if let Err(e) = result {
                tracing::error!(
                    error = ?e,
                    "failed to sync {}, deferring to periodic resync",
                    path.as_ref()
                );
            }
        }
    }

    async fn run_local_maintenance(self: Arc<Self>, cancel: CancellationToken) {
        let mut swept_at = None;
        repeat_with_delay(
            config::LOCAL_MAINTENANCE_DELAY.unsigned_abs(),
            &cancel,
            || {
                let now = jiff::Timestamp::now();
                if swept_at.is_none_or(|at| now.duration_since(at).abs() >= config::CPU_GC_INTERVAL)
                {
                    self.cpu.sweep(now);
                    swept_at = Some(now);
                }
                let store = Arc::clone(&self);
                async move {
                    let mut indices = Vec::new();
                    store.indices.iter_sync(|k, _| {
                        indices.push(k.clone());
                        true
                    });
                    for index_id in indices {
                        if let Err(e) = store.merge_once(&index_id).await {
                            tracing::error!(error = ?e, "failed to merge index {index_id}");
                        }
                        store.cleanup_index(&index_id).await;
                    }
                }
            },
        )
        .await;
    }

    async fn run_remote_resyncer(self: Arc<Self>, cancel: CancellationToken) {
        repeat_with_delay(config::REMOTE_RESYNC_DELAY.unsigned_abs(), &cancel, || {
            let store = Arc::clone(&self);
            async move { store.resync_unpushed() }
        })
        .await;
    }

    async fn run_remote_cleaner(self: Arc<Self>, cancel: CancellationToken) {
        repeat_with_delay(config::REMOTE_SWEEP_DELAY.unsigned_abs(), &cancel, || {
            let store = Arc::clone(&self);
            async move { store.sweep_expired_remote().await }
        })
        .await;
    }
}

impl Runtime {
    pub async fn run(self, shutdown: CancellationToken) {
        let Self {
            store,
            receivers,
            recover_remote,
        } = self;
        let cancel = shutdown.child_token();
        let mut tasks = tokio::task::JoinSet::new();
        spawn_labeled(
            &mut tasks,
            Some("local storage maintenance"),
            Arc::clone(&store).run_local_maintenance(cancel.clone()),
        );
        spawn_labeled(
            &mut tasks,
            Some("remote resyncer"),
            Arc::clone(&store).run_remote_resyncer(cancel.clone()),
        );
        spawn_labeled(
            &mut tasks,
            Some("remote cleaner"),
            Arc::clone(&store).run_remote_cleaner(cancel.clone()),
        );
        for rx in receivers {
            let store = Arc::clone(&store);
            let cancel = cancel.clone();
            spawn_labeled(&mut tasks, Some("remote sync worker"), async move {
                store.sync_worker(rx, cancel).await;
            });
        }
        if recover_remote {
            spawn_labeled(
                &mut tasks,
                None,
                Arc::clone(&store).recover_remote_ledger(cancel.clone()),
            );
        }

        loop {
            tokio::select! {
                biased;
                () = shutdown.cancelled() => break,
                result = tasks.join_next(), if !tasks.is_empty() => {
                    match result {
                        Some(Ok(None)) | None => {}
                        Some(Ok(Some(task))) => {
                            tracing::error!("{task} exited unexpectedly");
                            break;
                        }
                        Some(Err(e)) => {
                            tracing::error!(error = ?e, "storage background task failed");
                            break;
                        }
                    }
                }
            }
        }
        cancel.cancel();
        while let Some(result) = tasks.join_next().await {
            if let Err(e) = result {
                tracing::error!(error = ?e, "storage background task failed");
            }
        }
        store.syncing.clear_sync();
    }
}

fn query_input(df: DataFrame, index_id: &str, filter: Expr) -> Result<DataFrame> {
    let mut df = df.filter(filter)?;
    if !matches!(IndexKind::of(index_id), Some(IndexKind::Log)) {
        return Ok(df);
    }
    let compact = create_udf(
        "compact_view",
        vec![DataType::Utf8View],
        DataType::Utf8View,
        Volatility::Immutable,
        Arc::new(|args| {
            let [value] = args else {
                return internal_err!("compact_view requires one argument");
            };
            if let ColumnarValue::Array(array) = value {
                let view = as_string_view_array(array.as_ref())?;
                let backing: usize = view.data_buffers().iter().map(Buffer::capacity).sum();
                if backing > view.total_buffer_bytes_used().saturating_mul(2) {
                    return Ok(ColumnarValue::Array(Arc::new(view.gc())));
                }
            }
            Ok(value.clone())
        }),
    );
    for name in [log::SOURCE, insight_schema::CONTAINER_ID, NODE_ID, log::LOG] {
        df = df.with_column(name, compact.call(vec![col(name)]))?;
    }
    Ok(df)
}

fn spawn_labeled(
    tasks: &mut tokio::task::JoinSet<Option<&'static str>>,
    label: Option<&'static str>,
    task: impl Future<Output = ()> + Send + 'static,
) {
    tasks.spawn(async move {
        task.await;
        label
    });
}

async fn repeat_with_delay<F, Fut>(
    delay: std::time::Duration,
    cancel: &CancellationToken,
    mut task: F,
) where
    F: FnMut() -> Fut,
    Fut: Future<Output = ()>,
{
    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => return,
            () = async {
                task().await;
                tokio::time::sleep(delay).await;
            } => {}
        }
    }
}
