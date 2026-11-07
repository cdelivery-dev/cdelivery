use std::sync::Arc;

use arrow::array::{
    RecordBatch, StringArray, StringViewArray, TimestampNanosecondArray, UInt32Array, UInt64Array,
};
use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use arrow::ipc::writer::StreamWriter;
use arrow::util::display::array_value_to_string;
use base64::Engine as _;
use futures_util::TryStreamExt as _;
use insight::storage::meta::{Chunk, ChunkStats};
use insight::storage::store::Store;
use insight_schema::log::{LogBatchBuilder, LogRecord};
use insight_schema::sample::FilesystemSample;
use object_store::{ObjectStore as _, ObjectStoreExt as _};
use parquet::arrow::ArrowWriter;
use sha2::{Digest as _, Sha256};
use tokio::io::AsyncWriteExt as _;
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

#[path = "../../tests/process.rs"]
mod process;
mod support;

use support::{Context, Result, ensure, eventually};

#[tokio::test]
async fn local_durability() -> Result<()> {
    support::run(false).await
}

#[tokio::test]
async fn s3_durability_and_recovery() -> Result<()> {
    support::run(true).await
}

fn config(node: &str, remote: bool) -> insight::config::Config {
    insight::config::Config {
        storage: if remote {
            insight_wire::ObjectStoreConfig::S3Compatible {
                endpoint: "http://localhost:19390".into(),
                region: "garage".into(),
                bucket_name: "insight".into(),
                access_key_id: "GK00000000000000000000000000000000".into(),
                secret_access_key: cipher::Secret::new(
                    "2222222222222222222222222222222222222222222222222222222222222222".to_owned(),
                ),
            }
        } else {
            insight_wire::ObjectStoreConfig::Local
        },
        node_id: node.into(),
        retention: jiff::SignedDuration::from_hours(24),
        writer_flush_interval: jiff::SignedDuration::from_secs(5),
        row_group_max_bytes: 512 * 1024 * 1024,
        ingest_memory_budget: 512 * 1024 * 1024,
        query_memory_budget: 256 * 1024 * 1024,
        search_partitions: 2,
        aggregation_partitions: 2,
        single_instance: !remote,
    }
}

fn checksum(value: &str) -> u64 {
    Sha256::digest(value.as_bytes())
        .iter()
        .take(8)
        .fold(0_u64, |sum, byte| sum.wrapping_shl(8) | u64::from(*byte))
}

async fn read(store: &Store, index: &str, timestamp: i64) -> Result<(u64, u64)> {
    Ok(store
        .search(
            index,
            (timestamp - 1, timestamp + 1),
            None,
            0,
            None,
            |result: &mut (u64, u64), batch| {
                let values = batch
                    .column_by_name(insight_schema::log::LOG)
                    .and_then(|array| array.as_any().downcast_ref::<StringViewArray>())
                    .ok_or_else(|| {
                        insight::error::Error::from(insight::error::ErrorKind::Invariant(
                            "log query schema changed".into(),
                        ))
                    })?;
                for value in values.iter().flatten() {
                    result.0 += 1;
                    result.1 = result.1.wrapping_add(checksum(value));
                }
                Ok(())
            },
        )
        .await?)
}

