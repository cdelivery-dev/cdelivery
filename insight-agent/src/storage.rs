use arrow::record_batch::RecordBatch;
use insight_schema::metric::{filesystem_batch, zfs_dataset_batch, zfs_pool_batch};
use insight_schema::sample::{ZfsDatasetSample, ZfsPoolSample};
use insight_wire::metric::DatasetType;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::error::Result;

pub(crate) async fn run(
    tx: mpsc::Sender<(String, RecordBatch)>,
    node_id: String,
    cancel: CancellationToken,
) {
    let collect = async {
        let mut interval = tokio::time::interval(
            jiff::SignedDuration::from_secs(i64::from(
                insight_wire::metric::FILESYSTEM_SAMPLE_INTERVAL_SECS,
            ))
            .unsigned_abs(),
        );
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut filesystem_failed = false;
        let mut pools_failed = false;
        let mut datasets_failed = false;
        loop {
            interval.tick().await;
            let node = node_id.clone();
            let filesystem = tokio::task::spawn_blocking(move || {
                crate::filesystem::collect_round(jiff::Timestamp::now(), &node)
                    .map_err(std::io::Error::other)
            })
            .await
            .map_err(std::io::Error::other)
            .and_then(|result| result);
            if let Some(samples) = report(filesystem, &mut filesystem_failed, "filesystem") {
                crate::metric::queue(
                    &tx,
                    insight_wire::FILESYSTEM_METRIC_INDEX_ID,
                    filesystem_batch(&samples),
                );
            }
            let deadline = Instant::now() + jiff::SignedDuration::from_secs(30).unsigned_abs();
            if let Some(samples) = report(
                pools(&node_id, deadline).await,
                &mut pools_failed,
                "ZFS pool",
            ) {
                crate::metric::queue(
                    &tx,
                    insight_wire::ZFS_POOL_METRIC_INDEX_ID,
                    zfs_pool_batch(&samples),
                );
            }
            if let Some(samples) = report(
                datasets(&node_id, deadline).await,
                &mut datasets_failed,
                "ZFS dataset",
            ) {
                crate::metric::queue(
                    &tx,
                    insight_wire::ZFS_DATASET_METRIC_INDEX_ID,
                    zfs_dataset_batch(&samples),
                );
            }
        }
    };
    tokio::select! {
        () = cancel.cancelled() => {},
        () = collect => {},
    }
}

fn report<T, E: std::fmt::Debug>(
    result: std::result::Result<T, E>,
    failed: &mut bool,
    source: &str,
) -> Option<T> {
    match result {
        Ok(value) => {
            if std::mem::replace(failed, false) {
                tracing::info!("{source} metrics collection recovered");
            }
            Some(value)
        }
        Err(error) => {
            if !std::mem::replace(failed, true) {
                tracing::warn!(error = ?error, "{source} metrics collection failed");
            }
            None
        }
    }
}

async fn query(program: &str, args: &[&str], deadline: Instant) -> Result<String> {
    let bytes = host_exec::execute("cdelivery-insight-zfs.service", program, args, deadline)
        .await?
        .success()?;
    Ok(String::from_utf8(bytes).map_err(std::io::Error::other)?)
}

async fn pools(node_id: &str, deadline: Instant) -> Result<Vec<ZfsPoolSample>> {
    let output = query(
        "/usr/sbin/zpool",
        &["list", "-Hp", "-o", "name,size,allocated,free"],
        deadline,
    )
    .await?;
    let timestamp = jiff::Timestamp::now();
    output
        .lines()
        .map(|row| {
            let fields: Vec<_> = row.split('\t').collect();
            let [pool, size, allocated, free] = fields.as_slice() else {
                return Err(std::io::Error::other(format!("invalid ZFS pool row: {row}")).into());
            };
            Ok(ZfsPoolSample {
                timestamp,
                node_id: node_id.to_owned(),
                pool: (*pool).to_owned(),
                size_bytes: size.parse().map_err(std::io::Error::other)?,
                allocated_bytes: allocated.parse().map_err(std::io::Error::other)?,
                free_bytes: free.parse().map_err(std::io::Error::other)?,
            })
        })
        .collect()
}

async fn datasets(node_id: &str, deadline: Instant) -> Result<Vec<ZfsDatasetSample>> {
    let output = query("/usr/sbin/zfs", &["list", "-Hp", "-t", "filesystem,volume", "-o",
        "name,type,used,available,referenced,volsize,usedbydataset,usedbysnapshots,usedbychildren,usedbyrefreservation,quota,refquota,reservation,refreservation"], deadline).await?;
    let timestamp = jiff::Timestamp::now();
    output
        .lines()
        .map(|row| {
            let fields: Vec<_> = row.split('\t').collect();
            let [
                dataset,
                kind,
                used,
                available,
                referenced,
                volsize,
                own,
                snapshots,
                children,
                reserved,
                quota,
                refquota,
                reservation,
                refreservation,
            ] = fields.as_slice()
            else {
                return Err(
                    std::io::Error::other(format!("invalid ZFS dataset row: {row}")).into(),
                );
            };
            let dataset_type = match *kind {
                "filesystem" => DatasetType::Filesystem,
                "volume" => DatasetType::Volume,
                _ => {
                    return Err(
                        std::io::Error::other(format!("invalid ZFS dataset type: {kind}")).into(),
                    );
                }
            };
            let optional = |value: &str| -> std::io::Result<Option<u64>> {
                if value == "-" {
                    Ok(None)
                } else {
                    value.parse().map(Some).map_err(std::io::Error::other)
                }
            };
            Ok(ZfsDatasetSample {
                timestamp,
                node_id: node_id.to_owned(),
                dataset: (*dataset).to_owned(),
                dataset_type,
                used_bytes: used.parse().map_err(std::io::Error::other)?,
                available_bytes: available.parse().map_err(std::io::Error::other)?,
                referenced_bytes: referenced.parse().map_err(std::io::Error::other)?,
                volsize_bytes: optional(volsize)?,
                usedbydataset_bytes: optional(own)?,
                usedbysnapshots_bytes: optional(snapshots)?,
                usedbychildren_bytes: optional(children)?,
                usedbyrefreservation_bytes: optional(reserved)?,
                quota_bytes: optional(quota)?,
                refquota_bytes: optional(refquota)?,
                reservation_bytes: optional(reservation)?,
                refreservation_bytes: optional(refreservation)?,
            })
        })
        .collect()
}
