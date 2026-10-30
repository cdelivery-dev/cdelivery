use std::collections::HashMap;

use docker::types::{
    ContainerSpec, EndpointMode, EndpointSpec, LogDriver, NanoDuration, NetworkAttachment,
    RegistryAuth, RestartPolicy, RestartPolicyCondition, Service, ServiceConfig, ServiceMode,
    ServiceSpec, Task, TaskState, TaskTemplate, UpdateConfig, UpdateFailureAction, UpdateOrder,
};
use jiff::Timestamp;

use crate::concept;
use crate::config;
use crate::error::{ErrorKind, Result};

pub(super) struct Spec {
    pub(super) name: String,
    pub(super) labels: HashMap<String, String>,
    pub(super) image: String,
    pub(super) network: String,
    pub(super) command: Vec<String>,
    pub(super) env: Vec<String>,
    pub(super) configs: Vec<ServiceConfig>,
    pub(super) log_driver: Option<LogDriver>,
    pub(super) replicas: u64,
}

impl Spec {
    pub(super) fn build(self) -> ServiceSpec {
        let mut spec = ServiceSpec {
            extra: serde_json::Map::new(),
            name: self.name,
            labels: self.labels,
            task_template: TaskTemplate {
                container_spec: ContainerSpec {
                    image: self.image,
                    command: self.command,
                    env: self.env,
                    configs: self.configs,
                    ..ContainerSpec::default()
                },
                resources: None,
                restart_policy: None,
                placement: None,
                networks: Some(vec![NetworkAttachment {
                    target: self.network,
                    aliases: None,
                    driver_opts: None,
                }]),
                log_driver: self.log_driver,
                force_update: 0,
                extra: serde_json::Map::new(),
            },
            mode: ServiceMode::Replicated {
                replicas: self.replicas,
            },
            update_config: None,
            rollback_config: None,
            endpoint_spec: None,
        };
        one_shot(&mut spec);
        spec
    }
}

pub(super) fn one_shot(spec: &mut ServiceSpec) {
    spec.task_template.restart_policy = Some(RestartPolicy {
        condition: RestartPolicyCondition::None,
        delay: None,
        max_attempts: None,
        window: None,
    });
    spec.update_config = Some(UpdateConfig {
        parallelism: Some(1),
        delay: None,
        failure_action: Some(UpdateFailureAction::Continue),
        monitor: Some(NanoDuration(5_000_000_000)),
        max_failure_ratio: Some(0.0),
        order: Some(UpdateOrder::StopFirst),
    });
    spec.rollback_config = None;
    spec.endpoint_spec = Some(EndpointSpec {
        mode: EndpointMode::Vip,
        ports: None,
    });
}

pub(super) async fn rerun(
    docker: &super::super::docker::Docker,
    current: &Service,
    service_version: u64,
    auth: Option<&RegistryAuth>,
    mut spec: ServiceSpec,
) -> Result<()> {
    spec.task_template.force_update = current.spec.task_template.force_update.wrapping_add(1);
    docker
        .service_update(&current.spec.name, service_version, &spec, auth)
        .await
}

pub(super) async fn stop(
    docker: &super::super::docker::Docker,
    current: Service,
    service_version: u64,
) -> Result<bool> {
    if current.version.index != service_version {
        return Err(ErrorKind::Conflict.into());
    }
    if matches!(current.spec.mode, ServiceMode::Replicated { replicas: 0 }) {
        return Ok(false);
    }
    let mut spec = current.spec;
    spec.mode = ServiceMode::Replicated { replicas: 0 };
    docker
        .service_update(&spec.name, service_version, &spec, None)
        .await?;
    Ok(true)
}

pub(super) fn newest<'a>(service: &Service, tasks: &'a [Task]) -> Option<&'a Task> {
    let current = service.spec.task_template.force_update;
    tasks
        .iter()
        .filter(|t| t.service_id == service.id && t.spec.force_update == current)
        .max_by_key(|t| (t.created_at, t.version.index))
}

pub(super) fn task_status(task: &Task) -> concept::run::State {
    match &task.status.state {
        TaskState::New
        | TaskState::Allocated
        | TaskState::Pending
        | TaskState::Assigned
        | TaskState::Accepted
        | TaskState::Preparing
        | TaskState::Ready
        | TaskState::Starting
        | TaskState::Running => concept::run::State::Running {
            started_at: task.created_at,
        },
        TaskState::Complete if task.status.error().is_none() => concept::run::State::Passed {
            finished_at: task.status.timestamp,
        },
        TaskState::Complete
        | TaskState::Shutdown
        | TaskState::Failed
        | TaskState::Rejected
        | TaskState::Remove
        | TaskState::Orphaned
        | TaskState::Unknown(_) => concept::run::State::Failed {
            error: task.status.diagnostic(),
            finished_at: task.status.timestamp,
        },
    }
}

pub(super) fn round_status(
    service: &Service,
    tasks: &[Task],
    now: Timestamp,
) -> concept::run::State {
    match newest(service, tasks) {
        Some(task) => task_status(task),
        None if matches!(service.spec.mode, ServiceMode::Replicated { replicas: 0 }) => {
            concept::run::State::Settled
        }
        None if now.duration_since(service.updated_at) > config::TASK_SETTLE => {
            concept::run::State::Settled
        }
        None => concept::run::State::Pending,
    }
}