async fn durability(context: &Context, remote: bool) -> Result<()> {
    let node = support::name("writer");
    let index = format!("{}/log", uuid::Uuid::new_v4().simple());
    let root = context.root.join(&node);
    let configuration = config(&node, remote);
    let (store, runtime) = Store::start(&configuration, &root).await?;
    let cancel = CancellationToken::new();
    let _cancel_on_drop = cancel.clone().drop_guard();
    let task = tokio::spawn(runtime.run(cancel.clone()));
    let timestamp = insight_schema::now_nanos();
    let mut builder = LogBatchBuilder::new(1);
    builder.append(&LogRecord {
        timestamp,
        source: "stdout".into(),
        container_id: "fixture".into(),
        node_id: node.clone(),
        log: "persisted-small".into(),
    });
    store.flush(&index, &[builder.flush()?]).await?;
    ensure(
        read(&store, &index, timestamp).await? == (1, checksum("persisted-small")),
        "flushed data must be queryable",
    )?;
    if remote {
        s3(context, &store, &index, timestamp, &node).await?;
    }
    cancel.cancel();
    task.await?;
    drop(store);
    let (reopened, _) = Store::start(&configuration, &root).await?;
    let expected = if remote { 24_002 } else { 1 };
    ensure(
        read(&reopened, &index, timestamp).await?.0 == expected,
        "local restart must preserve all flushed rows",
    )?;
    if remote {
        let fresh = context.root.join(support::name("fresh"));
        let (recovered, _) = Store::start(&configuration, &fresh).await?;
        ensure(
            read(&recovered, &index, timestamp).await?
                == read(&reopened, &index, timestamp).await?,
            "a fresh local directory must recover exact data from S3",
        )?;
    }
    windows(context, remote).await?;
    if remote {
        merge_recovery(context).await?;
    } else {
        graceful_shutdown(context).await?;
        schema_evolution(context).await?;
    }
    Ok(())
}

fn logs(timestamp: i64, ids: std::ops::Range<u32>) -> Result<RecordBatch> {
    let mut builder = LogBatchBuilder::new(ids.len());
    for id in ids {
        builder.append(&LogRecord {
            timestamp,
            source: "stdout".into(),
            container_id: "fixture".into(),
            node_id: "fixture".into(),
            log: format!("record-{id}"),
        });
    }
    Ok(builder.flush()?)
}

