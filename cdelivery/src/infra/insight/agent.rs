use docker::types::{
    BindOptions, ContainerSpec, Mount, MountPropagation, MountType, ServiceMode, ServiceSpec,
    TaskTemplate, UpdateConfig, UpdateOrder,
};

use crate::error::Result;
use crate::{concept, config};

pub(crate) async fn reconcile(
    docker: &super::super::docker::Docker,
    image: &concept::image::Reference,
    sample_interval_secs: u16,
) -> Result<()> {
    let desired = build_spec(image, sample_interval_secs);
    let Some(service) = docker
        .service_inspect(config::INSIGHT_AGENT_SERVICE_NAME)
        .await?
    else {
        match docker.service_create(&desired, None).await {
            Ok(_) => return Ok(()),
            Err(e) if e.is_conflict() => {
                tracing::warn!("insight agent service already exists; leaving it in place");
                return Ok(());
            }
            Err(e) => return Err(e),
        }
    };
    let current: concept::image::Reference =
        service.spec.task_template.container_spec.image.parse()?;
    if image != &current
        || !matches!(&service.spec.mode, ServiceMode::Global {})
        || sample_interval(&service.spec).ok() != Some(sample_interval_secs)
    {
        docker
            .service_update(
                config::INSIGHT_AGENT_SERVICE_NAME,
                service.version.index,
                &desired,
                None,
            )
            .await?;
    }
    Ok(())
}

pub(crate) async fn delete(docker: &super::super::docker::Docker) -> Result<()> {
    docker
        .service_delete(config::INSIGHT_AGENT_SERVICE_NAME)
        .await?;
    Ok(())
}

pub(crate) async fn observe(
    docker: &super::super::docker::Docker,
) -> Result<Option<(concept::image::Reference, u16)>> {
    match inspect(docker).await? {
        Some((image, interval)) => Ok(Some((image, interval?))),
        None => Ok(None),
    }
}

pub(crate) async fn observe_for_reconcile(
    docker: &super::super::docker::Docker,
) -> Result<Option<(concept::image::Reference, Option<u16>)>> {
    Ok(inspect(docker)
        .await?
        .map(|(image, interval)| (image, interval.ok())))
}

async fn inspect(
    docker: &super::super::docker::Docker,
) -> Result<Option<(concept::image::Reference, Result<u16>)>> {
    let Some(service) = docker
        .service_inspect(config::INSIGHT_AGENT_SERVICE_NAME)
        .await?
    else {
        return Ok(None);
    };
    let interval = sample_interval(&service.spec);
    let image = service.spec.task_template.container_spec.image.parse()?;
    Ok(Some((image, interval)))
}

pub(crate) async fn service_id(docker: &super::super::docker::Docker) -> Result<Option<String>> {
    Ok(docker
        .service_inspect(config::INSIGHT_AGENT_SERVICE_NAME)
        .await?
        .map(|service| service.id))
}

fn sample_interval(spec: &ServiceSpec) -> Result<u16> {
    let name = insight_wire::SAMPLE_INTERVAL_SECS_ENV;
    let range = insight_wire::metric::SAMPLE_INTERVAL_MIN_SECS
        ..=insight_wire::metric::SAMPLE_INTERVAL_MAX_SECS;
    super::env_value(spec, name)?
        .parse::<u16>()
        .ok()
        .filter(|value| range.contains(value))
        .ok_or_else(|| {
            crate::error::SpecProblem::BadEnv {
                service: spec.name.clone(),
                env: name.to_owned(),
            }
            .into()
        })
}

fn bind(source: &str, target: &str, read_only: bool, bind_options: Option<BindOptions>) -> Mount {
    Mount {
        kind: MountType::Bind,
        target: target.to_owned(),
        source: source.to_owned(),
        read_only,
        bind_options,
    }
}

fn build_spec(image: &concept::image::Reference, sample_interval_secs: u16) -> ServiceSpec {
    let mut spec = ServiceSpec {
        extra: serde_json::Map::new(),
        name: config::INSIGHT_AGENT_SERVICE_NAME.to_owned(),
        labels: std::collections::HashMap::new(),
        task_template: TaskTemplate {
            container_spec: ContainerSpec {
                image: image.to_string(),
                stop_grace_period: Some(super::STOP_GRACE_PERIOD),
                env: vec![
                    super::node_id_env(),
                    format!(
                        "{}={sample_interval_secs}",
                        insight_wire::SAMPLE_INTERVAL_SECS_ENV
                    ),
                ],
                mounts: vec![
                    bind("/var/run/docker.sock", "/var/run/docker.sock", true, None),
                    bind(
                        "/",
                        insight_wire::HOST_MOUNT_PREFIX,
                        true,
                        Some(BindOptions {
                            propagation: Some(MountPropagation::Rslave),
                            read_only_force_recursive: true,
                        }),
                    ),
                    bind("/run", "/run", false, None),
                ],
                ..ContainerSpec::default()
            },
            networks: Some(super::platform_network()),
            ..TaskTemplate::default()
        },
        mode: ServiceMode::Global {},
        update_config: Some(UpdateConfig {
            order: Some(UpdateOrder::StopFirst),
            ..UpdateConfig::default()
        }),
        rollback_config: None,
        endpoint_spec: None,
    };
    super::super::platform::monitor_updates(&mut spec);
    spec
}
