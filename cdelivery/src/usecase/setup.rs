use cipher::MasterKey;
use tokio::sync::Mutex;

use crate::concept;
use crate::error::{ErrorKind, Result};
use crate::infra;

pub(crate) async fn state(docker: &infra::docker::Docker) -> Result<concept::setup::State> {
    let Some((_, attached)) = infra::keyring::observe(docker).await? else {
        return Ok(concept::setup::State::NeedsSetup);
    };
    Ok(if attached {
        concept::setup::State::Provisioning
    } else {
        concept::setup::State::NeedsResume
    })
}

pub(crate) async fn initialize(
    docker: &infra::docker::Docker,
    lock: &Mutex<()>,
    first_name: String,
    last_name: String,
    email: &str,
    password_sha256: &str,
) -> Result<concept::setup::Initialization> {
    let _guard = lock.lock().await;
    if let Some((secret_id, attached)) = infra::keyring::observe(docker).await? {
        if attached {
            return Err(ErrorKind::AlreadyInitialized.into());
        }
        infra::keyring::attach(docker, &secret_id).await?;
        return Ok(concept::setup::Initialization::Resumed);
    }
    let admin = super::user::build(
        first_name,
        last_name,
        email,
        password_sha256,
        concept::user::Role::Admin,
    )?;
    infra::user::purge(docker).await?;

    let key = MasterKey::generate()?;
    super::user::create(docker, &key, &admin).await?;
    let secret_id = infra::keyring::create(docker, &key).await?;
    infra::keyring::attach(docker, &secret_id).await?;
    Ok(concept::setup::Initialization::Initialized)
}

pub(crate) async fn resume(docker: &infra::docker::Docker, lock: &Mutex<()>) -> Result<()> {
    let _guard = lock.lock().await;
    infra::instance::verify(docker).await?;
    let secret_id =
        super::require(infra::keyring::find_id(docker), ErrorKind::NotInitialized).await?;
    infra::keyring::attach(docker, &secret_id).await?;
    Ok(())
}
