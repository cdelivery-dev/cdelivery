use cipher::{Domain, MasterKey};
use docker::types::{
    ContainerSpec, HealthCheck, Mount, MountType, NanoDuration, Placement, ServiceFileTarget,
    ServiceMode, ServiceSecret, ServiceSpec, TaskTemplate, UpdateConfig, UpdateOrder,
};
use insight_wire::ObjectStoreConfig;

use crate::concept;
use crate::config;
use crate::error::{Result, SpecProblem};

pub(crate) async fn create(
    docker: &super::super::docker::Docker,
    key: &MasterKey,
    insight: &concept::insight::Insight,
    secret_id: &str,
) -> Result<()> {
    let sealed = seal_storage(key, insight.storage())?;
    docker
        .service_create(&build_spec(insight, secret_id, &sealed), None)
        .await?;
    Ok(())
}

pub(crate) async fn load(
    docker: &super::super::docker::Docker,
    key: &MasterKey,
) -> Result<Option<concept::versioned::Versioned<concept::insight::Insight>>> {
    let Some(service) = docker.service_inspect(config::INSIGHT_SERVICE_NAME).await? else {
        return Ok(None);
    };
    let spec = &service.spec;
    let storage: ObjectStoreConfig = key
        .cipher_for(Domain::Insight)
        .open(super::env_value(spec, insight_wire::STORAGE_ENV)?)?;
    let retention_days = super::env_value(spec, insight_wire::DATA_RETENTION_DAYS_ENV)?
        .parse::<u16>()
        .map_err(|_| SpecProblem::BadEnv {
            service: spec.name.clone(),
            env: insight_wire::DATA_RETENTION_DAYS_ENV.to_owned(),
        })?;
    let flush_interval = super::env_value(spec, insight_wire::WRITER_FLUSH_INTERVAL_SECS_ENV)?
        .parse::<i64>()
        .ok()
        .and_then(insight_wire::writer_flush_interval_from_secs)
        .ok_or_else(|| SpecProblem::BadEnv {
            service: spec.name.clone(),
            env: insight_wire::WRITER_FLUSH_INTERVAL_SECS_ENV.to_owned(),
        })?;
    if !matches!(&spec.mode, ServiceMode::Global {}) {
        return Err(SpecProblem::NotGlobal(spec.name.clone()).into());
    }
    let placement =
        super::super::node::decode_selector(spec.task_template.placement.as_ref(), &spec.name)?;
    if super::env_value(spec, insight_wire::SINGLE_INSTANCE_ENV)?
        != placement_single_instance(&placement).to_string()
    {
        return Err(SpecProblem::BadEnv {
            service: spec.name.clone(),
            env: insight_wire::SINGLE_INSTANCE_ENV.to_owned(),
        }
        .into());
    }
    if matches!(&storage, ObjectStoreConfig::Local)
        && !matches!(&placement, concept::node::Selector::NodeId(_))
    {
        return Err(SpecProblem::NotPinned(spec.name.clone()).into());
    }
    let insight = concept::insight::Insight::new(
        storage,
        retention_days,
        flush_interval,
        placement,
        spec.task_template.container_spec.image.parse()?,
    );
    Ok(Some(concept::versioned::Versioned::new(
        service.version.index,
        insight,
    )))
}

pub(crate) async fn save(
    docker: &super::super::docker::Docker,
    key: &MasterKey,
    insight: &concept::versioned::Versioned<concept::insight::Insight>,
    secret_id: &str,
) -> Result<()> {
    if !insight.changed() {
        return Ok(());
    }
    let sealed = seal_storage(key, insight.get().storage())?;
    docker
        .service_update(
            config::INSIGHT_SERVICE_NAME,
            insight.version(),
            &build_spec(insight.get(), secret_id, &sealed),
            None,
        )
        .await
}

pub(crate) async fn delete(docker: &super::super::docker::Docker) -> Result<()> {
    docker.service_delete(config::INSIGHT_SERVICE_NAME).await?;
    docker.secret_delete(insight_wire::KEY_SECRET_NAME).await
}

pub(crate) async fn exists(docker: &super::super::docker::Docker) -> Result<bool> {
    docker.service_exists(config::INSIGHT_SERVICE_NAME).await
}

pub(crate) async fn service_id(docker: &super::super::docker::Docker) -> Result<Option<String>> {
    Ok(docker
        .service_inspect(config::INSIGHT_SERVICE_NAME)
        .await?
        .map(|service| service.id))
}

fn seal_storage(key: &MasterKey, storage: &ObjectStoreConfig) -> Result<String> {
    Ok(key.cipher_for(Domain::Insight).seal(storage)?)
}

fn placement_single_instance(placement: &concept::node::Selector) -> bool {
    matches!(placement, concept::node::Selector::NodeId(_))
}

fn build_spec(
    insight: &concept::insight::Insight,
    secret_id: &str,
    sealed_storage: &str,
) -> ServiceSpec {
    let single_instance = placement_single_instance(insight.placement());
    let mut spec = ServiceSpec {
        extra: serde_json::Map::new(),
        name: config::INSIGHT_SERVICE_NAME.to_owned(),
        labels: std::collections::HashMap::new(),
        task_template: TaskTemplate {
            container_spec: ContainerSpec {
                image: insight.image().to_string(),
                stop_grace_period: Some(super::STOP_GRACE_PERIOD),
                env: vec![
                    format!("{}={sealed_storage}", insight_wire::STORAGE_ENV),
                    super::node_id_env(),
                    format!(
                        "{}={}",
                        insight_wire::DATA_RETENTION_DAYS_ENV,
                        insight.retention_days()
                    ),
                    format!(
                        "{}={}",
                        insight_wire::WRITER_FLUSH_INTERVAL_SECS_ENV,
                        insight.flush_interval().as_secs()
                    ),
                    format!("{}={single_instance}", insight_wire::SINGLE_INSTANCE_ENV),
                ],
                healthcheck: Some(HealthCheck {
                    test: ["CMD", "/entrypoint", "healthz"]
                        .map(str::to_owned)
                        .to_vec(),
                    interval: NanoDuration::from_secs(5),
                    timeout: NanoDuration::from_secs(3),
                    retries: 3,
                    start_period: NanoDuration::from_secs(60),
                    start_interval: None,
                }),
                mounts: vec![Mount {
                    kind: MountType::Volume,
                    target: insight_wire::LOCAL_DATA_DIR.to_owned(),
                    source: config::INSIGHT_VOLUME.to_owned(),
                    read_only: false,
                    bind_options: None,
                }],
                secrets: vec![ServiceSecret {
                    file: ServiceFileTarget {
                        name: insight_wire::KEY_SECRET_NAME.to_owned(),
                        ..ServiceFileTarget::default()
                    },
                    secret_id: secret_id.to_owned(),
                    secret_name: insight_wire::KEY_SECRET_NAME.to_owned(),
                }],
                ..ContainerSpec::default()
            },
            networks: Some(super::platform_network()),
            placement: Some(Placement {
                constraints: Some(vec![super::super::node::selector_constraint(
                    insight.placement(),
                )]),
                max_replicas: None,
                extra: serde_json::Map::new(),
            }),
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
