mod error;

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::post};
use cdelivery_wire::{Command, name};
use zeroize::Zeroizing;

use crate::error::{ErrorKind, Result};

#[derive(Debug)]
struct Agent {
    docker: docker::Client,
    cipher: cipher::Cipher,
    node_id: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    logging::init()?;
    let docker = docker::Client::connect(docker::DEFAULT_SOCKET_PATH).await?;
    let node_id = std::env::var(cdelivery_wire::NODE_ID_ENV)?;
    let cipher = {
        let bytes = Zeroizing::new(std::fs::read(format!(
            "/run/secrets/{}",
            cdelivery_wire::KEY_SECRET_NAME
        ))?);
        let key: &[u8; 32] = bytes.as_slice().try_into()?;
        cipher::Cipher::from_key(key)
    };
    let agent = Arc::new(Agent {
        docker,
        cipher,
        node_id,
    });
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", cdelivery_wire::PORT)).await?;
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = axum::serve(
        listener,
        Router::new().route("/", post(handle)).with_state(agent),
    )
    .with_graceful_shutdown(async move {
        logging::shutdown_signal().await;
        tracing::info!("shutdown signal received");
        let _ = stop.send(());
    })
    .into_future();
    tokio::pin!(server);
    tokio::select! {
        result = &mut server => result?,
        _ = stopped => {
            match tokio::time::timeout(std::time::Duration::from_secs(25), server).await {
                Ok(result) => result?,
                Err(error) => tracing::warn!(error = ?error, "shutdown drain timed out"),
            }
        }
    }
    Ok(())
}

async fn handle(State(agent): State<Arc<Agent>>, body: String) -> Result<Response> {
    let Ok(request) = agent.cipher.open::<cdelivery_wire::Request>(&body) else {
        return Ok(StatusCode::UNAUTHORIZED.into_response());
    };
    if request.node_id != agent.node_id {
        return Ok(StatusCode::UNAUTHORIZED.into_response());
    }
    match &request.command {
        Command::Create { name } | Command::Delete { name }
            if !name::shape_ok(name) || name::platform(name) || name::volume_reserved(name) =>
        {
            return Ok(StatusCode::UNAUTHORIZED.into_response());
        }
        _ => {}
    }
    let volumes = agent.docker.volumes();
    match request.command {
        Command::List { sizes } => {
            let volumes = volumes
                .list(sizes)
                .await?
                .into_iter()
                .filter_map(|volume| {
                    let name = volume
                        .name
                        .strip_prefix(cdelivery_wire::VOLUME_PREFIX)
                        .filter(|name| {
                            *name != name::AGENT_VOLUME
                                && name::shape_ok(name)
                                && !name::platform(name)
                        })?
                        .to_owned();
                    let size = volume
                        .usage_data
                        .and_then(|usage| u64::try_from(usage.size).ok());
                    Some((name, size))
                })
                .collect();
            let fs =
                rustix::fs::statvfs(cdelivery_wire::DISK_PATH).map_err(std::io::Error::from)?;
            return Ok(Json(cdelivery_wire::Report {
                volumes,
                total_bytes: fs.f_blocks.saturating_mul(fs.f_frsize),
                available_bytes: fs.f_bavail.saturating_mul(fs.f_frsize),
            })
            .into_response());
        }
        Command::Create { name } => {
            let volume_name = format!("{}{name}", cdelivery_wire::VOLUME_PREFIX);
            if volumes.inspect(&volume_name).await?.is_some() {
                return Err(ErrorKind::VolumeExists(name).into());
            }
            volumes.create(&volume_name).await?;
        }
        Command::Delete { name } => {
            volumes
                .delete(&format!("{}{name}", cdelivery_wire::VOLUME_PREFIX))
                .await
                .map_err(|source| {
                    if source.is_conflict() {
                        ErrorKind::VolumeInUse(source)
                    } else {
                        ErrorKind::Docker(source)
                    }
                })?;
        }
    }
    Ok(Json(()).into_response())
}
