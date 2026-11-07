mod cgroup2;
mod config;
mod error;
mod filesystem;
mod fluentd;
mod host;
mod metric;
mod netstat;
mod procfs;
mod sender;

use tokio_util::sync::CancellationToken;

use crate::error::Result;

#[tokio::main]
async fn main() -> Result<()> {
    logging::init()?;
    let node_id = config::node_id_from_env()?;
    let docker = docker::Client::connect(docker::DEFAULT_SOCKET_PATH).await?;
    tracing::info!("starting insight agent on node {node_id}");

    let cancel = CancellationToken::new();
    let mut tasks = tokio::task::JoinSet::new();
    let fluentd_cancel = cancel.clone();
    tasks.spawn(async move {
        if let Err(e) = fluentd::start(fluentd_cancel).await {
            tracing::error!(error = ?e, "fluentd task failed");
        }
    });
    let metric_cancel = cancel.clone();
    tasks.spawn(async move {
        if let Err(e) = metric::start(docker, node_id, metric_cancel).await {
            tracing::error!(error = ?e, "metric task failed");
        }
    });

    Ok(logging::supervise(tasks, cancel).await?)
}
