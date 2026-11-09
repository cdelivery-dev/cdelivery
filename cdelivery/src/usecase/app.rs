use cipher::MasterKey;
use insight_wire::event::Kind;
use jiff::Timestamp;

use crate::concept;
use crate::error::{ErrorKind, Result};
use crate::infra;

#[derive(Debug, macros_derive::Ctor)]
pub(crate) struct UploadParams {
    target: String,
    data: Vec<u8>,
}

#[derive(Debug)]
pub(crate) enum XConfigParams {
    Upload(UploadParams),
    InUse(concept::app::XConfigRef),
    Secret {
        target: String,
        secret: concept::secret::Reference,
    },
}

async fn files(
    docker: &infra::docker::Docker,
    app: &concept::app::App,
    actions: Vec<XConfigParams>,
) -> Result<Vec<concept::app::XConfig>> {
    for action in &actions {
        match action {
            XConfigParams::Secret { secret, .. } => {
                super::secret::validate_reference(docker, secret, concept::secret::Kind::File)
                    .await?;
            }
            XConfigParams::InUse(existing)
                if !app
                    .xconfigs()
                    .iter()
                    .any(|file| file.config() == Some(existing)) =>
            {
                return Err(ErrorKind::ConfigNotFound.into());
            }
            XConfigParams::Upload(_) if app.mounts().is_some() => {
                return Err(ErrorKind::XConfigMountsConflict.into());
            }
            _ => {}
        }
    }
    let mut files = Vec::with_capacity(actions.len());
    for action in actions {
        files.push(match action {
            XConfigParams::Upload(upload) => concept::app::XConfig::Config(
                infra::app::xconfig::upload(docker, app.name(), &upload.target, &upload.data)
                    .await?,
            ),
            XConfigParams::InUse(existing) => concept::app::XConfig::Config(existing),
            XConfigParams::Secret { target, secret } => {
                concept::app::XConfig::Secret { target, secret }
            }
        });
    }
    Ok(files)
}

async fn prepare_tini(
    docker: &infra::docker::Docker,
    app: &mut concept::app::App,
    deployment_changed: bool,
) -> Result<()> {
    if app.text_secrets().next().is_none() {
        app.set_tini(Vec::new());
    } else if deployment_changed {
        app.set_tini(infra::app::tini::ensure(docker).await?);
    }
    Ok(())
}

async fn validate_text_secrets(
    docker: &infra::docker::Docker,
    app: &concept::app::App,
) -> Result<()> {
    for (_, secret) in app.text_secrets() {
        super::secret::validate_reference(docker, secret, concept::secret::Kind::Text).await?;
    }
    Ok(())
}

async fn ensure_group(docker: &infra::docker::Docker, group: &concept::name::Name) -> Result<()> {
    if infra::group::exists(docker, group).await? {
        Ok(())
    } else {
        Err(ErrorKind::GroupMissing(group.to_string()).into())
    }
}

pub(crate) async fn default_log_driver(
    docker: &infra::docker::Docker,
) -> Result<concept::app::LogDriver> {
    Ok(if infra::insight::collection_ready(docker).await? {
        concept::app::LogDriver::Insight
    } else {
        concept::app::LogDriver::FollowHost
    })
}

pub(crate) async fn create(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    mut desired: concept::app::Desired,
    uploads: Vec<XConfigParams>,
    operator: concept::app::Operator,
    now: Timestamp,
) -> Result<concept::app::App> {
    if matches!(desired.log_driver, concept::app::LogDriver::Insight) || desired.scaling.is_some() {
        super::insight::require_collection(docker).await?;
    }
    ensure_group(docker, &desired.group).await?;
    if let Some(mounts) = &mut desired.mounts {
        *mounts = concept::app::NodeMounts::new(
            mounts.node_id().clone(),
            super::volume::resolve_mounts(docker, key, mounts.node_id(), mounts.mounts()).await?,
        );
    }
    let mut app = concept::app::App::new(desired, operator, now)?;
    infra::app::check_image(docker, &app).await?;
    validate_text_secrets(docker, &app).await?;
    if !uploads.is_empty() {
        let refs = files(docker, &app, uploads).await?;
        let mut changes = concept::app::Changes::default();
        changes.replace_xconfigs(refs);
        changes.apply(&mut app)?;
    }
    prepare_tini(docker, &mut app, true).await?;
    super::pingress::check_routes(docker, key, app.domains(), None).await?;
    infra::app::create(docker, key, &app).await?;
    let app = get(docker, key, app.name()).await?;
    infra::event::emit(
        Kind::AppCreated,
        app.name().as_str(),
        &format!("group {}, image {}", app.group(), app.image()),
    );
    Ok(app)
}