async fn graceful_shutdown(context: &Context) -> Result<()> {
    let node = support::name("shutdown");
    let root = context.root.join(&node);
    let mut configuration = config(&node, false);
    configuration.writer_flush_interval = jiff::SignedDuration::from_hours(1);
    let (store, _) = Store::start(&configuration, &root).await?;
    let ingest = Arc::new(insight::ingest::Ingest::new(
        Arc::clone(&store),
        configuration.writer_flush_interval,
        configuration.ingest_memory_budget,
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let cancel = CancellationToken::new();
    let _guard = cancel.clone().drop_guard();
    let server =
        tokio::spawn(Arc::clone(&ingest).listen(listener, cancel.clone().cancelled_owned()));
    let mut client = TcpStream::connect(address).await?;
    let timestamp = insight_schema::now_nanos();
    let mut expected = Vec::new();
    for (index, start) in [("first/log", 0), ("second/log", 8)] {
        for offset in [0, 4] {
            let batch = logs(timestamp, start + offset..start + offset + 4)?;
            let mut frame = Vec::new();
            insight_wire::frame::write_header(&mut frame, index);
            {
                let mut writer = StreamWriter::try_new(&mut frame, &batch.schema())?;
                writer.write(&batch)?;
                writer.finish()?;
            }
            client.write_u32(u32::try_from(frame.len())?).await?;
            client.write_all(&frame).await?;
        }
        let checksum = (start..start + 8)
            .map(|id| checksum(&format!("record-{id}")))
            .fold(0_u64, u64::wrapping_add);
        expected.push((index, (8, checksum)));
    }
    eventually("buffered TCP records", || async {
        for (index, records) in &expected {
            ensure(
                read(&store, index, timestamp).await? == *records,
                "both writers must expose every buffered record before shutdown",
            )?;
        }
        Ok(())
    })
    .await?;
    let disk = object_store::local::LocalFileSystem::new_with_prefix(&root)?;
    for (index, _) in &expected {
        ensure(
            chunks(&disk, index).await?.is_empty(),
            "records must still be buffered before shutdown",
        )?;
    }
    cancel.cancel();
    tokio::time::timeout(std::time::Duration::from_secs(20), server).await??;
    drop(client);
    drop(ingest);
    drop(store);
    let (reopened, _) = Store::start(&configuration, &root).await?;
    for (index, records) in expected {
        ensure(
            read(&reopened, index, timestamp).await? == records,
            "graceful shutdown with an open connection must persist every buffered record",
        )?;
    }
    Ok(())
}

async fn write_chunk(
    disk: &dyn object_store::ObjectStore,
    index: &str,
    node: &str,
    level: u8,
    sealed: i64,
    batch: &RecordBatch,
) -> Result<Chunk> {
    let mut writer = ArrowWriter::try_new(Vec::new(), batch.schema(), None)?;
    writer.write(batch)?;
    let bytes = writer.into_inner()?;
    let timestamps = batch
        .column_by_name("timestamp")
        .and_then(|array| array.as_any().downcast_ref::<TimestampNanosecondArray>())
        .ok_or_else(|| support::ErrorKind::Check("fixture has no timestamp column".into()))?;
    let chunk = Chunk::new(
        index,
        node,
        level,
        ChunkStats {
            sealed,
            min_timestamp_nanos: arrow::compute::min(timestamps)
                .ok_or_else(|| support::ErrorKind::Check("fixture has no timestamps".into()))?,
            max_timestamp_nanos: arrow::compute::max(timestamps)
                .ok_or_else(|| support::ErrorKind::Check("fixture has no timestamps".into()))?,
            rows: u64::try_from(batch.num_rows())?,
            size: u64::try_from(bytes.len())?,
        },
    );
    disk.put(chunk.path(), bytes.into()).await?;
    Ok(chunk)
}

async fn chunks(disk: &dyn object_store::ObjectStore, index: &str) -> Result<Vec<Chunk>> {
    let prefix = object_store::path::Path::from(index);
    let objects: Vec<_> = disk.list(Some(&prefix)).try_collect().await?;
    objects
        .into_iter()
        .map(|object| {
            Chunk::parse(&object.location, object.size)
                .map(|(_, chunk)| chunk)
                .ok_or_else(|| {
                    support::ErrorKind::Check(format!("invalid chunk: {}", object.location)).into()
                })
        })
        .collect()
}

fn rows(batches: &[RecordBatch]) -> Result<Vec<Vec<Option<String>>>> {
    let mut rows = Vec::new();
    for batch in batches {
        for row in 0..batch.num_rows() {
            rows.push(
                batch
                    .columns()
                    .iter()
                    .map(|array| {
                        if array.is_null(row) {
                            Ok(None)
                        } else {
                            array_value_to_string(array.as_ref(), row).map(Some)
                        }
                    })
                    .collect::<std::result::Result<Vec<_>, _>>()?,
            );
        }
    }
    rows.sort_unstable();
    Ok(rows)
}

async fn check_schema(store: &Store, expected: &[FilesystemSample]) -> Result<()> {
    let expected = insight_schema::metric::filesystem_batch(expected)?;
    let actual: Vec<RecordBatch> = store
        .search(
            insight_wire::FILESYSTEM_METRIC_INDEX_ID,
            (i64::MIN, i64::MAX),
            None,
            0,
            None,
            |batches: &mut Vec<_>, batch| {
                batches.push(batch);
                Ok(())
            },
        )
        .await?;
    for batch in &actual {
        ensure(
            batch.num_columns() == expected.num_columns()
                && batch
                    .schema()
                    .fields()
                    .iter()
                    .zip(expected.schema().fields())
                    .all(|(actual, expected)| {
                        actual.name() == expected.name()
                            && actual.data_type() == expected.data_type()
                    }),
            "old and current files must return the current column names, order and types",
        )?;
    }
    ensure(
        rows(&actual)? == rows(&[expected])?,
        "schema adaptation must preserve every value and fill missing fields with null",
    )
}

async fn schema_evolution(context: &Context) -> Result<()> {
    let node = support::name("schema");
    let root = context.root.join(&node);
    std::fs::create_dir_all(&root)?;
    let disk = object_store::local::LocalFileSystem::new_with_prefix(&root)?;
    let configuration = config(&node, false);
    let index = insight_wire::FILESYSTEM_METRIC_INDEX_ID;
    let base = insight_schema::now_nanos() - 1_000_000_000;
    let schema = Arc::new(Schema::new(vec![
        Field::new(
            "timestamp",
            DataType::Timestamp(TimeUnit::Nanosecond, None),
            false,
        ),
        Field::new("mount_point", DataType::Utf8, false),
        Field::new("total_bytes", DataType::UInt32, false),
        Field::new("node_id", DataType::Utf8, false),
        Field::new("free_bytes", DataType::UInt64, false),
        Field::new("available_bytes", DataType::UInt64, false),
        Field::new("retired", DataType::Utf8, true),
    ]));
    let mut expected = Vec::new();
    let mut originals = Vec::new();
    for id in 0..9_u32 {
        let timestamp = base + i64::from(id);
        let mount = format!("/data/{id}");
        let total = u32::MAX - id;
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(TimestampNanosecondArray::from(vec![timestamp])),
                Arc::new(StringArray::from(vec![mount.as_str()])),
                Arc::new(UInt32Array::from(vec![total])),
                Arc::new(StringArray::from(vec![node.as_str()])),
                Arc::new(UInt64Array::from(vec![100 + u64::from(id)])),
                Arc::new(UInt64Array::from(vec![50 + u64::from(id)])),
                Arc::new(StringArray::from(vec!["discarded"])),
            ],
        )?;
        originals.push(write_chunk(&disk, index, &node, 0, timestamp, &batch).await?);
        expected.push(FilesystemSample {
            timestamp: jiff::Timestamp::from_nanosecond(i128::from(timestamp))?,
            node_id: node.clone(),
            mount_point: mount,
            total_bytes: u64::from(total),
            free_bytes: 100 + u64::from(id),
            available_bytes: 50 + u64::from(id),
            inodes_total: None,
            inodes_free: None,
        });
    }
    let (store, runtime) = Store::start(&configuration, &root).await?;
    check_schema(&store, &expected).await?;
    let new = FilesystemSample {
        timestamp: jiff::Timestamp::from_nanosecond(i128::from(base + 9))?,
        node_id: node.clone(),
        mount_point: "/new".into(),
        total_bytes: u64::from(u32::MAX) + 100,
        free_bytes: 200,
        available_bytes: 150,
        inodes_total: Some(1_000),
        inodes_free: Some(900),
    };
    store
        .flush(
            index,
            &[insight_schema::metric::filesystem_batch(
                std::slice::from_ref(&new),
            )?],
        )
        .await?;
    expected.push(new);
    check_schema(&store, &expected).await?;
    let cancel = CancellationToken::new();
    let _guard = cancel.clone().drop_guard();
    let task = tokio::spawn(runtime.run(cancel.clone()));
    eventually("schema compaction and cleanup", || async {
        let files = chunks(&disk, index).await?;
        ensure(
            files.iter().any(|chunk| chunk.level() == 1)
                && originals
                    .iter()
                    .all(|old| files.iter().all(|file| file.path() != old.path())),
            "maintenance must merge old files and remove superseded generations",
        )
    })
    .await?;
    cancel.cancel();
    task.await?;
    check_schema(&store, &expected).await?;
    drop(store);
    let (reopened, _) = Store::start(&configuration, &root).await?;
    check_schema(&reopened, &expected).await
}

