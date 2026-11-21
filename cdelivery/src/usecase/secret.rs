use crate::{concept, error, infra};

pub(crate) async fn create(
    docker: &infra::docker::Docker,
    name: concept::name::Name,
    kind: concept::secret::Kind,
    description: Option<String>,
    data: Vec<u8>,
) -> error::Result<()> {
    if data.is_empty() {
        return Err(error::ErrorKind::FieldRequired("content".to_owned()).into());
    }
    if kind == concept::secret::Kind::Text && data.contains(&0) {
        return Err(error::ErrorKind::SecretTextNul.into());
    }
    infra::secret::create(docker, &name, kind, description, &data).await?;
    infra::event::emit(
        insight_wire::event::Kind::SecretCreated,
        name.as_str(),
        kind.as_ref(),
    );
    Ok(())
}

pub(crate) async fn list(
    docker: &infra::docker::Docker,
    kind: Option<concept::secret::Kind>,
) -> error::Result<Vec<concept::secret::Secret>> {
    infra::secret::list(docker, kind).await
}

pub(crate) async fn validate_reference(
    docker: &infra::docker::Docker,
    reference: &concept::secret::Reference,
    kind: concept::secret::Kind,
) -> error::Result<()> {
    let secret = super::found(
        infra::secret::load(docker, reference.name()),
        error::ErrorKind::SecretNotFound,
    )
    .await?;
    if secret.reference() != reference || secret.kind() != kind {
        return Err(error::ErrorKind::SecretBindingInvalid.into());
    }
    Ok(())
}

pub(crate) async fn delete(
    docker: &infra::docker::Docker,
    name: &concept::name::Name,
) -> error::Result<()> {
    let Some(secret) = infra::secret::load(docker, name).await? else {
        return Ok(());
    };
    if let Some(app) = infra::app::secret_in_use(docker, secret.get().reference().id()).await? {
        return Err(error::ErrorKind::SecretInUse(app).into());
    }
    infra::secret::delete(docker, name).await?;
    infra::event::emit(insight_wire::event::Kind::SecretDeleted, name.as_str(), "");
    Ok(())
}
