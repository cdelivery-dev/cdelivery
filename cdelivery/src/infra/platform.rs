use crate::concept;
use crate::error::Result;

pub(super) fn monitor_updates(spec: &mut docker::types::ServiceSpec) {
    let monitor = Some(docker::types::NanoDuration::from_secs(
        crate::config::SERVICE_UPDATE_MONITOR
            .as_secs()
            .unsigned_abs(),
    ));
    spec.update_config
        .get_or_insert(docker::types::UpdateConfig {
            parallelism: Some(1),
            delay: None,
            failure_action: Some(docker::types::UpdateFailureAction::Pause),
            monitor: None,
            max_failure_ratio: Some(0.0),
            order: Some(docker::types::UpdateOrder::StopFirst),
        })
        .monitor
        .clone_from(&monitor);
    spec.rollback_config
        .get_or_insert(docker::types::RollbackConfig {
            parallelism: Some(1),
            delay: None,
            failure_action: Some(docker::types::RollbackFailureAction::Pause),
            monitor: None,
            max_failure_ratio: Some(0.0),
            order: Some(docker::types::UpdateOrder::StopFirst),
        })
        .monitor = monitor;
}

pub(crate) async fn observe_instances(
    docker: &super::docker::Docker,
    service: &str,
) -> Result<Option<concept::overview::Instances>> {
    Ok(docker
        .service_status(service)
        .await?
        .map(|(running, desired)| concept::overview::Instances::new(running, desired)))
}
