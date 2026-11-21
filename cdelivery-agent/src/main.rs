mod error;
mod zfs;

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::post};
use cdelivery_wire::{Backend, Command, name};
use tokio::time::Instant;
use zeroize::Zeroizing;

use crate::error::{ErrorKind, Result};

#[derive(Debug)]
struct Agent {
    docker: docker::Client,
    cipher: cipher::Cipher,
    node_id: String,
    zfs_enabled: bool,
    busy: tokio::sync::Mutex<()>,
}

#[tokio::main]
async fn main() -> Result<()> {
    logging::init()?;
    let docker = docker::Client::connect(docker::DEFAULT_SOCKET_PATH).await?;
    let node_id = std::env::var(cdelivery_wire::NODE_ID_ENV)?;
    let zfs_enabled = match std::env::var(cdelivery_wire::ZFS_ENABLED_ENV)?.as_str() {
        "true" => true,
        "false" => false,
        _ => return Err(ErrorKind::Zfs("invalid CDELIVERY_ZFS_ENABLED".to_owned()).into()),
    };
    let cipher = {
        let bytes = Zeroizing::new(std::fs::read(format!(
            "/run/secrets/{}",
            cdelivery_wire::KEY_SECRET_NAME,
        ))?);
        let key: &[u8; 32] = bytes.as_slice().try_into()?;
        cipher::Cipher::from_key(key)
    };
    let agent = Arc::new(Agent {
        docker,
        cipher,
        node_id,
        zfs_enabled,
        busy: tokio::sync::Mutex::new(()),
    });
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", cdelivery_wire::PORT)).await?;
    axum::serve(
        listener,
        Router::new().route("/", post(handle)).with_state(agent),
    )
    .with_graceful_shutdown(logging::shutdown_signal())
    .await?;
    Ok(())
}

#[expect(clippy::too_many_lines)]
async fn handle(State(agent): State<Arc<Agent>>, body: String) -> Result<Response> {
    let Ok(request) = agent.cipher.open::<cdelivery_wire::Request>(&body) else {
        return Ok(StatusCode::UNAUTHORIZED.into_response());
    };
    if request.node_id != agent.node_id {
        return Ok(StatusCode::UNAUTHORIZED.into_response());
    }
    let valid = match &request.command {
        Command::Create { name } | Command::Delete { name } => name::volume(name),
        Command::PreparePreview {
            name: preview,
            names,
        } => {
            name::preview(preview)
                && !names.is_empty()
                && names.iter().all(|name| name::volume(name))
        }
        Command::DeletePreview { name } => name::preview(name),
        Command::List { .. } => true,
    };
    if !valid {
        return Ok(StatusCode::BAD_REQUEST.into_response());
    }
    let deadline = Instant::now() + jiff::SignedDuration::from_secs(180).unsigned_abs();
    tokio::time::timeout_at(deadline, async {
        let _guard = if matches!(request.command, Command::List { .. }) {
            agent.busy.lock().await
        } else {
            agent.busy.try_lock().map_err(|_| ErrorKind::Busy)?
        };
        let volumes = agent.docker.volumes();
        match request.command {
            Command::List { sizes } => {
                return Ok(Json(zfs::list(&agent, sizes, deadline).await?).into_response());
            }
            Command::Create { name } => {
                let volume_name = format!("{}{name}", cdelivery_wire::VOLUME_PREFIX);
                if volumes.inspect(&volume_name).await?.is_some()
                    || (agent.zfs_enabled
                        && zfs::datasets(deadline)
                            .await?
                            .contains_key(&format!("cdelivery/volumes/{name}")))
                {
                    return Err(ErrorKind::VolumeExists(name).into());
                }
                if agent.zfs_enabled {
                    zfs::create(&agent, &name, deadline).await?;
                } else {
                    volumes.create(&volume_name).await?;
                }
            }
            Command::Delete { name } => {
                let volume_name = format!("{}{name}", cdelivery_wire::VOLUME_PREFIX);
                let existing = volumes.inspect(&volume_name).await?;
                let backend = existing
                    .as_ref()
                    .and_then(|volume| zfs::backend(volume, &name));
                if existing.is_some() && backend.is_none() {
                    return Err(ErrorKind::InvalidVolume(name).into());
                }
                let datasets = if backend == Some(Backend::Local)
                    || (backend.is_none() && !agent.zfs_enabled)
                {
                    BTreeMap::new()
                } else {
                    zfs::datasets(deadline).await?
                };
                if backend == Some(Backend::Zfs) && !datasets.contains_key(cdelivery_wire::ZFS_POOL)
                {
                    return Err(ErrorKind::Zfs("pool is not imported".to_owned()).into());
                }
                volumes.delete(&volume_name).await.map_err(|source| {
                    if source.is_conflict() {
                        ErrorKind::VolumeInUse(source)
                    } else {
                        ErrorKind::Docker(source)
                    }
                })?;
                let dataset = format!("cdelivery/volumes/{name}");
                if datasets.contains_key(&dataset) {
                    zfs::destroy(&dataset, deadline).await?;
                }
            }
            Command::PreparePreview { name, names } => {
                zfs::prepare(&agent, &name, names, deadline).await?;
            }
            Command::DeletePreview { name } => {
                zfs::cleanup(
                    &agent,
                    &name,
                    &zfs::datasets(deadline).await?,
                    &volumes.list(false).await?,
                    deadline,
                )
                .await?;
            }
        }
        Ok(Json(()).into_response())
    })
    .await
    .map_err(|_| ErrorKind::Timeout)?
}
