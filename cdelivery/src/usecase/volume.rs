use std::collections::BTreeMap;

use cdelivery_wire::Command;
use cipher::MasterKey;

use crate::error::{ErrorKind, Result};
use crate::{concept, infra};

pub(crate) async fn get(
    docker: &infra::docker::Docker,
) -> Result<Option<(concept::image::Reference, bool)>> {
    infra::volume::observe(docker).await
}

pub(crate) async fn enable(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    releases: &infra::release::Catalog,
    version: Option<&str>,
    zfs_enabled: Option<bool>,
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
    infra::volume::configure(docker, key, image.as_ref(), zfs_enabled).await
}

pub(crate) async fn disable(docker: &infra::docker::Docker) -> Result<()> {
    infra::volume::delete_agent(docker).await
}

pub(crate) async fn resolve_mounts(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    node: &str,
    mounts: &[concept::app::Mount],
) -> Result<Vec<concept::app::Mount>> {
    let report: cdelivery_wire::Report =
        infra::volume::request(docker, key, node.to_owned(), Command::List { sizes: false })
            .await?;
    mounts
        .iter()
        .map(|mount| {
            let backend = report
                .volumes
                .get(mount.volume())
                .filter(|volume| volume.ready)
                .and_then(|volume| volume.backend)
                .ok_or(ErrorKind::VolumeNotFound)?;
            Ok(concept::app::Mount::new(
                mount.target().clone(),
                mount.volume().clone(),
                mount.read_only(),
                backend,
            ))
        })
        .collect()
}

pub(crate) async fn require_guardian(
    docker: &infra::docker::Docker,
    app: &concept::app::App,
) -> Result<()> {
    if let Some(mounts) = app.mounts()
        && (!get(docker).await?.is_some_and(|(_, enabled)| enabled)
            || mounts
                .mounts()
                .iter()
                .any(|mount| mount.backend() != cdelivery_wire::Backend::Zfs))
    {
        return Err(ErrorKind::GuardianStatefulUnsupported.into());
    }
    Ok(())
}

pub(crate) async fn delete_guardian(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    id: String,
    name: &concept::name::Name,
) -> Result<()> {
    let _guard = super::guardian::acquire(name)?;
    if infra::app::guardian::exists(docker, name).await? {
        return Err(ErrorKind::GuardianPending.into());
    }
    infra::volume::request::<()>(
        docker,
        key,
        id,
        Command::DeleteGuardian {
            app: name.to_string(),
        },
    )
    .await
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
    if !report.volumes.get(&name).is_some_and(|volume| volume.ready) {
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
