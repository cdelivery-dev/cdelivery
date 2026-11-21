use cipher::MasterKey;
use docker::types::{ContainerSpec, ServiceMode, TaskState};
use jiff::Timestamp;
use uuid::Uuid;

use crate::error::{ErrorKind, Result};
use crate::{concept, config};

pub(super) const LABEL_CHANGE: &str = "dev.cdelivery.change_id";
const LABEL_PREVIEW: &str = "dev.cdelivery.preview";

pub(crate) fn clone_name(id: Uuid, app: &concept::name::Name) -> String {
    Uuid::new_v5(&id, app.as_str().as_bytes())
        .simple()
        .to_string()
}

pub(crate) fn name(id: Uuid, app: &concept::name::Name) -> String {
    format!("cdelivery-preview-{}", clone_name(id, app))
}

pub(crate) fn check_name(id: Uuid, app: &concept::name::Name) -> String {
    format!("cdelivery-check-{}", clone_name(id, app))
}

pub(crate) async fn service_id(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
    check: bool,
) -> Result<Option<String>> {
    let kind = if check {
        super::unit::Kind::GuardianCheck
    } else {
        super::unit::Kind::GuardianApp
    };
    for service in docker.service_list(&kind.for_app(app)).await? {
        if super::unit::decode(&service.spec)?.0 == *app {
            return Ok(Some(service.id));
        }
    }
    Ok(None)
}

fn rebind(
    container: &mut ContainerSpec,
    change: &concept::change::Change,
    app: &concept::app::App,
    connections: &[(String, String)],
) {
    for (env, binding) in app.dependency_env() {
        match binding.dependency() {
            concept::dependency::Reference::App(dependency) => {
                if matches!(change.target(), concept::change::Target::App { name, .. } if name == dependency)
                {
                    let prefix = format!("{env}=");
                    container.env.retain(|value| !value.starts_with(&prefix));
                    container
                        .env
                        .push(format!("{env}={}", name(change.id(), dependency)));
                }
            }
            concept::dependency::Reference::PostgreSQL(dependency) => {
                if let Some((_, id)) = connections
                    .iter()
                    .find(|(source, _)| source == dependency.id())
                {
                    let path = format!("/run/cdelivery/env/{env}@{}", binding.field());
                    for secret in &mut container.secrets {
                        if secret.file.name == path {
                            secret.secret_id.clone_from(id);
                            secret.secret_name =
                                change.resource_name("connection", dependency.name().as_str());
                        }
                    }
                }
            }
        }
    }
}

pub(crate) async fn create(
    docker: &super::super::docker::Docker,
    key: &MasterKey,
    change: &concept::change::Change,
    app: &concept::app::App,
    connections: &[(String, String)],
) -> Result<()> {
    let clone = clone_name(change.id(), app.name());
    if let Some(mounts) = app.mounts() {
        super::super::volume::request::<()>(
            docker,
            key,
            mounts.node_id().clone(),
            cdelivery_wire::Command::PreparePreview {
                name: clone.clone(),
                names: mounts
                    .mounts()
                    .iter()
                    .map(|mount| mount.volume().clone())
                    .collect(),
            },
        )
        .await?;
    }
    let mut spec = super::build_spec(key, app)?;
    spec.name = name(change.id(), app.name());
    spec.labels = super::unit::labels(app.name(), super::unit::Kind::GuardianApp);
    spec.labels
        .insert(LABEL_CHANGE.to_owned(), change.id().to_string());
    spec.labels.insert(LABEL_PREVIEW.to_owned(), clone.clone());
    spec.labels.insert(
        super::super::group::LABEL_GROUP.to_owned(),
        app.group().to_string(),
    );
    spec.task_template.container_spec.labels = spec.labels.clone();
    for mount in &mut spec.task_template.container_spec.mounts {
        if matches!(mount.kind, docker::types::MountType::Volume)
            && let Some(volume) = mount.source.strip_prefix(cdelivery_wire::VOLUME_PREFIX)
        {
            let dataset = format!("cdelivery/guardian/{clone}/{volume}");
            mount.source = cdelivery_wire::guardian_volume(&clone, volume);
            if let Some(options) = &mut mount.volume_options {
                options
                    .driver_config
                    .options
                    .insert("device".to_owned(), dataset);
            }
        }
    }
    rebind(
        &mut spec.task_template.container_spec,
        change,
        app,
        connections,
    );
    spec.mode = ServiceMode::Replicated { replicas: 1 };
    spec.task_template.force_update = 0;
    super::runner::one_shot(&mut spec);
    docker
        .service_create(&spec, Some(&super::registry_auth(app)))
        .await?;
    Ok(())
}

pub(crate) async fn wait_ready(
    docker: &super::super::docker::Docker,
    id: Uuid,
    app: &concept::name::Name,
) -> Result<()> {
    let name = name(id, app);
    let deadline = config::expiry(Timestamp::now(), config::GUARDIAN_DEPLOY_TIMEOUT);
    loop {
        let service = docker
            .service_inspect(&name)
            .await?
            .ok_or(ErrorKind::AppNotFound)?;
        let tasks = docker.task_list(&name).await?;
        if let Some(task) = super::runner::newest(&service, &tasks) {
            if task.desired_state == TaskState::Running && task.status.state == TaskState::Running {
                return Ok(());
            }
            if let concept::run::State::Failed { error, .. } = super::runner::task_status(task) {
                return Err(ErrorKind::ChangeExecution(error).into());
            }
        }
        if Timestamp::now() >= deadline {
            return Err(
                ErrorKind::ChangeExecution(format!("preview {app} did not become ready")).into(),
            );
        }
        tokio::time::sleep(config::CHANGE_POLL_INTERVAL.unsigned_abs()).await;
    }
}

