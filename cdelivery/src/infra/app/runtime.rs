use docker::types::{Service, ServiceMode, ServiceUpdateState, Task, TaskState};
use jiff::Timestamp;

use crate::concept;
use crate::error::{Result, SpecProblem};

pub(crate) async fn observe(
    docker: &super::super::docker::Docker,
    name: &concept::name::Name,
) -> Result<Option<(concept::overview::Instances, Vec<String>)>> {
    let Some(service) = docker
        .service_find(super::unit::Kind::App.for_app(name), name.as_str())
        .await?
    else {
        return Ok(None);
    };
    super::unit::owner(&service.spec, super::unit::Kind::App)?;
    let ServiceMode::Replicated { replicas } = service.spec.mode else {
        return Err(SpecProblem::NotReplicated(service.spec.name).into());
    };
    let tasks = docker.task_list(name.as_str()).await?;
    let tasks: Vec<_> = tasks
        .iter()
        .filter(|task| task.service_id == service.id)
        .collect();
    let mut diagnostics: Vec<_> = tasks
        .iter()
        .filter(|task| {
            demanded(task)
                && (task.status.error().is_some()
                    || matches!(task.desired_state, TaskState::Unknown(_))
                    || matches!(
                        task.status.state,
                        TaskState::Failed | TaskState::Rejected | TaskState::Unknown(_)
                    ))
        })
        .map(|task| (task.status.timestamp, task_diagnostic(task)))
        .chain(update_diagnostic(&service))
        .collect();
    diagnostics.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    Ok(Some((
        instances(&tasks, replicas),
        diagnostics.into_iter().map(|(_, detail)| detail).collect(),
    )))
}

pub(crate) fn instances(tasks: &[&Task], desired: u64) -> concept::overview::Instances {
    let running = tasks
        .iter()
        .filter(|task| {
            task.desired_state == TaskState::Running && task.status.state == TaskState::Running
        })
        .map(|_| 1)
        .sum();
    concept::overview::Instances::new(running, desired)
}

pub(crate) fn demanded(task: &Task) -> bool {
    !matches!(
        task.desired_state,
        TaskState::Complete
            | TaskState::Shutdown
            | TaskState::Failed
            | TaskState::Rejected
            | TaskState::Remove
            | TaskState::Orphaned
    )
}

pub(crate) fn task_diagnostic(task: &Task) -> String {
    format!(
        "Task desired {}, observed {} at {}: {}",
        task.desired_state,
        task.status.state,
        task.status.timestamp,
        task.status.diagnostic()
    )
}

pub(crate) fn update_diagnostic(service: &Service) -> Option<(Timestamp, String)> {
    service.update_status.as_ref().and_then(|update| {
        (!matches!(
            update.state,
            ServiceUpdateState::Updating
                | ServiceUpdateState::Completed
                | ServiceUpdateState::RollbackStarted
                | ServiceUpdateState::RollbackCompleted
        ))
        .then(|| {
            (
                service.updated_at,
                format!(
                    "Service update {}: {}",
                    update.state,
                    update.message.as_deref().unwrap_or_default()
                ),
            )
        })
    })
}
