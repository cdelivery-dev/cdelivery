use cipher::MasterKey;
use jiff::Timestamp;
use uuid::Uuid;

use crate::concept;
use crate::error::{ErrorKind, Result};
use crate::infra;

static EMAIL_UNIQUENESS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(super) async fn load(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    user_id: Uuid,
) -> Result<concept::versioned::Versioned<concept::user::User>> {
    super::require(
        infra::user::load(docker, key, &user_id),
        ErrorKind::TokenInvalid,
    )
    .await
}

pub(super) fn verify_current_password(hash: &str, candidate: &str) -> Result<()> {
    if infra::user::password::verify(candidate, hash)? {
        Ok(())
    } else {
        Err(ErrorKind::CurrentPasswordInvalid.into())
    }
}

pub(crate) async fn get(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    user_id: Uuid,
) -> Result<concept::user::User> {
    Ok(load(docker, key, user_id).await?.into_inner())
}

pub(crate) async fn get_managed(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    user_id: Uuid,
) -> Result<concept::user::User> {
    super::found(
        infra::user::load(docker, key, &user_id),
        ErrorKind::UserNotFound,
    )
    .await
}

pub(crate) async fn get_by_email(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    email: &str,
) -> Result<concept::user::User> {
    super::found(
        infra::user::find_by_email(docker, key, email),
        ErrorKind::UserNotFound,
    )
    .await
}

pub(crate) async fn find_by_id(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    user_id: Uuid,
) -> Result<Option<concept::user::User>> {
    Ok(infra::user::load(docker, key, &user_id)
        .await?
        .map(concept::versioned::Versioned::into_inner))
}

pub(crate) async fn find_by_im(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    im: concept::message::Im,
    id: &str,
) -> Result<Option<concept::user::User>> {
    Ok(infra::user::find_by_im(docker, key, im, id)
        .await?
        .map(concept::versioned::Versioned::into_inner))
}

pub(crate) async fn list(
    docker: &infra::docker::Docker,
    key: &MasterKey,
) -> Result<Vec<concept::user::User>> {
    infra::user::list(docker, key).await
}

pub(crate) async fn observe_im_profile(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    user_id: Uuid,
    im: concept::message::Im,
) -> Result<(Option<String>, Option<String>)> {
    let user = get(docker, key, user_id).await?;
    let id = user.im().get(&im).ok_or(ErrorKind::ImNotBound)?;
    if !infra::channel::has_profile(im) {
        return Ok((None, None));
    }
    let settings = infra::setting::load(docker, key).await?.into_inner();
    infra::channel::observe_profile(&settings, im, id).await
}

async fn mutate<R>(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    user_id: Uuid,
    apply: impl FnOnce(&mut concept::user::User) -> R,
) -> Result<R> {
    let mut user = load(docker, key, user_id).await?;
    let out = apply(user.get_mut());
    infra::user::save(docker, key, &user).await?;
    Ok(out)
}

pub(super) fn build(
    first_name: String,
    last_name: String,
    email: &str,
    password_sha256: &str,
    role: concept::user::Role,
) -> Result<concept::user::User> {
    Ok(concept::user::User::new(
        first_name,
        last_name,
        email.to_owned(),
        infra::user::password::hash(password_sha256)?,
        role,
    ))
}

pub(super) async fn create(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    user: &concept::user::User,
) -> Result<()> {
    let _guard = EMAIL_UNIQUENESS.lock().await;
    if infra::user::find_by_email(docker, key, user.email())
        .await?
        .is_some()
    {
        return Err(ErrorKind::EmailTaken.into());
    }
    infra::user::create(docker, key, user).await
}

pub(crate) async fn issue_api_key(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    user_id: Uuid,
    name: String,
    expires_at: Option<Timestamp>,
    role: concept::user::Role,
    now: Timestamp,
) -> Result<(String, concept::user::ApiKey)> {
    let (token, index) = infra::user::apikey::mint(key, user_id);
    let record = mutate(docker, key, user_id, |user| {
        user.add_api_key(name, index, now, expires_at, role)
    })
    .await?;
    Ok((token, record))
}

