use std::sync::Arc;

use arrow::array::StringViewArray;
use base64::Engine as _;
use futures_util::TryStreamExt as _;
use insight::storage::store::Store;
use insight_schema::log::{LogBatchBuilder, LogRecord};
use object_store::{ObjectStore as _, ObjectStoreExt as _};
use sha2::{Digest as _, Sha256};
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
    let remote = object_store::aws::AmazonS3Builder::new()
        .with_endpoint("http://localhost:19390")
        .with_allow_http(true)
        .with_region("garage")
        .with_bucket_name("insight")
        .with_access_key_id("GK00000000000000000000000000000000")
        .with_secret_access_key("2222222222222222222222222222222222222222222222222222222222222222")
        .build()?;
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
