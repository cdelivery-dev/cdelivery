use std::sync::LazyLock;

use cipher::MasterKey;
use jiff::Timestamp;
use totp_rs::Secret;
use uuid::Uuid;

use crate::config;
use crate::error::{ErrorKind, Result};
use crate::infra;

static ATTEMPTS: LazyLock<scc::HashMap<Uuid, Attempt>> = LazyLock::new(scc::HashMap::new);

#[derive(Default)]
struct Attempt {
    last_step: Option<u64>,
    failures: u8,
    retry_at: Option<Timestamp>,
}

pub(crate) async fn prepare(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    user_id: Uuid,
) -> Result<(String, String)> {
    let user = super::user::load(docker, key, user_id).await?;
    if user.get().totp_enabled() {
        return Err(ErrorKind::TotpAlreadyEnabled.into());
    }
    infra::user::totp::enrollment(user.get().email())
}

pub(crate) async fn enable(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    user_id: Uuid,
    current_password_sha256: String,
    secret: Secret,
    code: String,
    now: Timestamp,
) -> Result<()> {
    let mut user = super::user::load(docker, key, user_id).await?;
    if user.get().totp_enabled() {
        return Err(ErrorKind::TotpAlreadyEnabled.into());
    }
    super::user::verify_current_password(user.get().password_hash(), &current_password_sha256)?;
    let step = infra::user::totp::verify(&secret, &code, now)?.ok_or(ErrorKind::TotpCodeInvalid)?;
    user.get_mut().enable_totp(secret);
    infra::user::save(docker, key, &user).await?;
    ATTEMPTS
        .upsert_async(
            user_id,
            Attempt {
                last_step: Some(step),
                ..Attempt::default()
            },
        )
        .await;
    Ok(())
}

pub(crate) async fn disable(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    user_id: Uuid,
    current_password_sha256: String,
    code: String,
    now: Timestamp,
) -> Result<()> {
    let mut user = super::user::load(docker, key, user_id).await?;
    super::user::verify_current_password(user.get().password_hash(), &current_password_sha256)?;
    let Some(secret) = user.get().totp_secret() else {
        return Ok(());
    };
    if !authenticate(user_id, secret, &code, now).await? {
        return Err(ErrorKind::TotpCodeInvalid.into());
    }
    user.get_mut().disable_totp();
    infra::user::save(docker, key, &user).await
}

pub(super) async fn authenticate(
    user_id: Uuid,
    secret: &Secret,
    code: &str,
    now: Timestamp,
) -> Result<bool> {
    let mut attempt = ATTEMPTS.entry_async(user_id).await.or_default();
    if attempt.retry_at.is_some_and(|retry_at| retry_at > now) {
        return Ok(false);
    }
    attempt.retry_at = None;
    let matched = infra::user::totp::verify(secret, code, now)?;
    if let Some(step) = matched
        && attempt.last_step.is_none_or(|last_step| step > last_step)
    {
        attempt.last_step = Some(step);
        attempt.failures = 0;
        return Ok(true);
    }
    reject(&mut attempt, now);
    Ok(false)
}

pub(super) async fn forget(user_id: Uuid) {
    ATTEMPTS.remove_async(&user_id).await;
}

fn reject(attempt: &mut Attempt, now: Timestamp) {
    if attempt.failures < 4 {
        attempt.failures += 1;
    } else {
        attempt.retry_at = Some(config::expiry(now, config::TOTP_COOLDOWN));
    }
}
