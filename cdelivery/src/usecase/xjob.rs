use cipher::MasterKey;
use insight_wire::event::Kind;
use jiff::{SignedDuration, Timestamp};

use crate::concept;
use crate::config;
use crate::error::{ErrorKind, Result};
use crate::infra;

fn subject(app: &concept::name::Name, job: &concept::name::Name) -> String {
    format!("{app}/{job}")
}

pub(crate) async fn create(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    name: &concept::name::Name,
    job: &concept::name::Name,
    spec: concept::xjob::Spec,
    now: Timestamp,
) -> Result<concept::xjob::XJob> {
    let app = super::app::get(docker, key, name).await?;
    if infra::app::xjob::count(docker, name).await? >= config::XJOB_MAX_PER_APP {
        return Err(ErrorKind::XJobLimit {
            limit: config::XJOB_MAX_PER_APP,
        }
        .into());
    }
    infra::app::xjob::create(docker, &app, job, &spec).await?;
    infra::event::emit(Kind::XJobCreated, &subject(name, job), "");
    get(docker, name, job, now).await
}

#[expect(clippy::too_many_arguments)]
pub(crate) async fn update(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    name: &concept::name::Name,
    job: &concept::name::Name,
    interval: Option<SignedDuration>,
    timeout: Option<SignedDuration>,
    command: Option<Vec<String>>,
    now: Timestamp,
) -> Result<concept::xjob::XJob> {
    let app = super::app::get(docker, key, name).await?;
    let mut unit = infra::app::xjob::load(docker, name, job)
        .await?
        .ok_or(ErrorKind::XJobNotFound)?;
    unit.get_mut().update(interval, timeout, command);
    infra::app::xjob::save(docker, &app, job, &unit).await?;
    if unit.changed() {
        infra::event::emit(Kind::XJobUpdated, &subject(name, job), "");
    }
    get(docker, name, job, now).await
}

pub(crate) async fn get(
    docker: &infra::docker::Docker,
    name: &concept::name::Name,
    job: &concept::name::Name,
    now: Timestamp,
) -> Result<concept::xjob::XJob> {
    match infra::app::xjob::observe(docker, name, job, now).await? {
        Some((xjob, _)) => Ok(xjob),
        None if infra::app::service_id(docker, name).await?.is_none() => {
            Err(ErrorKind::AppNotFound.into())
        }
        None => Err(ErrorKind::XJobNotFound.into()),
    }
}

pub(crate) async fn list(
    docker: &infra::docker::Docker,
    name: &concept::name::Name,
    now: Timestamp,
) -> Result<Vec<concept::xjob::XJob>> {
    super::require(infra::app::service_id(docker, name), ErrorKind::AppNotFound).await?;
    infra::app::xjob::list(docker, name, now).await
}

pub(crate) async fn delete(
    docker: &infra::docker::Docker,
    name: &concept::name::Name,
    job: &concept::name::Name,
) -> Result<()> {
    if infra::app::xjob::delete(docker, name, job).await? {
        infra::event::emit(Kind::XJobDeleted, &subject(name, job), "");
        infra::event::HUB.dismiss(Kind::XJobFailed, &subject(name, job));
    }
    Ok(())
}

pub(crate) async fn run_now(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    name: &concept::name::Name,
    job: &concept::name::Name,
    now: Timestamp,
) -> Result<concept::xjob::XJob> {
    let app = super::app::get(docker, key, name).await?;
    let (xjob, version) = infra::app::xjob::observe(docker, name, job, now)
        .await?
        .ok_or(ErrorKind::XJobNotFound)?;
    if xjob.is_active() {
        return Err(ErrorKind::XJobRunning.into());
    }
    infra::app::xjob::trigger(docker, &app, job, xjob.spec(), version).await?;
    get(docker, name, job, now).await
}

pub(crate) async fn sweep(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    now: Timestamp,
) -> (usize, Vec<(String, String)>) {
    super::sweep_units(
        infra::app::xjob::observe_all(docker, now).await,
        "xjob",
        Kind::XJobFailed,
        |(app, xjob, _)| subject(app, xjob.name()),
        |(_, xjob, _)| Some(xjob.state().clone()),
        |(app, xjob, version)| async move { tick(docker, key, &app, xjob, version, now).await },
    )
    .await
}

async fn tick(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    name: &concept::name::Name,
    xjob: concept::xjob::XJob,
    version: u64,
    now: Timestamp,
) -> Result<bool> {
    if xjob.state().expired(xjob.spec().timeout(), now) {
        if infra::app::xjob::stop(docker, name, xjob.name(), version).await? {
            infra::event::HUB.raise(
                Kind::XJobFailed,
                &subject(name, xjob.name()),
                &format!(
                    "Execution exceeded {}; requested stop",
                    xjob.spec().timeout()
                ),
                Some(SignedDuration::ZERO),
            );
        }
        return Ok(false);
    }
    if !xjob.due(now) {
        return Ok(false);
    }
    let Some(app) = infra::app::load(docker, key, name).await? else {
        tracing::warn!(
            "deleting orphan xjob runner {name}/{}: app is gone",
            xjob.name()
        );
        infra::app::xjob::delete(docker, name, xjob.name()).await?;
        return Ok(false);
    };
    infra::app::xjob::trigger(docker, app.get(), xjob.name(), xjob.spec(), version).await?;
    Ok(true)
}
