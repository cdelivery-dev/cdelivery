use cipher::MasterKey;
use jiff::Timestamp;

use crate::concept;
use crate::config;
use crate::error::{ErrorKind, Result};
use crate::infra;

pub(crate) async fn session(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    token: &str,
    origin: concept::actor::Origin,
    now: Timestamp,
) -> Result<(
    concept::permission::Access,
    concept::user::Preferences,
    Option<String>,
)> {
    let (user_id, expires_at, fingerprint) = infra::user::session::verify(key, token, now)?;
    let user = super::found(
        infra::user::load(docker, key, &user_id),
        ErrorKind::TokenInvalid,
    )
    .await?;
    if !infra::user::session::matches_password(&fingerprint, user.password_hash()) {
        return Err(ErrorKind::TokenInvalid.into());
    }
    let renewed_expires_at = config::expiry(now, config::SESSION_TTL);
    let renewal = (renewed_expires_at.duration_since(expires_at)
        > config::SESSION_RENEW_MIN_EXTENSION)
        .then(|| {
            infra::user::session::issue(key, user.id(), renewed_expires_at, user.password_hash())
        })
        .transpose()?;
    let access = user_access(&user, None, origin);
    let preferences = user.preferences().clone();
    Ok((access, preferences, renewal))
}

pub(crate) async fn api_key(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    token: &str,
    origin: concept::actor::Origin,
    now: Timestamp,
) -> Result<concept::permission::Access> {
    let (user_id, index) =
        infra::user::apikey::parse(key, token).ok_or(ErrorKind::ApiKeyInvalid)?;
    let user = super::found(
        infra::user::load(docker, key, &user_id),
        ErrorKind::ApiKeyInvalid,
    )
    .await?;
    let api_key = user
        .api_key_by_index(&index, now)
        .ok_or(ErrorKind::ApiKeyInvalid)?;
    Ok(user_access(&user, Some(api_key), origin))
}

pub(crate) fn user_access(
    user: &concept::user::User,
    api_key: Option<&concept::user::ApiKey>,
    origin: concept::actor::Origin,
) -> concept::permission::Access {
    let role = user.effective_role(api_key);
    let actor = concept::actor::Actor::user(
        user.id(),
        user.name(),
        api_key.map(concept::user::ApiKey::id),
        origin,
    );
    concept::permission::Access::new(actor, role)
}
