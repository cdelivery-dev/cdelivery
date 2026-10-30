use std::collections::HashMap;
use std::path::PathBuf;

use arrow::record_batch::RecordBatch;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{cgroup2, filesystem, host};
use insight_schema::metric::{container_batch, filesystem_batch, host_batch};
use insight_schema::sample::{ContainerSample, HostSample};

const SENDER_CHANNEL_CAPACITY: usize = 1024;

#[derive(Debug)]
struct DeltaCollector {
    previous: HashMap<String, ContainerSample>,
    previous_uptime_ms: Option<u64>,
    max_elapsed_ms: u64,
}

impl DeltaCollector {
    fn new(interval_secs: u16) -> Self {
        Self {
            previous: HashMap::new(),
            previous_uptime_ms: None,
            max_elapsed_ms: u64::from(interval_secs).saturating_mul(2000),
        }
    }

    fn collect(
        &mut self,
        host: &HostSample,
        mut samples: Vec<ContainerSample>,
    ) -> insight_schema::Result<RecordBatch> {
        let previous_uptime = self.previous_uptime_ms.replace(host.node_uptime_ms);
        let elapsed_ms = previous_uptime
            .and_then(|uptime| host.node_uptime_ms.checked_sub(uptime))
            .filter(|elapsed| (1..=self.max_elapsed_ms).contains(elapsed));
        for sample in &mut samples {
            sample.elapsed_ms = elapsed_ms;
            let previous = self.previous.get(&sample.container_id).filter(|previous| {
                elapsed_ms.is_some() && previous.container_started_at == sample.container_started_at
            });
            macro_rules! deltas {
                ($previous:expr, $($field:ident => $counter:ident),+ $(,)?) => {
                    $(sample.$field = $previous.and_then(|previous| {
                        sample.$counter?.checked_sub(previous.$counter?)
                    });)+
                };
            }
            deltas!(previous,
                cpu_usage_usec_delta => cpu_usage_usec_total,
                cpu_throttled_usec_delta => cpu_throttled_usec_total,
                memory_oom_kills_delta => memory_oom_kills_total,
                psi_cpu_some_usec_delta => psi_cpu_some_usec_total,
                psi_memory_some_usec_delta => psi_memory_some_usec_total,
                psi_memory_full_usec_delta => psi_memory_full_usec_total,
                psi_io_some_usec_delta => psi_io_some_usec_total,
                psi_io_full_usec_delta => psi_io_full_usec_total,
                tcp_retrans_segs_delta => tcp_retrans_segs_total,
            );
            deltas!(previous.filter(|previous| sample.disk_device_set == previous.disk_device_set),
                disk_read_bytes_delta => disk_read_bytes_total,
                disk_write_bytes_delta => disk_write_bytes_total,
                disk_read_ops_delta => disk_read_ops_total,
                disk_write_ops_delta => disk_write_ops_total,
            );
            deltas!(previous.filter(|previous| sample.network_interface_set == previous.network_interface_set),
                network_receive_bytes_delta => network_receive_bytes_total,
                network_transmit_bytes_delta => network_transmit_bytes_total,
            );
        }
        let batch = container_batch(&samples);
        self.previous = samples
            .into_iter()
            .map(|sample| (sample.container_id.clone(), sample))
            .collect();
        batch
    }
}

pub(crate) async fn start(
    docker: docker::Client,
    node_id: String,
    cancel: CancellationToken,
) -> crate::error::Result<()> {
    let interval_secs = crate::config::sample_interval_from_env()?;
    let (tx, rx) = mpsc::channel::<(String, RecordBatch)>(SENDER_CHANNEL_CAPACITY);
    let sender_handle =
        tokio_util::task::AbortOnDropHandle::new(tokio::spawn(crate::sender::send_worker(
            format!(
                "{}:{}",
                insight_wire::SERVICE_NAME,
                insight_wire::INGEST_PORT
            ),
            rx,
        )));

    let proc_path = PathBuf::from(crate::config::HOST_PROC_DIR);
    let mut host_collector =
        host::HostCollector::new(proc_path).map_err(crate::error::ErrorKind::Config)?;
    let mut cgroup_collector = cgroup2::Cgroup2::new(
        docker,
        node_id.clone(),
        crate::config::HOST_MOUNTINFO_PATH,
        insight_wire::HOST_MOUNT_PREFIX.to_owned(),
    )
    .map_err(crate::error::ErrorKind::Config)?;
    let mut filesystem_collector = filesystem::Collector::default();
    let mut delta_collector = DeltaCollector::new(interval_secs);

    tracing::info!("collecting metrics every {interval_secs}s");
    let mut interval =
        tokio::time::interval(std::time::Duration::from_secs(u64::from(interval_secs)));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    'collect: loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => {
                tracing::info!("stopping metric collection");
                break;
            }
            _ = interval.tick() => {
                let reconciled = tokio::select! {
                    biased;
                    () = cancel.cancelled() => break 'collect,
                    result = cgroup_collector.reconcile() => result,
                };
                let now = jiff::Timestamp::now();
                let filesystem_samples = filesystem_collector.collect_due(now, &node_id);
                if !filesystem_samples.is_empty() {
                    queue(
                        &tx,
                        insight_wire::FILESYSTEM_METRIC_INDEX_ID,
                        filesystem_batch(&filesystem_samples),
                    );
                }
                let Some(host_sample) = host_collector.collect(now, &node_id) else {
                    tracing::error!("failed to collect host sample, skipping metric round");
                    delta_collector = DeltaCollector::new(interval_secs);
                    continue;
                };
                let container_samples = match reconciled {
                    Ok(()) => cgroup_collector.collect(&host_sample),
                    Err(e) => {
                        tracing::error!(error = ?e, "failed to reconcile container metrics");
                        Vec::new()
                    }
                };
                queue(
                    &tx,
                    insight_wire::HOST_METRIC_INDEX_ID,
                    host_batch(std::slice::from_ref(&host_sample)),
                );
                queue(
                    &tx,
                    insight_wire::CONTAINER_METRIC_INDEX_ID,
                    delta_collector.collect(&host_sample, container_samples),
                );
            }
        }
    }
    drop(tx);
    crate::sender::finish_sender(sender_handle).await;
    Ok(())
}

fn queue(
    tx: &mpsc::Sender<(String, RecordBatch)>,
    index_id: &str,
    batch: insight_schema::Result<RecordBatch>,
) {
    match batch {
        Ok(batch) if batch.num_rows() == 0 => {}
        Ok(batch) => {
            if let Err(e) = tx.try_send((index_id.to_owned(), batch)) {
                tracing::warn!(error = ?e, "failed to queue {index_id} batch for sending");
            }
        }
        Err(e) => tracing::error!(error = ?e, "failed to build {index_id} batch"),
    }
}
