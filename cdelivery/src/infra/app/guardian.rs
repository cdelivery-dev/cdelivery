use cipher::{Domain, MasterKey};
use docker::types::{Service, ServiceMode, ServiceSpec, TaskState};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::concept;
use crate::error::{Error, ErrorKind, Result};

const LABEL_STATE: &str = "dev.cdelivery.guardian_state";
const LABEL_CHECK_SHADOW_VERSION: &str = "dev.cdelivery.check_shadow_version";
const SHADOW_PREFIX: &str = "cdelivery-shadow-";
const CHECK_PREFIX: &str = "cdelivery-verify-";

#[derive(Serialize, Deserialize)]
struct State {
    changes: concept::app::Changes,
    initiator: concept::app::Operator,
    verdict: Option<concept::guardian::Verdict>,
}

pub(super) fn shadow_name(app: &concept::name::Name) -> String {
    format!("{SHADOW_PREFIX}{app}")
}

pub(super) fn check_name(app: &concept::name::Name) -> String {
    format!("{CHECK_PREFIX}{app}")
}

fn shadow_spec(
    app: &concept::name::Name,
    candidate_spec: &ServiceSpec,
    sealed_state: String,
) -> ServiceSpec {
    let mut spec = candidate_spec.clone();
    spec.name = shadow_name(app);
    spec.labels = super::unit::labels(app, super::unit::Kind::GuardianApp);
    if let Some(scaling) = candidate_spec.labels.get(super::LABEL_SCALING) {
        spec.labels
            .insert(super::LABEL_SCALING.to_owned(), scaling.clone());
    }
    spec.labels.insert(LABEL_STATE.to_owned(), sealed_state);
    spec.mode = ServiceMode::Replicated { replicas: 1 };
    spec.task_template.force_update = 0;
    super::runner::one_shot(&mut spec);
    spec
}

pub(crate) async fn create(
    docker: &super::super::docker::Docker,
    key: &MasterKey,
    candidate: &concept::app::App,
    changes: &concept::app::Changes,
    initiator: concept::app::Operator,
) -> Result<()> {
    let candidate_spec = super::build_spec(key, candidate)?;
    let sealed_state = seal_state(key, changes, initiator, None)?;
    let spec = shadow_spec(candidate.name(), &candidate_spec, sealed_state);
    docker
        .service_create_unique(
            &spec,
            super::registry_auth(candidate).as_ref(),
            ErrorKind::GuardianPending,
        )
        .await
}

pub(crate) async fn exists(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
) -> Result<bool> {
    Ok(find(docker, app).await?.is_some())
}

async fn find(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
) -> Result<Option<Service>> {
    docker
        .service_find(
            super::unit::Kind::GuardianApp.for_app(app),
            &shadow_name(app),
        )
        .await
}

pub(crate) async fn app_service_id(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
) -> Result<Option<String>> {
    Ok(find(docker, app).await?.map(|service| service.id))
}

pub(crate) async fn load(
    docker: &super::super::docker::Docker,
    key: &MasterKey,
    app: &concept::name::Name,
) -> Result<Option<concept::versioned::Versioned<concept::guardian::Guardian>>> {
    super::versioned(find(docker, app).await?, |s| decode(key, s))
}

fn decode(key: &MasterKey, service: &Service) -> Result<concept::guardian::Guardian> {
    let name = super::unit::owner(&service.spec, super::unit::Kind::GuardianApp)?;
    let state: State = key
        .cipher_for(Domain::Guardian)
        .open(super::label(service, LABEL_STATE)?)?;
    Ok(concept::guardian::Guardian::hydrate(
        name,
        state.changes,
        state.initiator,
        state.verdict,
        service.created_at,
    ))
}

fn seal_state(
    key: &MasterKey,
    changes: &concept::app::Changes,
    initiator: concept::app::Operator,
    verdict: Option<&concept::guardian::Verdict>,
) -> Result<String> {
    Ok(key.cipher_for(Domain::Guardian).seal(&State {
        changes: changes.clone(),
        initiator,
        verdict: verdict.cloned(),
    })?)
}

fn seal_guardian(key: &MasterKey, guardian: &concept::guardian::Guardian) -> Result<String> {
    seal_state(
        key,
        guardian.changes(),
        guardian.initiator(),
        guardian.verdict().as_ref(),
    )
}

pub(crate) async fn save(
    docker: &super::super::docker::Docker,
    key: &MasterKey,
    guardian: &concept::versioned::Versioned<concept::guardian::Guardian>,
) -> Result<()> {
    if !guardian.changed() {
        return Ok(());
    }
    let current = find(docker, guardian.get().name())
        .await?
        .ok_or(ErrorKind::Conflict)?;
    let sealed = seal_guardian(key, guardian.get())?;
    docker
        .service_update_labels(&current, guardian.version(), None, |labels| {
            labels.insert(LABEL_STATE.to_owned(), sealed);
        })
        .await
}

pub(crate) async fn redeploy(
    docker: &super::super::docker::Docker,
    key: &MasterKey,
    guardian: &concept::versioned::Versioned<concept::guardian::Guardian>,
    candidate: &concept::app::App,
) -> Result<()> {
    let name = shadow_name(guardian.get().name());
    let current = find(docker, guardian.get().name())
        .await?
        .ok_or(ErrorKind::GuardianNotFound)?;
    let candidate_spec = super::build_spec(key, candidate)?;
    let sealed_state = seal_guardian(key, guardian.get())?;
    let mut spec = shadow_spec(guardian.get().name(), &candidate_spec, sealed_state);
    spec.task_template.force_update = current.spec.task_template.force_update.wrapping_add(1);
    docker
        .service_update(
            &name,
            guardian.version(),
            &spec,
            super::registry_auth(candidate).as_ref(),
        )
        .await
}

