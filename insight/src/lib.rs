pub mod config;
mod cpu;
pub mod error;
pub mod http;
pub mod ingest;
mod query;
pub mod storage;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use crate::error::{ErrorKind, Result};

fn bind_addr(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port)
}

pub async fn run() -> Result<()> {
    let config = config::Config::from_env()?;
    let (store, storage_runtime) =
        storage::store::Store::start(&config, std::path::Path::new(insight_wire::LOCAL_DATA_DIR))
            .await?;
    let ingest = Arc::new(ingest::Ingest::new(
        Arc::clone(&store),
        config.writer_flush_interval,
        config.ingest_memory_budget,
    ));

    let ingest_listener =
        tokio::net::TcpListener::bind(bind_addr(insight_wire::INGEST_PORT)).await?;
    let http_listener = tokio::net::TcpListener::bind(bind_addr(insight_wire::HTTP_PORT)).await?;
    tracing::info!(
        "insight listening on {} (http), {} (ingest)",
        insight_wire::HTTP_PORT,
        insight_wire::INGEST_PORT
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let mut tasks = tokio::task::JoinSet::new();
    let storage_cancel = cancel.clone();
    tasks.spawn(storage_runtime.run(storage_cancel));
    let http_store = Arc::clone(&store);
    let http_cancel = cancel.clone();
    tasks.spawn(async move {
        if let Err(e) = http::serve(http_store, http_listener, http_cancel.cancelled_owned()).await
        {
            tracing::error!(error = ?e, "http server exited with error");
        }
    });
    let ingest_cancel = cancel.clone();
    tasks.spawn(Arc::clone(&ingest).listen(ingest_listener, ingest_cancel.cancelled_owned()));

    Ok(logging::supervise(tasks, cancel).await?)
}

pub async fn healthz() -> Result<()> {
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), insight_wire::HTTP_PORT);
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        tokio::net::TcpStream::connect(addr),
    )
    .await
    .map_err(|source| ErrorKind::HealthzTimeout { addr, source })??;
    Ok(())
}