pub(crate) async fn list(
    docker: &infra::docker::Docker,
) -> Result<Vec<(concept::name::Name, concept::name::Name)>> {
    infra::app::list(docker).await
}

pub(crate) async fn log_sources(
    docker: &infra::docker::Docker,
    name: &concept::name::Name,
) -> Result<Vec<concept::app::LogSource>> {
    super::require(infra::app::logs::list(docker, name), ErrorKind::AppNotFound).await
}

pub(crate) async fn get(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    name: &concept::name::Name,
) -> Result<concept::app::App> {
    super::found(infra::app::load(docker, key, name), ErrorKind::AppNotFound).await
}

pub(crate) async fn status(
    docker: &infra::docker::Docker,
    name: &concept::name::Name,
) -> Result<(concept::overview::Instances, Vec<String>)> {
    super::require(
        infra::app::runtime::observe(docker, name),
        ErrorKind::AppNotFound,
    )
    .await
}

pub(crate) async fn update(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    name: &concept::name::Name,
    mut changes: concept::app::Changes,
    xconfigs: Option<Vec<XConfigParams>>,
    operator: concept::app::Operator,
) -> Result<concept::app::UpdateOutcome> {
    let _guard = super::guardian::acquire(name)?;
    let mut app =
        super::require(infra::app::load(docker, key, name), ErrorKind::AppNotFound).await?;
    let name = app.get().name().clone();
    let log_driver = changes
        .log_driver()
        .as_ref()
        .unwrap_or_else(|| app.get().log_driver());
    if matches!(log_driver, concept::app::LogDriver::Insight) {
        super::insight::require_collection(docker).await?;
    }
    if infra::app::guardian::exists(docker, &name).await? {
        return Err(ErrorKind::GuardianPending.into());
    }
    if let Some(group) = changes.group() {
        ensure_group(docker, group).await?;
    }
    let domains_touched = changes.domains().is_some();
    let env_touched = changes.container_options().is_some();
    if let Some(mounts) = changes.mounts() {
        let node = app
            .get()
            .mounts()
            .as_ref()
            .ok_or(ErrorKind::MountsRequireStateful)?
            .node_id();
        changes.replace_mounts(super::volume::resolve_mounts(docker, key, node, mounts).await?);
    }
    changes.apply(app.get_mut())?;
    if env_touched {
        validate_text_secrets(docker, app.get()).await?;
    }
    if app.baseline().image() != app.get().image()
        || app.baseline().image_auth() != app.get().image_auth()
    {
        infra::app::check_image(docker, app.get()).await?;
    }
    if let Some(actions) = xconfigs {
        let refs = files(docker, app.get(), actions).await?;
        let mut changes = concept::app::Changes::default();
        changes.replace_xconfigs(refs);
        changes.apply(app.get_mut())?;
    }
    if app.baseline().scaling().is_none() && app.get().scaling().is_some() {
        super::insight::require_collection(docker).await?;
    }
    if domains_touched {
        super::pingress::check_routes(docker, key, app.get().domains(), Some(name.as_str()))
            .await?;
    }
    let runtime_changed = app.baseline().diff(app.get()).runtime_changed();
    prepare_tini(docker, app.get_mut(), runtime_changed).await?;
    let delta = app.baseline().diff(app.get());
    let note = match delta.image() {
        Some(to) => format!("image {} -> {to}", app.baseline().image()),
        None => String::new(),
    };
    let guardian_relevant = delta.guardian_relevant();
    let health_target_changed = delta.health_target_changed();
    let health = if health_target_changed || guardian_relevant {
        infra::app::xhealth::load(docker, &name)
            .await?
            .map(concept::versioned::Versioned::into_inner)
    } else {
        None
    };
    if let Some(spec) = &health {
        spec.validate_app(app.get())?;
    }
    if guardian_relevant && health.is_some_and(|spec| spec.guardian()) {
        super::volume::require_guardian(docker, app.get()).await?;
        infra::app::guardian::create(docker, key, app.get(), &delta, operator).await?;
        infra::event::emit(Kind::GuardianSubmitted, name.as_str(), &note);
        return Ok(concept::app::UpdateOutcome::Guarded);
    }
    let applied = app.changed();
    if applied {
        app.get_mut().record_operator(operator);
    }
    infra::app::save(docker, key, &app).await?;
    let reloaded = get(docker, key, &name).await?;
    if applied {
        infra::event::emit(Kind::AppUpdated, name.as_str(), &note);
    }
    Ok(concept::app::UpdateOutcome::Applied(Box::new(reloaded)))
}