pub(crate) async fn create_check(
    docker: &super::super::docker::Docker,
    change: &concept::change::Change,
    app: &concept::app::App,
    health: &concept::xhealth::Spec,
    connections: &[(String, String)],
) -> Result<()> {
    health.validate_app(app)?;
    let (command, env, configs) = super::xhealth::method_runtime(
        app,
        health,
        &name(change.id(), app.name()),
        health.internal_port(app)?,
        false,
    );
    let mut labels = super::unit::labels(app.name(), super::unit::Kind::GuardianCheck);
    labels.insert(LABEL_CHANGE.to_owned(), change.id().to_string());
    labels.insert(
        LABEL_PREVIEW.to_owned(),
        clone_name(change.id(), app.name()),
    );
    let mut spec = super::runner::Spec {
        name: check_name(change.id(), app.name()),
        labels,
        image: super::xhealth::image_for(app, health.method()),
        network: super::super::group::resource_name(app.group()),
        command,
        env,
        configs,
        log_driver: super::encode_log_driver(app.log_driver()),
        replicas: 1,
    }
    .build(app);
    spec.task_template.container_spec.labels = spec.labels.clone();
    rebind(
        &mut spec.task_template.container_spec,
        change,
        app,
        connections,
    );
    docker
        .service_create(&spec, super::xhealth::auth(app, health).as_ref())
        .await?;
    Ok(())
}

pub(crate) async fn check(
    docker: &super::super::docker::Docker,
    id: Uuid,
    app: &concept::name::Name,
    deadline: Timestamp,
) -> Result<()> {
    let name = check_name(id, app);
    loop {
        let service = docker
            .service_inspect(&name)
            .await?
            .ok_or(ErrorKind::AppNotFound)?;
        let tasks = docker.task_list(&name).await?;
        match super::runner::round_status(&service, &tasks, Timestamp::now()) {
            concept::run::State::Passed { .. } => return Ok(()),
            concept::run::State::Failed { error, .. } => {
                return Err(ErrorKind::ChangeExecution(format!("{app}: {error}")).into());
            }
            concept::run::State::Settled => {
                return Err(
                    ErrorKind::ChangeExecution(format!("{app}: check produced no result")).into(),
                );
            }
            concept::run::State::Pending | concept::run::State::Running { .. } => {}
        }
        if Timestamp::now() >= deadline {
            return Err(ErrorKind::ChangeExecution(format!("{app}: check timed out")).into());
        }
        tokio::time::sleep(config::CHANGE_POLL_INTERVAL.unsigned_abs()).await;
    }
}

pub(crate) async fn stop_and_delete(
    docker: &super::super::docker::Docker,
    name: &str,
) -> Result<()> {
    let Some(service) = docker.service_inspect(name).await? else {
        return Ok(());
    };
    let id = service.id.clone();
    let version = service.version.index;
    super::runner::stop(docker, service, version).await?;
    let deadline = config::expiry(Timestamp::now(), config::GUARDIAN_DEPLOY_TIMEOUT);
    loop {
        let running = docker.task_list(&id).await?.iter().any(|task| {
            matches!(
                task.status.state,
                TaskState::New
                    | TaskState::Allocated
                    | TaskState::Pending
                    | TaskState::Assigned
                    | TaskState::Accepted
                    | TaskState::Preparing
                    | TaskState::Ready
                    | TaskState::Starting
                    | TaskState::Running
            )
        });
        if !running {
            break;
        }
        if Timestamp::now() >= deadline {
            return Err(ErrorKind::ChangeExecution(format!("{name}: tasks did not stop")).into());
        }
        tokio::time::sleep(config::CHANGE_POLL_INTERVAL.unsigned_abs()).await;
    }
    docker.service_delete(name).await?;
    Ok(())
}

pub(crate) async fn delete(
    docker: &super::super::docker::Docker,
    key: &MasterKey,
    id: Uuid,
    app: &concept::app::App,
) -> Result<()> {
    stop_and_delete(docker, &check_name(id, app.name())).await?;
    stop_and_delete(docker, &name(id, app.name())).await?;
    if let Some(mounts) = app.mounts() {
        super::super::volume::request::<()>(
            docker,
            key,
            mounts.node_id().clone(),
            cdelivery_wire::Command::DeletePreview {
                name: clone_name(id, app.name()),
            },
        )
        .await?;
    }
    Ok(())
}

pub(crate) fn identity(
    spec: &docker::types::ServiceSpec,
    app: &concept::name::Name,
    check: bool,
) -> Result<String> {
    let id = spec
        .labels
        .get(LABEL_CHANGE)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| super::bad_label(&spec.name, LABEL_CHANGE))?;
    Ok(if check {
        check_name(id, app)
    } else {
        name(id, app)
    })
}
