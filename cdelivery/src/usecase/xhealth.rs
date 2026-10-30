use cipher::MasterKey;
use insight_wire::event::Kind;
use jiff::{SignedDuration, Timestamp};

use crate::error::{self, ErrorKind, Result};
use crate::{concept, infra};

pub(crate) enum MethodParams {
    Hurl {
        file: Option<Vec<u8>>,
        version: Option<String>,
    },
    Custom {
        command: Vec<String>,
    },
}

async fn gate(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    name: &concept::name::Name,
    guardian: bool,
) -> Result<concept::app::App> {
    let app = super::app::get(docker, key, name).await?;
    if infra::app::guardian::exists(docker, name).await? {
        return Err(ErrorKind::GuardianPending.into());
    }
    if guardian && app.mounts().is_some() {
        return Err(ErrorKind::GuardianStatefulUnsupported.into());
    }
    Ok(app)
}

async fn hurl(
    docker: &infra::docker::Docker,
    releases: &infra::release::Catalog,
    name: &concept::name::Name,
    current: Option<&concept::xhealth::Method>,
    file: Option<Vec<u8>>,
    version: Option<String>,
) -> Result<concept::xhealth::Method> {
    let current = match current {
        Some(concept::xhealth::Method::Hurl { image, config }) => Some((image, config)),
        _ => None,
    };
    let image = super::release::select(
        releases,
        concept::release::Component::Hurl,
        version.as_deref(),
        current.map(|(image, _)| image),
        "method.version",
    )?;
    let config = match file {
        Some(file) => {
            infra::app::xconfig::upload(docker, name, crate::config::HURL_FILE_PATH, &file).await?
        }
        None => error::required(current.map(|(_, config)| config.clone()), "method.file")?,
    };
    Ok(concept::xhealth::Method::Hurl { image, config })
}

#[expect(clippy::too_many_arguments)]
pub(crate) async fn create(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    releases: &infra::release::Catalog,
    name: &concept::name::Name,
    method: MethodParams,
    interval: SignedDuration,
    target: concept::xhealth::Target,
    guardian: bool,
    now: Timestamp,
) -> Result<concept::xhealth::XHealth> {
    concept::xhealth::Spec::validate_interval(interval)?;
    let app = gate(docker, key, name, guardian).await?;
    target.resolve(&app, matches!(&method, MethodParams::Hurl { .. }))?;
    let method = match method {
        MethodParams::Custom { command } => concept::xhealth::Method::Custom { command },
        MethodParams::Hurl { file, version } => {
            hurl(docker, releases, name, None, file, version).await?
        }
    };
    let spec = concept::xhealth::Spec::new(method, interval, target, guardian)?;
    infra::app::xhealth::create(docker, &app, &spec).await?;
    infra::event::emit(Kind::XHealthEnabled, name.as_str(), "");
    get(docker, name, now).await
}

#[expect(clippy::too_many_arguments)]
pub(crate) async fn update(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    releases: &infra::release::Catalog,
    name: &concept::name::Name,
    method: Option<MethodParams>,
    interval: Option<SignedDuration>,
    target: Option<concept::xhealth::Target>,
    guardian: Option<bool>,
    now: Timestamp,
) -> Result<concept::xhealth::XHealth> {
    let mut unit = infra::app::xhealth::load(docker, name)
        .await?
        .ok_or(ErrorKind::XHealthNotEnabled)?;
    let target = target.unwrap_or_else(|| unit.get().target().clone());
    let guardian = guardian.unwrap_or_else(|| unit.get().guardian());
    let interval = interval.unwrap_or_else(|| unit.get().interval());
    concept::xhealth::Spec::validate_interval(interval)?;
    let app = gate(docker, key, name, guardian).await?;
    let is_hurl = match &method {
        Some(MethodParams::Hurl { .. }) => true,
        Some(MethodParams::Custom { .. }) => false,
        None => matches!(unit.get().method(), concept::xhealth::Method::Hurl { .. }),
    };
    target.resolve(&app, is_hurl)?;
    let method = match method {
        Some(MethodParams::Custom { command }) => concept::xhealth::Method::Custom { command },
        Some(MethodParams::Hurl { file, version }) => {
            let current = unit.get().method();
            hurl(docker, releases, name, Some(current), file, version).await?
        }
        None => unit.get().method().clone(),
    };
    *unit.get_mut() = concept::xhealth::Spec::new(method, interval, target, guardian)?;
    let changed = unit.changed();
    infra::app::xhealth::save(docker, &app, &unit).await?;
    if changed {
        infra::event::emit(Kind::XHealthUpdated, name.as_str(), "");
    }
    get(docker, name, now).await
}

