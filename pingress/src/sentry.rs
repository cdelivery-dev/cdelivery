use async_trait::async_trait;
use pingora::server::ShutdownWatch;
use pingora::services::background::BackgroundService;

use crate::cert;
use crate::config;
use crate::fetch;
use crate::route;

pub(crate) async fn apply(snapshot: pingress_wire::Snapshot) {
    cert::apply(&snapshot.certs);
    route::apply(&snapshot.apps).await;
}

pub(crate) async fn boot_fetch(client: &fetch::Client) -> pingress_wire::Snapshot {
    let mut backoff = config::BOOT_BACKOFF_START;
    loop {
        match client.fetch(None).await {
            Ok(Some(snapshot)) => return snapshot,
            Ok(None) => {
                tracing::warn!("boot fetch returned 204 without an etag; retrying");
            }
            Err(error) => {
                tracing::warn!(error = ?error, "boot config fetch failed; retrying in {backoff:?}");
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(config::BOOT_BACKOFF_MAX);
    }
}

pub(crate) struct Sentry {
    client: fetch::Client,
    boot_etag: String,
}

impl Sentry {
    pub(crate) fn new(client: fetch::Client, boot_etag: String) -> Self {
        Self { client, boot_etag }
    }

    async fn run(&self) {
        let mut etag = self.boot_etag.clone();
        loop {
            tokio::time::sleep(config::POLL_INTERVAL).await;
            match self.client.fetch(Some(&etag)).await {
                Ok(Some(snapshot)) => {
                    etag = snapshot.etag.clone();
                    apply(snapshot).await;
                }
                Ok(None) => route::refresh().await,
                Err(error) => {
                    tracing::error!(error = ?error, "config fetch failed; keeping last-known-good");
                    route::refresh().await;
                }
            }
        }
    }
}

#[async_trait]
impl BackgroundService for Sentry {
    async fn start(&self, mut shutdown: ShutdownWatch) {
        tokio::select! {
            _ = shutdown.changed() => {}
            () = self.run() => {}
        }
    }
}
