use docker::types::{RegistryAuth, Service, ServiceConfig, ServiceSpec};
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
) -> Result<(Vec<String>, Vec<String>, Vec<ServiceConfig>)> {
    let port = spec.target_port(app)?;
    Ok(match spec.method() {
        concept::xhealth::Method::Hurl { config, .. } => (
            vec![
                "hurl".to_owned(),
                "--test".to_owned(),
                config::HURL_FILE_PATH.to_owned(),
            ],
            vec![format!(
                "{}=http://{host}:{port}",
                config::HURL_TARGET_URL_ENV
            )],
            vec![super::xconfig::to_service_config(config)],
        ),
        concept::xhealth::Method::Custom { command } => {
            let mut env = super::runner_env(app);
            env.retain(|entry| {
                let key = entry.split_once('=').map_or(entry.as_str(), |(key, _)| key);
                key != config::TARGET_HOST_ENV && key != config::TARGET_PORT_ENV
            });
            env.extend([
                format!("{}={host}", config::TARGET_HOST_ENV),
                format!("{}={port}", config::TARGET_PORT_ENV),
            ]);
            (
                command.clone(),
                env,
                app.xconfigs()
                    .iter()
                    .map(super::xconfig::to_service_config)
                    .collect(),
            )
        }
    })
}

fn build_spec(app: &concept::app::App, spec: &concept::xhealth::Spec) -> Result<ServiceSpec> {
    let (command, env, configs) = method_runtime(app, spec, app.name().as_str())?;
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
    .build())
}

pub(super) fn image_for(app: &concept::app::App, method: &concept::xhealth::Method) -> String {
    match method {
        concept::xhealth::Method::Custom { .. } => app.image().to_string(),
        concept::xhealth::Method::Hurl { image, .. } => image.to_string(),
    }
}

pub(super) fn auth(app: &concept::app::App, spec: &concept::xhealth::Spec) -> Option<RegistryAuth> {
    match spec.method() {
        concept::xhealth::Method::Custom { .. } => super::registry_auth(app),
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
    let service_spec = build_spec(app, spec)?;
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
        build_spec(app, spec)?,
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