fn s3_client() -> Result<object_store::aws::AmazonS3> {
    Ok(object_store::aws::AmazonS3Builder::new()
        .with_endpoint("http://localhost:19390")
        .with_allow_http(true)
        .with_region("garage")
        .with_bucket_name("insight")
        .with_access_key_id("GK00000000000000000000000000000000")
        .with_secret_access_key("2222222222222222222222222222222222222222222222222222222222222222")
        .build()?)
}

#[expect(clippy::too_many_lines)]
async fn merge_recovery(context: &Context) -> Result<()> {
    let node = support::name("merge");
    let root = context.root.join(&node);
    std::fs::create_dir_all(&root)?;
    let disk = object_store::local::LocalFileSystem::new_with_prefix(&root)?;
    let remote = s3_client()?;
    let configuration = config(&node, true);
    let index = format!("{}/log", uuid::Uuid::new_v4().simple());
    let timestamp = insight_schema::now_nanos() - 1_000_000_000;
    let groups = (0..6)
        .map(|group| (1, group * 8..(group + 1) * 8))
        .chain((48..65).map(|id| (0, id..id + 1)));
    let mut originals = Vec::new();
    for (level, ids) in groups {
        let chunk = write_chunk(
            &disk,
            &index,
            &node,
            level,
            timestamp + i64::from(ids.end),
            &logs(timestamp, ids)?,
        )
        .await?;
        remote
            .put(
                chunk.path(),
                disk.get(chunk.path()).await?.bytes().await?.into(),
            )
            .await?;
        originals.push(chunk);
    }
    let expected_checksum = (0..66)
        .map(|id| checksum(&format!("record-{id}")))
        .fold(0_u64, u64::wrapping_add);
    let (store, runtime) = Store::start(&configuration, &root).await?;
    context.compose(&["stop", "garage"])?;
    store.flush(&index, &[logs(timestamp, 65..66)?]).await?;
    let cancel = CancellationToken::new();
    let _guard = cancel.clone().drop_guard();
    let task = tokio::spawn(runtime.run(cancel.clone()));
    eventually("offline compaction", || async {
        let files = chunks(&disk, &index).await?;
        ensure(
            files.iter().any(|file| file.level() == 2)
                && files
                    .iter()
                    .any(|file| file.level() == 0 && file.stats().rows == 2),
            "unuploaded compaction and appended records must remain on disk",
        )
    })
    .await?;
    cancel.cancel();
    task.await?;
    drop(store);
    context.compose(&["start", "--wait", "garage"])?;
    let before_upload = chunks(&remote, &index).await?;
    ensure(
        before_upload.len() == originals.len()
            && originals
                .iter()
                .all(|old| before_upload.iter().any(|file| file.path() == old.path())),
        "remote originals must survive the outage before replacements are uploaded",
    )?;
    let reader_config = config(&support::name("reader"), true);
    let (reader, _) = Store::start(&reader_config, &context.root.join("before-upload")).await?;
    ensure(
        read(&reader, &index, timestamp).await?
            == (65, expected_checksum.wrapping_sub(checksum("record-65"))),
        "remote originals must still contain every previously uploaded record",
    )?;
    drop(reader);
    let (store, runtime) = Store::start(&configuration, &root).await?;
    let cancel = CancellationToken::new();
    let _guard = cancel.clone().drop_guard();
    let task = tokio::spawn(runtime.run(cancel.clone()));
    eventually("compaction upload and remote cleanup", || async {
        let files = chunks(&remote, &index).await?;
        ensure(
            files.iter().any(|file| file.level() == 2)
                && files
                    .iter()
                    .any(|file| file.level() == 0 && file.stats().rows == 2)
                && originals
                    .iter()
                    .all(|old| files.iter().all(|file| file.path() != old.path())),
            "uploaded replacements must eventually retire every superseded remote object",
        )
    })
    .await?;
    cancel.cancel();
    task.await?;
    ensure(
        read(&store, &index, timestamp).await? == (66, expected_checksum),
        "compaction must preserve every record",
    )?;
    let (reader, _) = Store::start(&reader_config, &context.root.join("after-upload")).await?;
    ensure(
        read(&reader, &index, timestamp).await? == (66, expected_checksum),
        "a fresh reader must recover every record after compaction and backfill",
    )
}

