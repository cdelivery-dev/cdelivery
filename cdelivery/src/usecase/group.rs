use insight_wire::event::Kind;

use crate::concept;
use crate::error::{ErrorKind, Result};
use crate::infra;

pub(crate) async fn create(
    docker: &infra::docker::Docker,
    name: concept::name::Name,
    description: Option<String>,
) -> Result<concept::group::Group> {
    let group = infra::group::create(docker, name, description).await?;
    infra::event::emit(Kind::GroupCreated, group.name().as_str(), "");
    Ok(group)
}

#[expect(clippy::option_option)]
pub(crate) async fn update(
    docker: &infra::docker::Docker,
    name: &concept::name::Name,
    description: Option<Option<String>>,
) -> Result<concept::group::Group> {
    let mut group =
        super::require(infra::group::load(docker, name), ErrorKind::GroupNotFound).await?;
    if let Some(description) = description {
        group.get_mut().set_description(description);
    }
    let applied = group.changed();
    infra::group::save(docker, &group).await?;
    if applied {
        infra::event::emit(Kind::GroupUpdated, name.as_str(), "");
    }
    Ok(group.into_inner())
}

pub(crate) async fn delete(
    docker: &infra::docker::Docker,
    name: &concept::name::Name,
) -> Result<()> {
    if let Some((network_id, deleted)) = infra::group::delete(docker, name).await? {
        if deleted {
            infra::event::emit(Kind::GroupDeleted, name.as_str(), "");
        }
        infra::group::finish_delete(docker, name, &network_id).await?;
    }
    Ok(())
}

pub(crate) async fn list(docker: &infra::docker::Docker) -> Result<Vec<concept::group::Group>> {
    infra::group::list(docker).await
}

pub(crate) async fn get(
    docker: &infra::docker::Docker,
    name: &concept::name::Name,
) -> Result<concept::group::Group> {
    super::found(infra::group::load(docker, name), ErrorKind::GroupNotFound).await
}
