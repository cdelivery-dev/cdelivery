mod error;

pub use error::{Error, Result};

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{EnvFilter, Layer, Registry, fmt};

pub fn init() -> Result<()> {
    let default_level = if cfg!(debug_assertions) {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };
    init_with_default_level(default_level)
}

pub fn init_with_default_level(default_level: tracing::Level) -> Result<()> {
    let filter = EnvFilter::builder()
        .with_default_directive(default_level.into())
        .from_env_lossy();

    let registry = tracing_subscriber::registry().with(fmt_layer().with_filter(filter));

    #[cfg(feature = "tokio-console")]
    let registry = registry.with(console_subscriber::spawn());

    registry.try_init()?;
    Ok(())
}

fn fmt_layer() -> Box<dyn Layer<Registry> + Send + Sync> {
    let layer = fmt::layer();
    let json = match std::env::var("LOG_FORMAT").ok().as_deref() {
        Some("json") => true,
        Some("pretty") => false,
        _ => !cfg!(debug_assertions),
    };
    if json {
        layer
            .json()
            .flatten_event(true)
            .with_current_span(true)
            .boxed()
    } else {
        layer.pretty().boxed()
    }
}

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(20);

pub fn rate_limited_warn(counter: &AtomicU64, increment: u64, message: &str) {
    let seen = counter.fetch_add(increment, Ordering::Relaxed) + increment;
    if seen == increment || seen / 10_000 != (seen - increment) / 10_000 {
        tracing::warn!(occurrences = seen, "{message}");
    }
}

pub async fn supervise(mut tasks: JoinSet<()>, cancel: CancellationToken) -> Result<()> {
    let failed = tokio::select! {
        biased;
        first_exit = tasks.join_next() => {
            tracing::error!(task_result = ?first_exit, "core task exited unexpectedly, terminating");
            true
        }
        () = shutdown_signal() => {
            false
        }
    };

    cancel.cancel();
    let stopped = tokio::time::timeout(SHUTDOWN_TIMEOUT, async {
        while let Some(result) = tasks.join_next().await {
            if let Err(e) = result {
                tracing::error!(error = ?e, "task failed during shutdown");
            }
        }
    })
    .await
    .is_ok();
    if !stopped {
        tracing::warn!("tasks did not stop within {SHUTDOWN_TIMEOUT:?}, aborting them");
        tasks.abort_all();
    }

    if failed {
        Err(error::ErrorKind::CoreTaskExited.into())
    } else {
        Ok(())
    }
}

pub async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::warn!(error = ?error, "failed to listen for Ctrl-C");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                let _ = signal.recv().await;
            }
            Err(error) => {
                tracing::warn!(error = ?error, "failed to install SIGTERM handler");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
}