pub(crate) async fn get(
    docker: &infra::docker::Docker,
    name: &concept::name::Name,
    now: Timestamp,
) -> Result<concept::xhealth::XHealth> {
    match infra::app::xhealth::observe(docker, name, now).await? {
        Some((xhealth, _)) => Ok(xhealth),
        None if infra::app::service_id(docker, name).await?.is_none() => {
            Err(ErrorKind::AppNotFound.into())
        }
        None => Err(ErrorKind::XHealthNotEnabled.into()),
    }
}

pub(crate) async fn run_now(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    name: &concept::name::Name,
    now: Timestamp,
) -> Result<()> {
    let app = super::app::get(docker, key, name).await?;
    let (xhealth, version) = infra::app::xhealth::observe(docker, name, now)
        .await?
        .ok_or(ErrorKind::XHealthNotEnabled)?;
    if !xhealth.state().finished() {
        return Err(ErrorKind::XHealthRunning.into());
    }
    infra::app::xhealth::run_round(docker, &app, xhealth.spec(), version).await?;
    infra::event::emit(Kind::XHealthTriggered, name.as_str(), "");
    Ok(())
}

pub(crate) async fn disable(
    docker: &infra::docker::Docker,
    name: &concept::name::Name,
) -> Result<()> {
    if infra::app::guardian::exists(docker, name).await? {
        return Err(ErrorKind::GuardianPending.into());
    }
    if infra::app::xhealth::delete(docker, name).await? {
        infra::event::emit(Kind::XHealthDisabled, name.as_str(), "");
        infra::event::HUB.dismiss(Kind::XHealthFailed, name.as_str());
    }
    Ok(())
}

pub(crate) async fn download_file(
    docker: &infra::docker::Docker,
    name: &concept::name::Name,
) -> Result<Vec<u8>> {
    let spec = infra::app::xhealth::load(docker, name)
        .await?
        .ok_or(ErrorKind::XHealthNotEnabled)?
        .into_inner();
    let concept::xhealth::Method::Hurl { config, .. } = spec.method() else {
        return Err(ErrorKind::ConfigNotFound.into());
    };
    infra::app::xconfig::download(docker, config.config_name())
        .await?
        .ok_or_else(|| ErrorKind::ConfigNotFound.into())
}

pub(crate) async fn sweep(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    now: Timestamp,
) -> (usize, Vec<(String, String)>) {
    super::sweep_units(
        infra::app::xhealth::observe_all(docker, now).await,
        "xhealth",
        Kind::XHealthFailed,
        |(name, _, _)| name.to_string(),
        |(_, xhealth, _)| Some(xhealth.state().clone()),
        |(name, xhealth, service_version)| async move {
            tick(docker, key, &name, xhealth, service_version, now).await
        },
    )
    .await
}

async fn tick(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    name: &concept::name::Name,
    xhealth: concept::xhealth::XHealth,
    service_version: u64,
    now: Timestamp,
) -> Result<bool> {
    if xhealth.state().expired(xhealth.spec().interval(), now) {
        if infra::app::xhealth::stop(docker, name, service_version).await? {
            tracing::warn!("xhealth {name} timed out; stopping current check");
        }
        return Ok(false);
    }
    if !xhealth.due(now) {
        return Ok(false);
    }
    let Some(app) = infra::app::load(docker, key, name).await? else {
        tracing::warn!("deleting orphan xhealth runner {name}: app is gone");
        infra::app::xhealth::delete(docker, name).await?;
        return Ok(false);
    };
    infra::app::xhealth::run_round(docker, app.get(), xhealth.spec(), service_version).await?;
    Ok(true)
}
