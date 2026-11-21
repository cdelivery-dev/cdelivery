use cipher::MasterKey;
use jiff::Timestamp;

use crate::concept;
use crate::config::{SESSION_TTL, expiry};
use crate::error::{ErrorKind, Result};
use crate::infra;

pub(crate) async fn login(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    email: &str,
    password_sha256: &str,
    totp_code: Option<&str>,
    now: Timestamp,
) -> Result<String> {
    let user = verify_credentials(docker, key, email, password_sha256).await?;
    if let Some(secret) = user.totp_secret()
        && !super::totp::authenticate(user.id(), secret, totp_code.unwrap_or(""), now).await?
    {
        return Err(ErrorKind::CredentialsInvalid.into());
    }
    infra::user::session::issue(
        key,
        user.id(),
        expiry(now, SESSION_TTL),
        user.password_hash(),
    )
}

async fn verify_credentials(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    email: &str,
    password_sha256: &str,
) -> Result<concept::user::User> {
    match infra::user::find_by_email(docker, key, email)
        .await?
        .map(concept::versioned::Versioned::into_inner)
    {
        Some(user) if infra::user::password::verify(password_sha256, user.password_hash())? => {
            Ok(user)
        }
        Some(_) => Err(ErrorKind::CredentialsInvalid.into()),
        None => {
            infra::user::password::hash(password_sha256)?;
            Err(ErrorKind::CredentialsInvalid.into())
        }
    }
}
