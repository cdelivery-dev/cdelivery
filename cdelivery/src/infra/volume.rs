use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};

use docker::types::{
    ContainerSpec, Mount, MountType, NanoDuration, NetworkAttachment, ServiceFileTarget,
    ServiceMode, ServiceSecret, ServiceSpec, TaskState, TaskTemplate,
};
use serde::de::DeserializeOwned;

use crate::error::{self, ErrorKind, Result};
use crate::{concept, config};

const LABEL_PREFIX: &str = "dev.cdelivery.volume.";
const DESCRIPTION_SUFFIX: &str = ".description";

pub(crate) async fn observe_descriptions(
    docker: &super::docker::Docker,
    id: &str,
) -> Result<Option<BTreeMap<String, String>>> {
    Ok(docker.node_inspect(id).await?.map(|node| {
        node.spec
            .labels
            .into_iter()
            .filter_map(|(key, value)| {
                let name = key
                    .strip_prefix(LABEL_PREFIX)?
                    .strip_suffix(DESCRIPTION_SUFFIX)?;
                (!value.is_empty()).then(|| (name.to_owned(), value))
            })
            .collect()
    }))
}

pub(crate) async fn describe(
    docker: &super::docker::Docker,
    id: &str,
    name: &str,
    description: Option<&str>,
) -> Result<()> {
    let mut node = docker
        .node_inspect(id)
        .await?
        .ok_or(ErrorKind::NodeNotFound)?;
    let key = format!("{LABEL_PREFIX}{name}{DESCRIPTION_SUFFIX}");
    let description = description.filter(|value| !value.is_empty());
    if node.spec.labels.get(&key).map(String::as_str) == description {
        return Ok(());
    }
    if let Some(description) = description {
        node.spec.labels.insert(key, description.to_owned());
    } else {
        node.spec.labels.remove(&key);
    }
    docker.node_update(id, node.version.index, &node.spec).await
}

pub(crate) async fn observe(
    docker: &super::docker::Docker,
) -> Result<Option<concept::image::Reference>> {
    docker
        .service_inspect(config::VOLUME_AGENT_SERVICE_NAME)
        .await?
        .map(|service| service.spec.task_template.container_spec.image.parse())
        .transpose()
}

async fn create_agent(
    docker: &super::docker::Docker,
    key: &cipher::MasterKey,
    image: &concept::image::Reference,
) -> Result<()> {
    let secret = key_secret(docker, key).await?;
    let mut spec = ServiceSpec {
        name: config::VOLUME_AGENT_SERVICE_NAME.to_owned(),
        labels: std::collections::HashMap::new(),
        task_template: TaskTemplate {
            container_spec: ContainerSpec {
                image: image.to_string(),
                stop_grace_period: Some(NanoDuration::from_secs(30)),
                env: vec![format!("{}={{{{.Node.ID}}}}", cdelivery_wire::NODE_ID_ENV)],
                secrets: vec![secret],
                mounts: vec![
                    Mount {
                        kind: MountType::Bind,
                        source: docker::DEFAULT_SOCKET_PATH.to_owned(),
                        target: docker::DEFAULT_SOCKET_PATH.to_owned(),
                        read_only: true,
                        bind_options: None,
                    },
                    Mount {
                        kind: MountType::Volume,
                        source: config::VOLUME_AGENT_SERVICE_NAME.to_owned(),
                        target: cdelivery_wire::DISK_PATH.to_owned(),
                        read_only: true,
                        bind_options: None,
                    },
                ],
                ..ContainerSpec::default()
            },
            networks: Some(vec![NetworkAttachment {
                target: config::NETWORK_NAME.to_owned(),
                aliases: None,
                driver_opts: None,
            }]),
            ..TaskTemplate::default()
        },
        mode: ServiceMode::Global {},
        update_config: None,
        rollback_config: None,
        endpoint_spec: None,
        extra: serde_json::Map::new(),
    };
    super::platform::monitor_updates(&mut spec);
    match docker.service_create(&spec, None).await {
        Ok(_) => Ok(()),
        Err(error) if error.is_conflict() => {
            tracing::warn!("volume agent service already exists; leaving it in place");
            Ok(())
        }
        Err(error) => Err(error),
    }
}