pub(crate) async fn list_api_keys(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    user_id: Uuid,
) -> Result<Vec<concept::user::ApiKey>> {
    Ok(get(docker, key, user_id).await?.api_keys().clone())
}

pub(crate) async fn revoke_api_key(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    user_id: Uuid,
    id: Uuid,
) -> Result<()> {
    mutate(docker, key, user_id, |user| user.revoke_api_key(id)).await
}

#[expect(clippy::option_option)]
pub(crate) async fn update_preferences(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    user_id: Uuid,
    language: Option<Option<crate::locale::Locale>>,
    theme: Option<crate::theme::Theme>,
    alert_email: Option<bool>,
    alerts: Option<std::collections::BTreeSet<concept::message::Im>>,
) -> Result<()> {
    mutate(docker, key, user_id, |user| {
        user.update_preferences(language, theme, alert_email, alerts);
    })
    .await
}

pub(crate) async fn update_im(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    user_id: Uuid,
    im: concept::message::Im,
    id: Option<String>,
) -> Result<()> {
    mutate(docker, key, user_id, |user| user.set_im(im, id)).await
}

pub(crate) async fn update_profile(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    user_id: Uuid,
    first_name: Option<String>,
    last_name: Option<String>,
) -> Result<()> {
    mutate(docker, key, user_id, |user| {
        user.rename(first_name, last_name);
    })
    .await
}

pub(crate) async fn change_password(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    user_id: Uuid,
    current_password_sha256: String,
    password_sha256: String,
) -> Result<()> {
    let mut user = load(docker, key, user_id).await?;
    verify_current_password(user.get().password_hash(), &current_password_sha256)?;
    let hash = infra::user::password::hash(&password_sha256)?;
    user.get_mut().change_password(hash);
    infra::user::save(docker, key, &user).await
}

pub(crate) async fn create_managed(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    first_name: String,
    last_name: String,
    email: String,
    password_sha256: String,
    role: concept::user::Role,
) -> Result<concept::user::User> {
    let user = build(first_name, last_name, &email, &password_sha256, role)?;
    create(docker, key, &user).await?;
    Ok(user)
}

#[expect(clippy::too_many_arguments)]
pub(crate) async fn update(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    user_id: Uuid,
    first_name: Option<String>,
    last_name: Option<String>,
    email: Option<String>,
    role: Option<concept::user::Role>,
    password_sha256: Option<String>,
    reset_totp: bool,
) -> Result<concept::user::User> {
    let hash = password_sha256
        .map(|password| infra::user::password::hash(&password))
        .transpose()?;
    let mut user = super::require(
        infra::user::load(docker, key, &user_id),
        ErrorKind::UserNotFound,
    )
    .await?;
    let _email_guard = EMAIL_UNIQUENESS.lock().await;
    if let Some(email) = &email
        && let Some(existing) = infra::user::find_by_email(docker, key, email).await?
        && existing.get().id() != user_id
    {
        return Err(ErrorKind::EmailTaken.into());
    }
    user.get_mut().rename(first_name, last_name);
    if let Some(email) = email {
        user.get_mut().set_email(email);
    }
    if let Some(role) = role {
        user.get_mut().set_role(role);
    }
    if let Some(hash) = hash {
        user.get_mut().change_password(hash);
    }
    if reset_totp {
        user.get_mut().disable_totp();
    }
    infra::user::save(docker, key, &user).await?;
    Ok(user.into_inner())
}

pub(crate) async fn rescue(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    email: &str,
    password_sha256: &str,
) -> Result<()> {
    let hash = infra::user::password::hash(password_sha256)?;
    let mut user = super::require(
        infra::user::find_by_email(docker, key, email),
        ErrorKind::UserNotFound,
    )
    .await?;
    user.get_mut().change_password(hash);
    user.get_mut().disable_totp();
    infra::user::save(docker, key, &user).await
}

pub(crate) async fn delete(docker: &infra::docker::Docker, user_id: Uuid) -> Result<()> {
    infra::user::delete(docker, &user_id).await?;
    super::totp::forget(user_id).await;
    Ok(())
}
