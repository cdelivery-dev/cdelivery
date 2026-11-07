use docker::types::{NanoDuration, NetworkDriver, Service, ServiceMode, ServiceSpec};

use crate::concept;
use crate::config;
use crate::error::{ErrorKind, InstanceProblem, Result};

pub(crate) async fn verify(docker: &super::docker::Docker) -> Result<()> {
    verified_service(docker).await?;
    Ok(())
}

pub(super) async fn verified_service(docker: &super::docker::Docker) -> Result<Service> {
    let service = require_service(docker).await?;
    let replicas = replicated(&service)?;
    if replicas != 1 {
        return Err(InstanceProblem::NotSingleton { replicas }.into());
    }
    require_attached(docker, &service.spec).await?;
    Ok(service)
}

pub(crate) async fn converge(docker: &super::docker::Docker) -> Result<()> {
    let service = require_service(docker).await?;
    let replicas = replicated(&service)?;
    if replicas != 1 {
        let mut spec = service.spec.clone();
        spec.mode = ServiceMode::Replicated { replicas: 1 };
        match docker
            .service_update(config::SERVICE_NAME, service.version.index, &spec, None)
            .await
        {
            Ok(()) => tracing::warn!("platform service scaled back from {replicas} to 1 replica"),
            Err(e) if e.is_conflict() => {
                tracing::warn!("platform service scale-back conflicted with another update");
            }
            Err(e) => return Err(e),
        }
    }
    if let Err(e) = require_attached(docker, &service.spec).await {
        tracing::warn!(error = ?e, "platform network check failed");
    }
    match super::insight::collection_ready(docker).await {
        Ok(on) => {
            if let Err(e) = set_log_driver(docker, on).await {
                tracing::warn!(error = ?e, "platform log driver reconcile failed");
            }
        }
        Err(e) => {
            tracing::warn!(error = ?e, "platform log driver reconcile cannot probe insight collection");
        }
    }
    Ok(())
}

pub(crate) async fn require_network(docker: &super::docker::Docker) -> Result<()> {
    let service = require_service(docker).await?;
    require_attached(docker, &service.spec).await
}

pub(crate) async fn configuration(
    docker: &super::docker::Docker,
) -> Result<(concept::image::Reference, Option<concept::domain::Rule>)> {
    let service = require_service(docker).await?;
    let image = service.spec.task_template.container_spec.image.parse()?;
    let domain = if service.spec.labels.contains_key(super::app::LABEL_DOMAINS) {
        super::app::domains(&service)?.into_iter().next()
    } else {
        None
    };
    Ok((image, domain))
}

pub(crate) async fn service_id(docker: &super::docker::Docker) -> Result<String> {
    Ok(require_service(docker).await?.id)
}

#[expect(clippy::option_option)]
pub(crate) async fn configure(
    docker: &super::docker::Docker,
    image: Option<&concept::image::Reference>,
    domain: Option<Option<&concept::domain::Rule>>,
) -> Result<(Option<String>, bool)> {
    let value = domain.map(|domain| domain.map(|rule| super::label::json(&[rule])));
    let mut updated_image = None;
    let mut domain_changed = false;
    apply_spec(docker, |spec| {
        if let Some(image) = image
            && let Some(replacement) =
                image.replacement_for(&spec.task_template.container_spec.image)?
        {
            spec.task_template
                .container_spec
                .image
                .clone_from(&replacement);
            updated_image = Some(replacement);
            prepare_replacement(spec);
        }
        domain_changed = match value {
            Some(Some(value)) => {
                spec.labels
                    .insert(super::app::LABEL_DOMAINS.to_owned(), value.clone())
                    != Some(value)
            }
            Some(None) => spec.labels.remove(super::app::LABEL_DOMAINS).is_some(),
            None => false,
        };
        Ok(updated_image.is_some() || domain_changed)
    })
    .await?;
    Ok((updated_image, domain_changed))
}

pub(crate) async fn set_log_driver(docker: &super::docker::Docker, on: bool) -> Result<()> {
    apply_spec(docker, |spec| {
        if super::insight::is_fluentd(spec.task_template.log_driver.as_ref()) == on {
            return Ok(false);
        }
        spec.task_template.log_driver = on.then(super::insight::fluentd_log_driver);
        prepare_replacement(spec);
        Ok(true)
    })
    .await
}

async fn apply_spec(
    docker: &super::docker::Docker,
    mutate: impl FnOnce(&mut ServiceSpec) -> Result<bool>,
) -> Result<()> {
    let service = require_service(docker).await?;
    let version = service.version.index;
    let mut spec = service.spec;
    if !mutate(&mut spec)? {
        return Ok(());
    }
    docker
        .service_update(config::SERVICE_NAME, version, &spec, None)
        .await
}

pub(super) fn prepare_replacement(spec: &mut ServiceSpec) {
    spec.task_template.container_spec.stop_grace_period = Some(NanoDuration::from_secs(50));
    super::platform::monitor_updates(spec);
}

pub(crate) async fn require_service(docker: &super::docker::Docker) -> Result<Service> {
    docker
        .service_inspect(config::SERVICE_NAME)
        .await?
        .ok_or_else(|| ErrorKind::NoService.into())
}

fn replicated(service: &Service) -> Result<u64> {
    if let ServiceMode::Replicated { replicas } = service.spec.mode {
        Ok(replicas)
    } else {
        Err(InstanceProblem::NotReplicated.into())
    }
}

async fn require_attached(docker: &super::docker::Docker, spec: &ServiceSpec) -> Result<()> {
    let Some(network) = docker.network_inspect(config::NETWORK_NAME).await? else {
        return Err(InstanceProblem::NetworkMissing.into());
    };
    if !matches!(network.driver, NetworkDriver::Overlay) {
        return Err(InstanceProblem::NetworkNotOverlay.into());
    }
    let attached = spec
        .task_template
        .networks
        .as_deref()
        .unwrap_or_default()
        .iter()
        .any(|n| n.target == config::NETWORK_NAME || n.target == network.id);
    if attached {
        Ok(())
    } else {
        Err(InstanceProblem::NotAttached.into())
    }
}
