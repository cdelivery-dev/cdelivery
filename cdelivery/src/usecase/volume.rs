use std::collections::BTreeMap;

use cdelivery_wire::Command;
use cipher::MasterKey;

use crate::error::{ErrorKind, Result};
use crate::{concept, infra};

pub(crate) async fn get(
    docker: &infra::docker::Docker,
) -> Result<Option<concept::image::Reference>> {
    infra::volume::observe(docker).await
}

pub(crate) async fn enable(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    releases: &infra::release::Catalog,
    version: Option<&str>,
) -> Result<()> {
    let image = version
        .map(|version| {
            super::release::resolve(
                releases,
                concept::release::Component::CdeliveryAgent,
                version,
                "version",
            )
        })
        .transpose()?;
    infra::volume::configure(docker, key, image.as_ref()).await
}

pub(crate) async fn disable(docker: &infra::docker::Docker) -> Result<()> {
    infra::volume::delete_agent(docker).await
}

pub(crate) async fn list(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    id: String,
    sizes: bool,
) -> Result<(cdelivery_wire::Report, BTreeMap<String, String>)> {
    let descriptions = super::require(
        infra::volume::observe_descriptions(docker, &id),
        ErrorKind::NodeNotFound,
    )
    .await?;
    let report = infra::volume::request(docker, key, id, Command::List { sizes }).await?;
    Ok((report, descriptions))
}

pub(crate) async fn create(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    id: String,
    name: String,
    description: Option<String>,
) -> Result<()> {
    infra::volume::request::<()>(
        docker,
        key,
        id.clone(),
        Command::Create { name: name.clone() },
    )
    .await?;
    infra::volume::describe(docker, &id, &name, description.as_deref())
        .await
        .map_err(|source| ErrorKind::VolumeDescriptionSave(Box::new(source)).into())
}

#[expect(clippy::option_option)]
pub(crate) async fn update(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    id: String,
    name: String,
    description: Option<Option<String>>,
) -> Result<()> {
    let report: cdelivery_wire::Report =
        infra::volume::request(docker, key, id.clone(), Command::List { sizes: false }).await?;
    if !report.volumes.contains_key(&name) {
        return Err(ErrorKind::VolumeNotFound.into());
    }
    if let Some(description) = description {
        infra::volume::describe(docker, &id, &name, description.as_deref()).await?;
    }
    Ok(())
}

pub(crate) async fn delete(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    id: String,
    name: String,
) -> Result<()> {
    infra::volume::request::<()>(
        docker,
        key,
        id.clone(),
        Command::Delete { name: name.clone() },
    )
    .await?;
    if let Err(error) = infra::volume::describe(docker, &id, &name, None).await {
        tracing::warn!(error = ?error, "volume {name} on node {id} description cleanup failed");
    }
    Ok(())
}
