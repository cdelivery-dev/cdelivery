use std::sync::Arc;

use axum::ServiceExt;
use axum::extract::Request;
use cipher::MasterKey;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tower::Layer;
use tower_http::normalize_path::NormalizePathLayer;

use crate::config::{INTERNAL_LISTEN_ADDR, LISTEN_ADDR};
use crate::cron;
use crate::error::Result;
use crate::infra;
use crate::{agent, http};

pub(crate) struct Setup {
    pub(crate) docker: infra::docker::Docker,
    pub(crate) lock: Mutex<()>,
}

pub(crate) struct App {
    pub(crate) docker: infra::docker::Docker,
    pub(crate) key: Arc<MasterKey>,
    pub(crate) releases: infra::release::Catalog,
    pub(crate) insight: infra::insight::query::Api,
    pub(crate) challenges: infra::acme::Challenges,
    pub(crate) agent_reload: Notify,
}

pub(crate) async fn run() -> Result<()> {
    let cancel = CancellationToken::new();
    let running = run_until_stopped(&cancel);
    tokio::pin!(running);

    tokio::select! {
        result = &mut running => return result,
        () = logging::shutdown_signal() => {
            tracing::info!("shutdown signal received");
            cancel.cancel();
        }
    }
    match tokio::time::timeout(crate::config::SHUTDOWN_TIMEOUT.unsigned_abs(), running).await {
        Ok(result) => result,
        Err(error) => {
            tracing::warn!(error = ?error, "shutdown drain timed out");
            Ok(())
        }
    }
}

async fn run_until_stopped(cancel: &CancellationToken) -> Result<()> {
    let releases_url = crate::config::releases_url()?;
    let docker = infra::docker::Docker::connect(docker::DEFAULT_SOCKET_PATH).await?;

    if let Some(key) = infra::keyring::load()? {
        tracing::info!("master key mounted");
        infra::instance::converge(&docker).await?;
        let key = Arc::new(key);
        let app = Arc::new(App {
            docker,
            key,
            releases: infra::release::Catalog::new(releases_url)?,
            insight: infra::insight::query::Api::new()?,
            challenges: infra::acme::Challenges::default(),
            agent_reload: Notify::new(),
        });
        let public = http::app_router(Arc::clone(&app), cancel.child_token())?;
        let internal = http::internal::router(Arc::clone(&app));
        let public_listener = bind(LISTEN_ADDR).await?;
        let internal_listener = bind(INTERNAL_LISTEN_ADDR).await?;
        let tracker = TaskTracker::new();
        let records_cancel = CancellationToken::new();
        let records = tokio::spawn(infra::write_records(records_cancel.clone()));
        cron::start(&tracker, cancel, &app);
        tracker.spawn(agent::duty::alerts(Arc::clone(&app), cancel.child_token()));
        tracker.spawn(agent::supervisor::run(
            Arc::clone(&app),
            cancel.child_token(),
        ));
        let (public, internal) = tokio::join!(
            serve(public, cancel, public_listener),
            serve(internal, cancel, internal_listener),
        );
        tracker.close();
        tracker.wait().await;
        records_cancel.cancel();
        if let Err(error) = records.await {
            tracing::warn!(error = ?error, "record writer failed during shutdown");
        }
        public.and(internal)
    } else {
        tracing::info!("master key not mounted; serving setup");
        let setup = Arc::new(Setup {
            docker,
            lock: Mutex::new(()),
        });
        let router = http::setup_router(setup);
        let listener = bind(LISTEN_ADDR).await?;
        serve(router, cancel, listener).await
    }
}

async fn bind(addr: (&str, u16)) -> Result<TcpListener> {
    let listener = TcpListener::bind(addr).await?;
    tracing::info!("listening on {}", listener.local_addr()?);
    Ok(listener)
}

async fn serve(
    router: axum::Router,
    cancel: &CancellationToken,
    listener: TcpListener,
) -> Result<()> {
    let router = NormalizePathLayer::trim_trailing_slash().layer(router);
    axum::serve(listener, ServiceExt::<Request>::into_make_service(router))
        .with_graceful_shutdown(cancel.clone().cancelled_owned())
        .await?;
    Ok(())
}
