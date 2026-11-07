use std::collections::{BTreeMap, HashMap, HashSet};

use cipher::MasterKey;
use docker::types::{ServiceMode, ServiceUpdateState, TaskState};

use crate::concept;
use crate::error::{ErrorKind, Result};

pub(crate) async fn observe(
    docker: &super::super::docker::Docker,
    key: &MasterKey,
) -> Result<BTreeMap<String, Result<concept::scaling::Observation>>> {
    let services = docker
        .service_list(&super::unit::Kind::App.filter().label(super::LABEL_SCALING))
        .await?;
    if services.is_empty() {
        return Ok(BTreeMap::new());
    }
    let guardian_filter = super::unit::Kind::GuardianApp.filter();
    let (tasks, guardians) = tokio::try_join!(
        docker.task_list_all(),
        docker.service_list(&guardian_filter),
    )?;
    let guardians: HashSet<_> = guardians
        .into_iter()
        .filter_map(|service| {
            super::unit::owner(&service.spec, super::unit::Kind::GuardianApp)
                .ok()
                .map(|name| name.to_string())
        })
        .collect();
    let mut by_service: HashMap<_, Vec<_>> = HashMap::new();
    for task in tasks.into_iter().filter(|task| {
        task.desired_state == TaskState::Running
            && !matches!(
                task.status.state,
                TaskState::Complete
                    | TaskState::Shutdown
                    | TaskState::Failed
                    | TaskState::Rejected
                    | TaskState::Remove
                    | TaskState::Orphaned
            )
    }) {
        by_service
            .entry(task.service_id.clone())
            .or_default()
            .push(task);
    }
    Ok(services
        .into_iter()
        .map(|service| {
            let observation =
                super::decode_spec(key, &service.spec, service.created_at).map(|app| {
                    let blocked = service.update_status.as_ref().is_some_and(|status| {
                        !matches!(
                            status.state,
                            ServiceUpdateState::Completed | ServiceUpdateState::RollbackCompleted
                        )
                    }) || guardians.contains(app.name().as_str());
                    let tasks = by_service.remove(&service.id).unwrap_or_default();
                    let mut occupied = HashMap::new();
                    for task in tasks.iter().filter(|task| !task.node_id.is_empty()) {
                        *occupied.entry(task.node_id.clone()).or_default() += 1;
                    }
                    let containers = tasks
                        .into_iter()
                        .filter(|task| task.status.state == TaskState::Running)
                        .map(|task| {
                            let id = task.status.container_status?.container_id;
                            (!id.is_empty() && !task.node_id.is_empty())
                                .then_some((id, task.node_id))
                        })
                        .collect();
                    concept::scaling::Observation::new(
                        service.id,
                        app,
                        service.version.index,
                        containers,
                        occupied,
                        blocked,
                    )
                });
            (service.spec.name, observation)
        })
        .collect())
}

pub(crate) async fn scale(
    docker: &super::super::docker::Docker,
    name: &concept::name::Name,
    version: u64,
    replicas: u64,
) -> Result<()> {
    let mut service = docker
        .service_inspect(name.as_str())
        .await?
        .ok_or(ErrorKind::AppNotFound)?;
    super::unit::owner(&service.spec, super::unit::Kind::App)?;
    if service.version.index != version {
        return Err(ErrorKind::Conflict.into());
    }
    service.spec.mode = ServiceMode::Replicated { replicas };
    docker
        .service_update(name.as_str(), version, &service.spec, None)
        .await
}