pub(crate) async fn configure(
    docker: &super::docker::Docker,
    key: &cipher::MasterKey,
    image: Option<&concept::image::Reference>,
) -> Result<()> {
    let Some(mut service) = docker
        .service_inspect(config::VOLUME_AGENT_SERVICE_NAME)
        .await?
    else {
        let image = error::required(image, "version")?;
        return create_agent(docker, key, image).await;
    };
    let secrets = vec![key_secret(docker, key).await?];
    let container = &mut service.spec.task_template.container_spec;
    let mut changed = container.secrets != secrets;
    container.secrets = secrets;
    if let Some(image) = image
        && let Some(replacement) = image.replacement_for(&container.image)?
    {
        container.image = replacement;
        changed = true;
    }
    if changed {
        container.stop_grace_period = Some(NanoDuration::from_secs(30));
        super::platform::monitor_updates(&mut service.spec);
        docker
            .service_update(
                config::VOLUME_AGENT_SERVICE_NAME,
                service.version.index,
                &service.spec,
                None,
            )
            .await?;
    }
    Ok(())
}

pub(crate) async fn delete_agent(docker: &super::docker::Docker) -> Result<()> {
    docker
        .service_delete(config::VOLUME_AGENT_SERVICE_NAME)
        .await?;
    docker.secret_delete(cdelivery_wire::KEY_SECRET_NAME).await
}

async fn key_secret(
    docker: &super::docker::Docker,
    key: &cipher::MasterKey,
) -> Result<ServiceSecret> {
    let secret_id = super::keyring::ensure_subkey_secret(
        docker,
        key,
        cdelivery_wire::KEY_SECRET_NAME,
        cipher::Domain::Volume,
    )
    .await?;
    Ok(ServiceSecret {
        file: ServiceFileTarget {
            name: cdelivery_wire::KEY_SECRET_NAME.to_owned(),
            ..ServiceFileTarget::default()
        },
        secret_id,
        secret_name: cdelivery_wire::KEY_SECRET_NAME.to_owned(),
    })
}

pub(crate) async fn request<T: DeserializeOwned>(
    docker: &super::docker::Docker,
    key: &cipher::MasterKey,
    node_id: String,
    command: cdelivery_wire::Command,
) -> Result<T> {
    let task = docker
        .task_list(config::VOLUME_AGENT_SERVICE_NAME)
        .await?
        .into_iter()
        .find(|task| {
            task.node_id == node_id
                && task.desired_state == TaskState::Running
                && task.status.state == TaskState::Running
        })
        .ok_or(ErrorKind::VolumeUnavailable)?;
    let address = task
        .networks_attachments
        .iter()
        .flat_map(|network| &network.addresses)
        .next()
        .ok_or(ErrorKind::VolumeUnavailable)?;
    let (ip, _) = address
        .split_once('/')
        .ok_or(ErrorKind::VolumeUnavailable)?;
    let ip = ip.parse::<IpAddr>().map_err(std::io::Error::other)?;
    let address = SocketAddr::new(ip, cdelivery_wire::PORT);
    let request = cdelivery_wire::Request { node_id, command };
    let body = key.cipher_for(cipher::Domain::Volume).seal(&request)?;
    let response = reqwest::Client::builder()
        .build()?
        .post(format!("http://{address}/"))
        .body(body)
        .send()
        .await?;
    let conflict = response.status() == reqwest::StatusCode::CONFLICT;
    if conflict && matches!(&request.command, cdelivery_wire::Command::Create { .. }) {
        return Err(ErrorKind::VolumeExists.into());
    }
    if let Err(source) = response.error_for_status_ref() {
        let bytes = response.bytes().await?;
        let error = error::Error::from(ErrorKind::VolumeAgentHttp {
            source,
            detail: String::from_utf8_lossy(&bytes).into_owned(),
        });
        if conflict && matches!(&request.command, cdelivery_wire::Command::Delete { .. }) {
            return Err(ErrorKind::VolumeInUse(Box::new(error)).into());
        }
        return Err(error);
    }
    Ok(serde_json::from_slice(&response.bytes().await?)?)
}
