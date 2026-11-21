pub(crate) mod apikey;
pub(crate) mod password;
pub(crate) mod session;
pub(crate) mod totp;

use std::collections::HashMap;

use cipher::{Domain, MasterKey};
use docker::types::Secret;
use uuid::Uuid;

use crate::concept;
use crate::error::Result;

const PREFIX: &str = "cdelivery-user-";
const LABEL_EMAIL: &str = "dev.cdelivery.email";
const LABEL_DATA: &str = "dev.cdelivery.user.data";

fn secret_name(id: &Uuid) -> String {
    format!("{PREFIX}{id}")
}

fn email_index(key: &MasterKey, email: &str) -> String {
    key.blind_index(
        Domain::UserEmailIndex,
        email.trim().to_lowercase().as_bytes(),
    )
}

fn im_label(im: concept::message::Im) -> String {
    format!("dev.cdelivery.im.{}", im.key())
}

fn im_index(key: &MasterKey, im: concept::message::Im, id: &str) -> String {
    key.blind_index(Domain::UserImIndex, format!("{}:{id}", im.key()).as_bytes())
}

fn encode(key: &MasterKey, user: &concept::user::User) -> Result<HashMap<String, String>> {
    let mut labels = HashMap::from([
        (LABEL_EMAIL.to_owned(), email_index(key, user.email())),
        (
            LABEL_DATA.to_owned(),
            key.cipher_for(Domain::User).seal(user)?,
        ),
    ]);
    for (im, id) in user.im() {
        labels.insert(im_label(*im), im_index(key, *im, id));
    }
    Ok(labels)
}

fn decode(key: &MasterKey, labels: &HashMap<String, String>) -> Result<concept::user::User> {
    Ok(key
        .cipher_for(Domain::User)
        .open(labels.get(LABEL_DATA).map_or("", String::as_str))?)
}

fn from_secret(
    key: &MasterKey,
    secret: &Secret,
) -> Result<concept::versioned::Versioned<concept::user::User>> {
    Ok(concept::versioned::Versioned::new(
        secret.version.index,
        decode(key, &secret.spec.labels)?,
    ))
}

pub(crate) async fn load(
    docker: &super::docker::Docker,
    key: &MasterKey,
    id: &Uuid,
) -> Result<Option<concept::versioned::Versioned<concept::user::User>>> {
    match docker.secret_inspect(&secret_name(id)).await? {
        Some(secret) => Ok(Some(from_secret(key, &secret)?)),
        None => Ok(None),
    }
}

pub(crate) async fn create(
    docker: &super::docker::Docker,
    key: &MasterKey,
    user: &concept::user::User,
) -> Result<()> {
    docker
        .secret_create(
            &secret_name(&user.id()),
            encode(key, user)?,
            super::docker::PLACEHOLDER,
        )
        .await?;
    Ok(())
}

pub(crate) async fn save(
    docker: &super::docker::Docker,
    key: &MasterKey,
    user: &concept::versioned::Versioned<concept::user::User>,
) -> Result<()> {
    if !user.changed() {
        return Ok(());
    }
    docker
        .secret_update_labels(
            &secret_name(&user.get().id()),
            user.version(),
            encode(key, user.get())?,
        )
        .await
}

pub(crate) async fn find_by_email(
    docker: &super::docker::Docker,
    key: &MasterKey,
    email: &str,
) -> Result<Option<concept::versioned::Versioned<concept::user::User>>> {
    docker
        .secret_find(LABEL_EMAIL, &email_index(key, email))
        .await?
        .map(|secret| from_secret(key, &secret))
        .transpose()
}

pub(crate) async fn find_by_im(
    docker: &super::docker::Docker,
    key: &MasterKey,
    im: concept::message::Im,
    id: &str,
) -> Result<Option<concept::versioned::Versioned<concept::user::User>>> {
    docker
        .secret_find(&im_label(im), &im_index(key, im, id))
        .await?
        .map(|secret| from_secret(key, &secret))
        .transpose()
}

pub(crate) async fn list(
    docker: &super::docker::Docker,
    key: &MasterKey,
) -> Result<Vec<concept::user::User>> {
    let mut users = Vec::new();
    for secret in docker.secret_list(PREFIX).await? {
        match decode(key, &secret.spec.labels) {
            Ok(user) => users.push(user),
            Err(e) => {
                tracing::warn!(error = ?e, "user record {} skipped", secret.spec.name);
            }
        }
    }
    users.sort_by(|a, b| {
        a.first_name()
            .cmp(b.first_name())
            .then_with(|| a.last_name().cmp(b.last_name()))
            .then_with(|| a.id().cmp(&b.id()))
    });
    Ok(users)
}

pub(crate) async fn delete(docker: &super::docker::Docker, id: &Uuid) -> Result<()> {
    docker.secret_delete(&secret_name(id)).await
}

pub(crate) async fn purge(docker: &super::docker::Docker) -> Result<()> {
    for secret in docker.secret_list(PREFIX).await? {
        docker.secret_delete(&secret.spec.name).await?;
    }
    Ok(())
}
