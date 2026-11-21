use std::net::IpAddr;

use docker::types::{NodeState, RegistryAuth, Service, ServiceConfig, ServiceSpec};
use jiff::Timestamp;

use crate::concept;
use crate::config;
use crate::error::{Error, ErrorKind, Result};

const LABEL_CONFIG: &str = "dev.cdelivery.xhealth_config";
const SERVICE_PREFIX: &str = "cdelivery-xhealth-";

pub(super) fn service_name(app: &concept::name::Name) -> String {
    format!("{SERVICE_PREFIX}{app}")
}

pub(super) fn method_runtime(
    app: &concept::app::App,
    spec: &concept::xhealth::Spec,
    host: &str,
    port: u16,
    https: bool,
) -> (Vec<String>, Vec<String>, Vec<ServiceConfig>) {
    let mut env = super::runner_env(app);
    let mut configs = super::user_configs(app);
    let reserved = spec.reserved_env();
    env.retain(|entry| {
        let key = entry.split_once('=').map_or(entry.as_str(), |(key, _)| key);
        !reserved.contains(&key)
    });
    let scheme = if https { "https" } else { "http" };
    let command = match spec.method() {
        concept::xhealth::Method::Hurl { config, .. } => {
            let host = if host.contains(':') {
                format!("[{host}]")
            } else {
                host.to_owned()
            };
            env.push(format!(
                "{}={scheme}://{host}:{port}",
                config::HURL_TARGET_URL_ENV
            ));
            configs.push(super::xconfig::to_service_config(config));
            vec![
                "hurl".to_owned(),
                "--test".to_owned(),
                config::HURL_FILE_PATH.to_owned(),
            ]
        }
        concept::xhealth::Method::Custom { command } => {
            env.extend([
                format!("{}={host}", config::TARGET_HOST_ENV),
                format!("{}={port}", config::TARGET_PORT_ENV),
            ]);
            if matches!(spec.target(), concept::xhealth::Target::Domain(_)) {
                env.push(format!("{}={scheme}", config::TARGET_SCHEME_ENV));
            }
            command.clone()
        }
    };
    (command, env, configs)
}

async fn build_spec(
    docker: &super::super::docker::Docker,
    app: &concept::app::App,
    spec: &concept::xhealth::Spec,
) -> Result<ServiceSpec> {
    spec.validate_app(app)?;
    let (host, port, https) = match spec.target() {
        concept::xhealth::Target::Domain(domain) => {
            let https = super::list_domains(docker)
                .await?
                .iter()
                .flat_map(concept::app::Routing::rules)
                .any(|rule| rule.domain().as_str() == domain.as_str() && rule.https());
            (domain.to_string(), if https { 443 } else { 80 }, https)
        }
        concept::xhealth::Target::PublishedPort { published } => {
            let (_, address) = docker
                .node_list()
                .await?
                .into_iter()
                .filter(|node| matches!(node.status.state, NodeState::Ready))
                .filter_map(|node| {
                    let address = node.status.addr.parse::<IpAddr>().ok()?;
                    (!address.is_loopback() && !address.is_unspecified())
                        .then_some((node.id, address))
                })
                .min_by(|a, b| a.0.cmp(&b.0))
                .ok_or(ErrorKind::XHealthNodeAddressUnavailable)?;
            (address.to_string(), published.get(), false)
        }
        concept::xhealth::Target::InternalPort(port) => (app.name().to_string(), port.get(), false),
    };
    let (command, env, configs) = method_runtime(app, spec, &host, port, https);
    let mut labels = super::unit::labels(app.name(), super::unit::Kind::XHealth);
    labels.insert(LABEL_CONFIG.to_owned(), super::super::label::json(spec));
    Ok(super::runner::Spec {
        name: service_name(app.name()),
        labels,
        image: image_for(app, spec.method()),
        network: super::super::group::resource_name(app.group()),
        command,
        env,
        configs,
        log_driver: super::encode_log_driver(app.log_driver()),
        replicas: 1,
    }
    .build(app))
}

pub(super) fn image_for(app: &concept::app::App, method: &concept::xhealth::Method) -> String {
    match method {
        concept::xhealth::Method::Custom { .. } => app.image().to_string(),
        concept::xhealth::Method::Hurl { image, .. } => image.to_string(),
    }
}