pub(crate) async fn delete(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
) -> Result<()> {
    docker.service_delete(&check_name(app)).await?;
    docker.service_delete(&shadow_name(app)).await?;
    Ok(())
}

async fn find_check(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
) -> Result<Option<Service>> {
    docker
        .service_find(
            super::unit::Kind::GuardianCheck.for_app(app),
            &check_name(app),
        )
        .await
}

pub(crate) async fn check_service_id(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
) -> Result<Option<String>> {
    Ok(find_check(docker, app).await?.map(|service| service.id))
}

fn check_spec(
    app: &concept::name::Name,
    candidate: &concept::app::App,
    health: &concept::xhealth::Spec,
    image: String,
    shadow_version: u64,
) -> Result<ServiceSpec> {
    let (command, env, configs) =
        super::xhealth::method_runtime(candidate, health, &shadow_name(app))?;
    let mut labels = super::unit::labels(app, super::unit::Kind::GuardianCheck);
    labels.insert(
        LABEL_CHECK_SHADOW_VERSION.to_owned(),
        shadow_version.to_string(),
    );
    Ok(super::runner::Spec {
        name: check_name(app),
        labels,
        image,
        network: super::super::group::resource_name(candidate.group()),
        command,
        env,
        configs,
        log_driver: super::encode_log_driver(candidate.log_driver()),
        replicas: 1,
    }
    .build())
}

pub(crate) async fn run_check(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
    candidate: &concept::app::App,
    health: &concept::xhealth::Spec,
    shadow_version: u64,
    check_version: Option<u64>,
) -> Result<()> {
    let image = super::xhealth::image_for(candidate, health.method());
    let auth = super::xhealth::auth(candidate, health);
    if let Some(check_version) = check_version {
        let Some(check) = find_check(docker, app).await? else {
            return Ok(());
        };
        let spec = check_spec(app, candidate, health, image, shadow_version)?;
        return super::runner::rerun(docker, &check, check_version, auth.as_ref(), spec).await;
    }
    let spec = check_spec(app, candidate, health, image, shadow_version)?;
    match docker.service_create(&spec, auth.as_ref()).await {
        Ok(_) => Ok(()),
        Err(e) if e.is_conflict() => {
            tracing::warn!(error = ?e, "check service {} already exists", spec.name);
            Ok(())
        }
        Err(e) => Err(e),
    }
}

pub(crate) async fn delete_check(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
) -> Result<()> {
    docker.service_delete(&check_name(app)).await?;
    Ok(())
}

pub(crate) async fn stop(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
    shadow_version: u64,
    check_version: Option<u64>,
) -> Result<()> {
    let Some(shadow) = find(docker, app).await? else {
        return Ok(());
    };
    super::runner::stop(docker, shadow, shadow_version).await?;
    if let Some(version) = check_version
        && let Some(check) = find_check(docker, app).await?
    {
        super::runner::stop(docker, check, version).await?;
    }
    Ok(())
}

pub(crate) async fn observe(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
    now: Timestamp,
) -> Result<Option<concept::guardian::Snapshot>> {
    let Some(shadow) = find(docker, app).await? else {
        return Ok(None);
    };
    let tasks = docker.task_list(&shadow.spec.name).await?;
    let task = super::runner::newest(&shadow, &tasks);
    let ready = task.is_some_and(|task| task.status.state == TaskState::Running);
    let latest_task = task.map(super::runner::task_status);
    let check = match find_check(docker, app).await? {
        None => None,
        Some(service) => {
            let tasks = docker.task_list(&service.spec.name).await?;
            let for_shadow = service
                .spec
                .labels
                .get(LABEL_CHECK_SHADOW_VERSION)
                .and_then(|raw| raw.parse().ok())
                .unwrap_or_else(|| {
                    tracing::warn!(
                        "check service {} carries no readable shadow version, treating check as stale",
                        service.spec.name
                    );
                    0
                });
            Some(concept::guardian::Check::new(
                service.version.index,
                for_shadow,
                super::runner::round_status(&service, &tasks, now),
            ))
        }
    };
    Ok(Some(concept::guardian::Snapshot::new(
        concept::guardian::Shadow::new(shadow.version.index, shadow.updated_at, ready, latest_task),
        check,
    )))
}

pub(crate) async fn owners(
    docker: &super::super::docker::Docker,
) -> Result<(Vec<concept::name::Name>, Vec<(String, Error)>)> {
    let mut names = std::collections::BTreeSet::new();
    let mut errors = Vec::new();
    for kind in [
        super::unit::Kind::GuardianApp,
        super::unit::Kind::GuardianCheck,
    ] {
        for service in docker.service_list(&kind.filter()).await? {
            match super::unit::owner(&service.spec, kind) {
                Ok(owner) => {
                    names.insert(owner);
                }
                Err(e) => {
                    tracing::warn!(error = ?e, "guardian service {} carries an invalid identity", service.spec.name);
                    errors.push((service.spec.name, e));
                }
            }
        }
    }
    Ok((names.into_iter().collect(), errors))
}
