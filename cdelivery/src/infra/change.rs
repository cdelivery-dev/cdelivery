use std::collections::HashMap;

use cipher::{Domain, MasterKey};
use uuid::Uuid;

use crate::{concept, error};

const PREFIX: &str = "cdelivery-change-";
const LABEL_STATE: &str = "dev.cdelivery.change";
const LABEL_ACTIVE: &str = "dev.cdelivery.change.active";

pub(crate) fn resource_name(id: Uuid) -> String {
    format!("{PREFIX}{}", id.simple())
}

fn labels(
    key: &MasterKey,
    change: &concept::change::Change,
) -> error::Result<HashMap<String, String>> {
    Ok(HashMap::from([
        (
            LABEL_STATE.to_owned(),
            key.cipher_for(Domain::Change).seal(change)?,
        ),
        (LABEL_ACTIVE.to_owned(), change.active().to_string()),
        (
            "dev.cdelivery.change.target".to_owned(),
            change.target().name().to_string(),
        ),
    ]))
}

fn decode(
    key: &MasterKey,
    secret: &docker::types::Secret,
) -> error::Result<concept::versioned::Versioned<concept::change::Change>> {
    let sealed = secret
        .spec
        .labels
        .get(LABEL_STATE)
        .ok_or(error::ErrorKind::ChangeNotFound)?;
    let change: concept::change::Change = key.cipher_for(Domain::Change).open(sealed)?;
    if secret.spec.name != resource_name(change.id()) {
        return Err(error::ErrorKind::ChangeNotFound.into());
    }
    Ok(concept::versioned::Versioned::new(
        secret.version.index,
        change,
    ))
}

pub(crate) async fn create(
    docker: &super::docker::Docker,
    key: &MasterKey,
    change: &concept::change::Change,
) -> error::Result<()> {
    delete_completed(docker, key, &change.target().reference()).await?;
    let id = change.id().to_string();
    let data = key.scoped_key(Domain::Postgres, id.as_bytes());
    docker
        .secret_create(
            &resource_name(change.id()),
            labels(key, change)?,
            data.as_ref(),
        )
        .await?;
    Ok(())
}

pub(crate) async fn load(
    docker: &super::docker::Docker,
    key: &MasterKey,
    id: Uuid,
) -> error::Result<Option<concept::versioned::Versioned<concept::change::Change>>> {
    docker
        .secret_inspect(&resource_name(id))
        .await?
        .as_ref()
        .map(|secret| decode(key, secret))
        .transpose()
}

pub(crate) async fn save(
    docker: &super::docker::Docker,
    key: &MasterKey,
    change: &concept::versioned::Versioned<concept::change::Change>,
) -> error::Result<()> {
    if !change.changed() {
        return Ok(());
    }
    docker
        .secret_update_labels(
            &resource_name(change.get().id()),
            change.version(),
            labels(key, change.get())?,
        )
        .await
}

pub(crate) async fn list(
    docker: &super::docker::Docker,
    key: &MasterKey,
    active_only: bool,
) -> error::Result<Vec<concept::change::Change>> {
    let mut filter = docker::types::Filter::new().name(PREFIX);
    if active_only {
        filter = filter.label(format!("{LABEL_ACTIVE}=true"));
    }
    let mut changes = Vec::new();
    for secret in docker.secret_list_filtered(&filter).await? {
        if !secret.spec.name.starts_with(PREFIX) {
            continue;
        }
        changes.push(decode(key, &secret)?.into_inner());
    }
    changes.sort_by_key(|change| std::cmp::Reverse(change.created_at()));
    Ok(changes)
}

pub(crate) async fn delete_completed(
    docker: &super::docker::Docker,
    key: &MasterKey,
    target: &concept::dependency::Reference,
) -> error::Result<()> {
    let name = match target {
        concept::dependency::Reference::App(name) => name,
        concept::dependency::Reference::PostgreSQL(reference) => reference.name(),
    };
    let filter = docker::types::Filter::new()
        .name(PREFIX)
        .label(format!("dev.cdelivery.change.target={name}"))
        .label(format!("{LABEL_ACTIVE}=false"));
    for secret in docker.secret_list_filtered(&filter).await? {
        let change = decode(key, &secret)?.into_inner();
        if !change.active() && change.target().reference() == *target {
            docker.secret_delete(&resource_name(change.id())).await?;
        }
    }
    Ok(())
}