pub(super) fn auth(app: &concept::app::App, spec: &concept::xhealth::Spec) -> Option<RegistryAuth> {
    match spec.method() {
        concept::xhealth::Method::Custom { .. } => Some(super::registry_auth(app)),
        concept::xhealth::Method::Hurl { .. } => None,
    }
}

async fn find(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
) -> Result<Option<Service>> {
    docker
        .service_find(super::unit::Kind::XHealth.for_app(app), &service_name(app))
        .await
}

pub(crate) async fn service_id(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
) -> Result<Option<String>> {
    Ok(find(docker, app).await?.map(|service| service.id))
}

fn decode(service: &Service) -> Result<concept::xhealth::Spec> {
    let spec: concept::xhealth::Spec = super::json_label(service, LABEL_CONFIG)?;
    concept::xhealth::Spec::validate_interval(spec.interval())
        .map_err(|_| super::bad_label(&service.spec.name, LABEL_CONFIG))?;
    Ok(spec)
}

pub(crate) async fn load(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
) -> Result<Option<concept::versioned::Versioned<concept::xhealth::Spec>>> {
    super::versioned(find(docker, app).await?, decode)
}

pub(crate) async fn create(
    docker: &super::super::docker::Docker,
    app: &concept::app::App,
    spec: &concept::xhealth::Spec,
) -> Result<()> {
    let service_spec = build_spec(docker, app, spec).await?;
    docker
        .service_create_unique(
            &service_spec,
            auth(app, spec).as_ref(),
            ErrorKind::XHealthExists,
        )
        .await
}

pub(crate) async fn save(
    docker: &super::super::docker::Docker,
    app: &concept::app::App,
    spec: &concept::versioned::Versioned<concept::xhealth::Spec>,
) -> Result<()> {
    if !spec.changed() {
        return Ok(());
    }
    run_round(docker, app, spec.get(), spec.version()).await
}

pub(crate) async fn run_round(
    docker: &super::super::docker::Docker,
    app: &concept::app::App,
    spec: &concept::xhealth::Spec,
    service_version: u64,
) -> Result<()> {
    let current = find(docker, app.name()).await?.ok_or(ErrorKind::Conflict)?;
    super::runner::rerun(
        docker,
        &current,
        service_version,
        auth(app, spec).as_ref(),
        build_spec(docker, app, spec).await?,
    )
    .await
}

pub(crate) async fn delete(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
) -> Result<bool> {
    docker.service_delete(&service_name(app)).await
}

pub(crate) async fn stop(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
    version: u64,
) -> Result<bool> {
    let Some(current) = find(docker, app).await? else {
        return Ok(false);
    };
    super::runner::stop(docker, current, version).await
}

async fn observe_service(
    docker: &super::super::docker::Docker,
    service: &Service,
    now: Timestamp,
) -> Result<concept::xhealth::XHealth> {
    let spec = decode(service)?;
    let tasks = docker.task_list(&service.spec.name).await?;
    let started_at =
        super::runner::newest(service, &tasks).map_or(service.updated_at, |task| task.created_at);
    Ok(concept::xhealth::XHealth::new(
        spec,
        super::runner::round_status(service, &tasks, now),
        started_at,
    ))
}

pub(crate) async fn observe(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
    now: Timestamp,
) -> Result<Option<(concept::xhealth::XHealth, u64)>> {
    let Some(service) = find(docker, app).await? else {
        return Ok(None);
    };
    Ok(Some((
        observe_service(docker, &service, now).await?,
        service.version.index,
    )))
}

pub(crate) async fn observe_all(
    docker: &super::super::docker::Docker,
    now: Timestamp,
) -> Result<(
    Vec<(concept::name::Name, concept::xhealth::XHealth, u64)>,
    Vec<(String, Error)>,
)> {
    super::scan(
        docker,
        &super::unit::Kind::XHealth.filter(),
        |s| async move {
            Ok((
                super::unit::owner(&s.spec, super::unit::Kind::XHealth)?,
                observe_service(docker, &s, now).await?,
                s.version.index,
            ))
        },
    )
    .await
}