async fn windows(context: &Context, remote: bool) -> Result<()> {
    let node = support::name("windows");
    let index = format!("{}/log", uuid::Uuid::new_v4().simple());
    let root = context.root.join(&node);
    let configuration = config(&node, remote);
    let (store, runtime) = Store::start(&configuration, &root).await?;
    let cancel = CancellationToken::new();
    let _guard = cancel.clone().drop_guard();
    let task = tokio::spawn(runtime.run(cancel.clone()));
    let now = insight_schema::now_nanos();
    for offsets in [[5, 1, 3], [2, 6, 4], [9, 7, 8], [0, 11, 10]] {
        let mut builder = LogBatchBuilder::new(3);
        for offset in offsets {
            builder.append(&LogRecord {
                timestamp: now + offset,
                source: "stdout".into(),
                container_id: "window".into(),
                node_id: node.clone(),
                log: format!("row-{offset}"),
            });
        }
        store.flush(&index, &[builder.flush()?]).await?;
    }
    cancel.cancel();
    task.await?;
    drop(store);
    let (store, _) = Store::start(&configuration, &root).await?;
    let read = async |start, end, skip, limit| {
        store
            .search(
                &index,
                (now + start, now + end),
                None,
                skip,
                limit,
                |rows: &mut Vec<String>, batch| {
                    let logs = batch
                        .column_by_name(insight_schema::log::LOG)
                        .and_then(|array| array.as_any().downcast_ref::<StringViewArray>())
                        .ok_or_else(|| {
                            insight::error::Error::from(insight::error::ErrorKind::Invariant(
                                "missing log column".into(),
                            ))
                        })?;
                    rows.extend(logs.iter().flatten().map(str::to_owned));
                    Ok(())
                },
            )
            .await
    };
    let expected: Vec<_> = (0..12)
        .rev()
        .map(|offset| format!("row-{offset}"))
        .collect();
    ensure(
        read(0, 12, 0, None).await? == expected,
        "out-of-order flushes must retain every row in descending time order across restart",
    )?;
    ensure(
        read(3, 6, 0, None).await? == ["row-5", "row-4", "row-3"],
        "time windows must include start and exclude end",
    )?;
    let mut pages = Vec::new();
    for skip in [0, 4, 8, 12] {
        pages.extend(read(0, 12, skip, Some(4)).await?);
    }
    ensure(
        pages == expected,
        "pagination across batches must not drop or duplicate rows",
    )?;
    ensure(
        read(3, 3, 0, None).await?.is_empty() && read(12, 20, 0, None).await?.is_empty(),
        "empty and nonoverlapping windows must return no rows",
    )?;
    let foreign: usize = store
        .search(
            "nonexistent/log",
            (now, now + 12),
            None,
            0,
            None,
            |rows: &mut usize, batch| {
                *rows += batch.num_rows();
                Ok(())
            },
        )
        .await?;
    ensure(
        foreign == 0,
        "index isolation must survive batch rollover and restart",
    )?;
    Ok(())
}

