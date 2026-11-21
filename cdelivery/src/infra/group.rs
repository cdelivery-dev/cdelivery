use std::collections::{HashMap, HashSet};

use docker::types::Config;

use crate::concept;
use crate::error::{Error, ErrorKind, Result};

const NETWORK_PREFIX: &str = "cdelivery-group-";
pub(super) const LABEL_GROUP: &str = "dev.cdelivery.group";

pub(crate) fn resource_name(name: &concept::name::Name) -> String {
    format!("{NETWORK_PREFIX}{name}")
}

fn group_name(resource: &str) -> Option<concept::name::Name> {
    resource.strip_prefix(NETWORK_PREFIX)?.parse().ok()
}

fn decode(config: &Config) -> Result<Option<String>> {
    Ok(serde_json::from_str(
        config
            .spec
            .labels
            .get(LABEL_GROUP)
            .map_or("", String::as_str),
    )?)
}

pub(crate) async fn exists(
    docker: &super::docker::Docker,
    name: &concept::name::Name,
) -> Result<bool> {
    Ok(docker
        .network_inspect(&resource_name(name))
        .await?
        .is_some())
}

pub(crate) async fn create(
    docker: &super::docker::Docker,
    name: concept::name::Name,
    description: Option<String>,
) -> Result<concept::group::Group> {
    let resource = resource_name(&name);
    let network_id = match docker
        .network_create(&resource, super::label::marker(LABEL_GROUP))
        .await
    {
        Ok(id) => id,
        Err(e) if e.is_conflict() => return Err(ErrorKind::GroupExists.into()),
        Err(e) => return Err(e),
    };
    let mut group = concept::group::Group::new(name, description, network_id);
    if let Err(error) = docker
        .config_create(
            group.network_id(),
            super::label::single(LABEL_GROUP, group.description()),
            super::docker::PLACEHOLDER,
        )
        .await
    {
        tracing::warn!(
            error = ?error,
            "group {} record create failed; created without description", group.name()
        );
        group.set_description(None);
    }
    Ok(group)
}

pub(crate) async fn load(
    docker: &super::docker::Docker,
    name: &concept::name::Name,
) -> Result<Option<concept::versioned::Versioned<concept::group::Group>>> {
    let Some(network) = docker.network_inspect(&resource_name(name)).await? else {
        return Ok(None);
    };
    if let Some(config) = docker.config_inspect(&network.id).await? {
        return Ok(Some(concept::versioned::Versioned::new(
            config.version.index,
            concept::group::Group::new(name.clone(), decode(&config)?, network.id),
        )));
    }
    Ok(Some(concept::versioned::Versioned::new(
        0,
        concept::group::Group::new(name.clone(), None, network.id),
    )))
}

pub(crate) async fn save(
    docker: &super::docker::Docker,
    group: &concept::versioned::Versioned<concept::group::Group>,
) -> Result<()> {
    if !group.changed() {
        return Ok(());
    }
    if group.version() == 0 {
        docker
            .config_create(
                group.get().network_id(),
                super::label::single(LABEL_GROUP, group.get().description()),
                super::docker::PLACEHOLDER,
            )
            .await?;
        return Ok(());
    }
    docker
        .config_update_labels(
            group.get().network_id(),
            group.version(),
            super::label::single(LABEL_GROUP, group.get().description()),
        )
        .await
}

pub(crate) async fn delete(
    docker: &super::docker::Docker,
    name: &concept::name::Name,
) -> Result<Option<(String, bool)>> {
    let resource = resource_name(name);
    let Some(network) = docker.network_inspect(&resource).await? else {
        return Ok(None);
    };
    let deleted = match docker.network_delete(&resource).await {
        Ok(deleted) => deleted,
        Err(e) if e.is_conflict() => return Err(ErrorKind::GroupBusy.into()),
        Err(e) => return Err(e),
    };
    Ok(Some((network.id, deleted)))
}

pub(crate) async fn finish_delete(
    docker: &super::docker::Docker,
    name: &concept::name::Name,
    network_id: &str,
) -> Result<()> {
    let resource = resource_name(name);
    let removed = async {
        loop {
            match docker.network_inspect(&resource).await? {
                Some(current) if current.id == network_id => {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                _ => return Ok::<(), Error>(()),
            }
        }
    };
    tokio::time::timeout(crate::config::GROUP_DELETE_WAIT.unsigned_abs(), removed)
        .await
        .map_err(|source| {
            Error::new(ErrorKind::OperationTimeout {
                operation: "group_delete",
                source,
            })
        })??;
    docker.config_delete(network_id).await
}

pub(crate) async fn list(docker: &super::docker::Docker) -> Result<Vec<concept::group::Group>> {
    let mut records = HashMap::new();
    for config in docker.config_list(LABEL_GROUP).await? {
        match decode(&config) {
            Ok(description) => {
                records.insert(config.spec.name.clone(), description);
            }
            Err(error) => {
                tracing::warn!(error = ?error, "undecodable group record {}", config.spec.name);
            }
        }
    }
    let mut groups = Vec::new();
    for network in docker.network_list(LABEL_GROUP).await? {
        let Some(name) = group_name(&network.name) else {
            tracing::warn!(
                "labeled network {} without a parsable group name",
                network.name
            );
            continue;
        };
        groups.push(concept::group::Group::new(
            name,
            records.remove(&network.id).flatten(),
            network.id,
        ));
    }
    groups.sort_by(|a, b| a.name().as_str().cmp(b.name().as_str()));
    Ok(groups)
}

pub(crate) async fn sweep(docker: &super::docker::Docker) -> Result<usize> {
    let configs = docker.config_list(LABEL_GROUP).await?;
    let network_ids = docker
        .network_list(LABEL_GROUP)
        .await?
        .into_iter()
        .map(|network| network.id)
        .collect::<HashSet<_>>();
    let mut removed = 0;
    for config in configs {
        if network_ids.contains(&config.spec.name) {
            continue;
        }
        tracing::warn!("collecting orphan group record {}", config.spec.name);
        docker.config_delete(&config.spec.name).await?;
        removed += 1;
    }
    Ok(removed)
}
