use std::collections::HashMap;

use crate::{concept, error};

const PREFIX: &str = "cdelivery-secret-";
const LABEL_KIND: &str = "dev.cdelivery.secret_type";
const LABEL_DESCRIPTION: &str = "dev.cdelivery.description";

pub(crate) fn resource_name(name: &concept::name::Name) -> String {
    format!("{PREFIX}{name}")
}

pub(crate) fn reference(name: &str, id: &str) -> error::Result<concept::secret::Reference> {
    Ok(concept::secret::Reference::new(
        name.strip_prefix(PREFIX)
            .ok_or(error::ErrorKind::SecretBindingInvalid)?
            .parse()?,
        id.to_owned(),
    ))
}

fn decode(
    secret: &docker::types::Secret,
) -> error::Result<concept::versioned::Versioned<concept::secret::Secret>> {
    let kind = secret
        .spec
        .labels
        .get(LABEL_KIND)
        .and_then(|value| value.parse().ok())
        .ok_or(error::ErrorKind::SecretNotFound)?;
    Ok(concept::versioned::Versioned::new(
        secret.version.index,
        concept::secret::Secret::new(
            reference(&secret.spec.name, &secret.id)?,
            kind,
            secret.spec.labels.get(LABEL_DESCRIPTION).cloned(),
            secret.created_at,
        ),
    ))
}

pub(crate) async fn create(
    docker: &super::docker::Docker,
    name: &concept::name::Name,
    kind: concept::secret::Kind,
    description: Option<String>,
    data: &[u8],
) -> error::Result<()> {
    if data.len() > docker::MAX_SECRET_DATA_SIZE {
        return Err(error::ErrorKind::PayloadTooLarge.into());
    }
    let mut labels = HashMap::from([(LABEL_KIND.to_owned(), kind.as_ref().to_owned())]);
    if let Some(description) = description.filter(|value| !value.is_empty()) {
        labels.insert(LABEL_DESCRIPTION.to_owned(), description);
    }
    match docker
        .secret_create(&resource_name(name), labels, data)
        .await
    {
        Ok(_) => Ok(()),
        Err(error) if error.is_conflict() => Err(error::ErrorKind::SecretExists.into()),
        Err(error) => Err(error),
    }
}

pub(crate) async fn load(
    docker: &super::docker::Docker,
    name: &concept::name::Name,
) -> error::Result<Option<concept::versioned::Versioned<concept::secret::Secret>>> {
    docker
        .secret_inspect(&resource_name(name))
        .await?
        .as_ref()
        .map(decode)
        .transpose()
}

pub(crate) async fn list(
    docker: &super::docker::Docker,
    kind: Option<concept::secret::Kind>,
) -> error::Result<Vec<concept::secret::Secret>> {
    let mut secrets = Vec::new();
    for secret in docker.secret_list(PREFIX).await? {
        match decode(&secret) {
            Ok(secret) if kind.is_none_or(|kind| kind == secret.get().kind()) => {
                secrets.push(secret.into_inner());
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(error = ?error, "skipping unreadable user secret metadata");
            }
        }
    }
    secrets.sort_by(|a, b| {
        a.reference()
            .name()
            .as_str()
            .cmp(b.reference().name().as_str())
    });
    Ok(secrets)
}

pub(crate) async fn delete(
    docker: &super::docker::Docker,
    name: &concept::name::Name,
) -> error::Result<()> {
    match docker.secret_delete(&resource_name(name)).await {
        Err(error) if matches!(error.kind(), error::ErrorKind::Docker(source) if source.is_in_use()) => {
            Err(error::ErrorKind::SecretInUseRuntime(Box::new(error)).into())
        }
        result => result,
    }
}
