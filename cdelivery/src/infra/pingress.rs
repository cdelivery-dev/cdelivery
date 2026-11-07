use std::collections::HashMap;

use cipher::{Domain, MasterKey};
use docker::types::{
    ContainerSpec, EndpointMode, EndpointPortConfig, EndpointSpec, HealthCheck, NanoDuration,
    NetworkAttachment, Placement, PortProtocol, PortPublishMode, ServiceFileTarget, ServiceMode,
    ServiceSecret, ServiceSpec, TaskTemplate,
};

use crate::concept;
use crate::config;
use crate::error::{ErrorKind, Result, SpecProblem};

pub(crate) async fn create(
    docker: &super::docker::Docker,
    key: &MasterKey,
    pingress: &concept::pingress::Pingress,
) -> Result<()> {
    match docker
        .network_create(config::PINGRESS_NETWORK, HashMap::new())
        .await
    {
        Ok(_) => {}
        Err(e) if e.is_conflict() => {}
        Err(e) => return Err(e),
    }
    let secret_id = super::keyring::ensure_subkey_secret(
        docker,
        key,
        pingress_wire::KEY_SECRET_NAME,
        Domain::Pingress,
    )
    .await?;
    docker
        .service_create_unique(
            &build_spec(pingress, &secret_id),
            None,
            ErrorKind::PingressExists,
        )
        .await
}

pub(crate) async fn load(
    docker: &super::docker::Docker,
) -> Result<Option<concept::versioned::Versioned<concept::pingress::Pingress>>> {
    let Some(service) = docker
        .service_inspect(config::PINGRESS_SERVICE_NAME)
        .await?
    else {
        return Ok(None);
    };
    if !matches!(&service.spec.mode, ServiceMode::Global {}) {
        return Err(SpecProblem::NotGlobal(service.spec.name).into());
    }
    let placement = super::node::decode_selector(
        service.spec.task_template.placement.as_ref(),
        config::PINGRESS_SERVICE_NAME,
    )?;
    let insight_logs = super::insight::is_fluentd(service.spec.task_template.log_driver.as_ref());
    let pingress = concept::pingress::Pingress::hydrate(
        service.spec.task_template.container_spec.image.parse()?,
        placement,
        insight_logs,
    );
    Ok(Some(concept::versioned::Versioned::new(
        service.version.index,
        pingress,
    )))
}

pub(crate) async fn service_id(docker: &super::docker::Docker) -> Result<Option<String>> {
    Ok(docker
        .service_inspect(config::PINGRESS_SERVICE_NAME)
        .await?
        .map(|service| service.id))
}

pub(crate) async fn save(
    docker: &super::docker::Docker,
    pingress: &concept::versioned::Versioned<concept::pingress::Pingress>,
) -> Result<()> {
    if !pingress.changed() {
        return Ok(());
    }
    let secret_id = docker
        .secret_inspect(pingress_wire::KEY_SECRET_NAME)
        .await?
        .map(|s| s.id)
        .ok_or(ErrorKind::PingressNotEnabled)?;
    docker
        .service_update(
            config::PINGRESS_SERVICE_NAME,
            pingress.version(),
            &build_spec(pingress.get(), &secret_id),
            None,
        )
        .await
}

pub(crate) async fn delete(docker: &super::docker::Docker) -> Result<()> {
    docker.service_delete(config::PINGRESS_SERVICE_NAME).await?;
    docker.secret_delete(pingress_wire::KEY_SECRET_NAME).await
}

pub(crate) async fn exists(docker: &super::docker::Docker) -> Result<bool> {
    docker.service_exists(config::PINGRESS_SERVICE_NAME).await
}

pub(crate) async fn instances(docker: &super::docker::Docker) -> Result<u64> {
    Ok(docker
        .service_status(config::PINGRESS_SERVICE_NAME)
        .await?
        .map_or(0, |(running, _)| running))
}

fn build_spec(pingress: &concept::pingress::Pingress, secret_id: &str) -> ServiceSpec {
    let mut spec = ServiceSpec {
        extra: serde_json::Map::new(),
        name: config::PINGRESS_SERVICE_NAME.to_owned(),
        labels: HashMap::new(),
        task_template: TaskTemplate {
            container_spec: ContainerSpec {
                image: pingress.image().to_string(),
                stop_grace_period: Some(NanoDuration::from_secs(30)),
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
                secrets: vec![ServiceSecret {
                    file: ServiceFileTarget {
                        name: pingress_wire::KEY_SECRET_NAME.to_owned(),
                        ..ServiceFileTarget::default()
                    },
                    secret_id: secret_id.to_owned(),
                    secret_name: pingress_wire::KEY_SECRET_NAME.to_owned(),
                }],
                ..ContainerSpec::default()
            },
            networks: Some(
                [config::NETWORK_NAME, config::PINGRESS_NETWORK]
                    .map(|target| NetworkAttachment {
                        target: target.to_owned(),
                        aliases: None,
                        driver_opts: None,
                    })
                    .to_vec(),
            ),
            placement: Some(Placement {
                constraints: Some(vec![super::node::selector_constraint(pingress.placement())]),
                max_replicas: None,
                extra: serde_json::Map::new(),
            }),
            log_driver: pingress
                .insight_logs()
                .then(super::insight::fluentd_log_driver),
            ..TaskTemplate::default()
        },
        mode: ServiceMode::Global {},
        update_config: None,
        rollback_config: None,
        endpoint_spec: Some(EndpointSpec {
            mode: EndpointMode::Vip,
            ports: Some(
                [80u16, 443]
                    .map(|port| EndpointPortConfig {
                        protocol: PortProtocol::Tcp,
                        target_port: port,
                        published_port: Some(port),
                        publish_mode: PortPublishMode::Host,
                    })
                    .to_vec(),
            ),
        }),
    };
    super::platform::monitor_updates(&mut spec);
    spec
}