pub(crate) async fn delete(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    name: &concept::name::Name,
) -> Result<()> {
    let _guard = super::guardian::acquire(name)?;
    infra::app::guardian::delete(docker, key, name).await?;
    infra::app::xhealth::delete(docker, name).await?;
    infra::app::xjob::delete_all(docker, name).await?;
    if infra::app::delete(docker, name).await? {
        infra::event::emit(Kind::AppDeleted, name.as_str(), "");
    }
    Ok(())
}

pub(crate) async fn rollback(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    name: &concept::name::Name,
    now: Timestamp,
) -> Result<concept::app::App> {
    let app = super::require(infra::app::load(docker, key, name), ErrorKind::AppNotFound).await?;
    match app.get().rollback() {
        Some(concept::app::Rollback::Available(_)) => {}
        Some(
            concept::app::Rollback::InProgress(_)
            | concept::app::Rollback::Paused(_)
            | concept::app::Rollback::Completed(_),
        ) => {
            return Err(ErrorKind::AlreadyRolledBack.into());
        }
        None => {
            return Err(ErrorKind::NothingToRollBack.into());
        }
    }
    super::guardian::cancel(docker, key, name, now).await?;
    let _guard = super::guardian::acquire(name)?;
    if infra::app::guardian::exists(docker, name).await? {
        return Err(ErrorKind::GuardianPending.into());
    }
    infra::app::rollback(docker, name, app.version()).await?;
    let app = get(docker, key, name).await?;
    infra::event::emit(
        Kind::AppRolledBack,
        name.as_str(),
        &format!("image {}", app.image()),
    );
    Ok(app)
}

pub(crate) async fn restart(
    docker: &infra::docker::Docker,
    name: &concept::name::Name,
) -> Result<()> {
    let _guard = super::guardian::acquire(name)?;
    infra::app::restart(docker, name).await?;
    infra::event::emit(Kind::AppRestarted, name.as_str(), "");
    Ok(())
}

pub(crate) async fn download_xconfig(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    name: &concept::name::Name,
    config_name: &str,
) -> Result<Vec<u8>> {
    let app = super::found(infra::app::load(docker, key, name), ErrorKind::AppNotFound).await?;
    if !app
        .xconfigs()
        .iter()
        .filter_map(concept::app::XConfig::config)
        .any(|x| x.config_name() == config_name)
    {
        return Err(ErrorKind::ConfigNotFound.into());
    }
    infra::app::xconfig::download(docker, config_name)
        .await?
        .ok_or_else(|| ErrorKind::ConfigNotFound.into())
}

pub(crate) async fn access(
    docker: &infra::docker::Docker,
    api: &infra::insight::query::Api,
    name: &concept::name::Name,
    range: (Timestamp, Timestamp),
    skip: u32,
    limit: u16,
    mut filter: pingress_wire::access::Filter,
) -> Result<Vec<pingress_wire::access::Access>> {
    super::insight::require_server(docker).await?;
    let service_id = infra::app::service_id(docker, name)
        .await?
        .ok_or(ErrorKind::AppNotFound)?;
    filter.service_id = Some(service_id);
    api.access(range, skip, limit, &filter).await
}