#[expect(clippy::too_many_lines)]
async fn s3(
    context: &Context,
    store: &Arc<Store>,
    index: &str,
    timestamp: i64,
    node: &str,
) -> Result<()> {
    let remote = s3_client()?;
    let prefix = object_store::path::Path::from(index);
    eventually("ordinary S3 upload", || async {
        let objects: Vec<_> = remote.list(Some(&prefix)).try_collect().await?;
        ensure(
            objects
                .iter()
                .any(|object| object.size > 0 && object.size <= 96 * 1024 * 1024),
            "small Parquet must be uploaded",
        )
    })
    .await?;
    let mut builder = LogBatchBuilder::new(24_000);
    let mut expected = checksum("persisted-small");
    let mut random = 0x1234_5678_9abc_def0_u64;
    for row in 0..24_000 {
        let bytes: Vec<_> = (0..4608)
            .map(|_| {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                random.to_le_bytes()[0]
            })
            .collect();
        let log = format!(
            "{row}:{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        );
        expected = expected.wrapping_add(checksum(&log));
        builder.append(&LogRecord {
            timestamp,
            source: "stdout".into(),
            container_id: "fixture".into(),
            node_id: node.into(),
            log,
        });
    }
    store.flush(index, &[builder.flush()?]).await?;
    eventually("multipart S3 upload", || async {
        let objects: Vec<_> = remote.list(Some(&prefix)).try_collect().await?;
        let object = objects
            .iter()
            .find(|object| object.size > 96 * 1024 * 1024)
            .ok_or_else(|| {
                support::ErrorKind::Check(
                    "no Parquet larger than 96 MiB; multipart path was not exercised".into(),
                )
            })?;
        let head = remote.head(&object.location).await?;
        ensure(
            head.e_tag.as_ref().is_some_and(|etag| etag.contains('-')),
            "Garage must report a multipart ETag",
        )
    })
    .await?;
    ensure(
        read(store, index, timestamp).await? == (24_001, expected),
        "large dataset row count and checksum must match",
    )?;
    context.compose(&["stop", "garage"])?;
    let offline = async {
        let mut builder = LogBatchBuilder::new(1);
        builder.append(&LogRecord {
            timestamp,
            source: "stdout".into(),
            container_id: "fixture".into(),
            node_id: node.into(),
            log: "during-outage".into(),
        });
        store.flush(index, &[builder.flush()?]).await?;
        ensure(
            read(store, index, timestamp).await.is_err(),
            "multi-instance queries must fail when the remote inventory is unavailable",
        )
    }
    .await;
    offline?;
    context.compose(&["start", "garage"])?;
    let reader_config = config(&support::name("reader"), true);
    let reader_root = context.root.join(&reader_config.node_id);
    eventually("S3 backfill and multi-instance visibility", || async {
        let (reader, _) = Store::start(&reader_config, &reader_root).await?;
        ensure(
            read(&reader, index, timestamp).await?
                == (24_002, expected.wrapping_add(checksum("during-outage"))),
            "remote reader must see exact data after recovery",
        )
    })
    .await?;
    for (label, storage) in [
        (
            "bad-key",
            insight_wire::ObjectStoreConfig::S3Compatible {
                endpoint: "http://localhost:19390".into(),
                region: "garage".into(),
                bucket_name: "insight".into(),
                access_key_id: "invalid".into(),
                secret_access_key: cipher::Secret::new("invalid".to_owned()),
            },
        ),
        (
            "missing-bucket",
            insight_wire::ObjectStoreConfig::S3Compatible {
                endpoint: "http://localhost:19390".into(),
                region: "garage".into(),
                bucket_name: "absent".into(),
                access_key_id: "GK00000000000000000000000000000000".into(),
                secret_access_key: cipher::Secret::new(
                    "2222222222222222222222222222222222222222222222222222222222222222".to_owned(),
                ),
            },
        ),
    ] {
        let mut invalid = config(label, true);
        invalid.storage = storage;
        let directory = context.root.join(support::name(label));
        let degraded_index = format!("{}/log", uuid::Uuid::new_v4().simple());
        let (degraded, _) = Store::start(&invalid, &directory).await?;
        let mut builder = LogBatchBuilder::new(1);
        builder.append(&LogRecord {
            timestamp,
            source: "stdout".into(),
            container_id: "fixture".into(),
            node_id: label.into(),
            log: label.into(),
        });
        degraded.flush(&degraded_index, &[builder.flush()?]).await?;
        ensure(
            read(&degraded, &degraded_index, timestamp).await.is_err(),
            "invalid remote config must not silently return an incomplete query",
        )?;
        drop(degraded);
        let (recovered, _) = Store::start(&config(label, true), &directory).await?;
        ensure(
            read(&recovered, &degraded_index, timestamp).await? == (1, checksum(label)),
            "flushed writes from degraded startup must survive restart with corrected credentials",
        )?;
    }
    Ok(())
}
