use std::collections::{HashMap, HashSet};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use docker::types::{Filter, ServiceConfig, ServiceFileTarget};
use jiff::Timestamp;
use uuid::Uuid;

use crate::concept;
use crate::config;
use crate::error::Result;

fn config_name(app: &concept::name::Name) -> String {
    format!("{app}-{}", Uuid::new_v4().simple())
}

pub(crate) async fn upload(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
    target: &str,
    data: &[u8],
) -> Result<concept::app::XConfigRef> {
    let name = config_name(app);
    let labels = HashMap::from([(super::unit::LABEL_APP.to_owned(), app.to_string())]);
    let id = docker.config_create(&name, labels, data).await?;
    Ok(concept::app::XConfigRef::new(target.to_owned(), name, id))
}

pub(crate) async fn download(
    docker: &super::super::docker::Docker,
    config_name: &str,
) -> Result<Option<Vec<u8>>> {
    let Some(config) = docker.config_inspect(config_name).await? else {
        return Ok(None);
    };
    let Some(data) = config.spec.data else {
        return Ok(None);
    };
    Ok(Some(STANDARD.decode(data.as_bytes())?))
}

pub(crate) fn to_service_config(x: &concept::app::XConfigRef) -> ServiceConfig {
    ServiceConfig {
        file: ServiceFileTarget {
            name: x.target().clone(),
            ..ServiceFileTarget::default()
        },
        config_id: x.config_id().clone(),
        config_name: x.config_name().clone(),
    }
}

pub(crate) fn to_ref(config: &ServiceConfig) -> concept::app::XConfigRef {
    concept::app::XConfigRef::new(
        config.file.name.clone(),
        config.config_name.clone(),
        config.config_id.clone(),
    )
}

pub(crate) async fn sweep(docker: &super::super::docker::Docker, now: Timestamp) -> Result<usize> {
    let configs = docker.config_list(super::unit::LABEL_APP).await?;
    if configs.is_empty() {
        return Ok(0);
    }
    let mut referenced = HashSet::new();
    for service in docker
        .service_list(&Filter::new().label(super::unit::LABEL_APP))
        .await?
    {
        let specs = [Some(&service.spec), service.previous_spec.as_ref()];
        for reference in specs
            .into_iter()
            .flatten()
            .flat_map(|s| &s.task_template.container_spec.configs)
        {
            referenced.insert(reference.config_id.clone());
            referenced.insert(reference.config_name.clone());
        }
    }
    let mut removed = 0;
    for config in configs {
        if now.duration_since(config.created_at) <= config::XCONFIG_GC_GRACE
            || referenced.contains(&config.id)
            || referenced.contains(&config.spec.name)
        {
            continue;
        }
        tracing::warn!(
            "collecting orphan xconfig {}: {}",
            config.spec.name,
            serde_json::json!(config.spec.labels)
        );
        docker.config_delete(&config.spec.name).await?;
        removed += 1;
    }
    Ok(removed)
}
