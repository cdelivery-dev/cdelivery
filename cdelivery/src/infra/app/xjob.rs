use std::collections::HashMap;

use docker::types::{Filter, Service, ServiceMode, ServiceSpec, TaskState};
use jiff::Timestamp;

use crate::concept;
use crate::error::{Error, ErrorKind, Result, SpecProblem};

const LABEL_NAME: &str = "dev.cdelivery.xjob_name";
const LABEL_CONFIG: &str = "dev.cdelivery.xjob_config";
const SERVICE_PREFIX: &str = "cdelivery-xjob-";

pub(super) fn service_name(app: &concept::name::Name, job: &concept::name::Name) -> String {
    format!("{SERVICE_PREFIX}{app}_{job}")
}

fn labels(
    app: &concept::name::Name,
    job: &concept::name::Name,
    spec: &concept::xjob::Spec,
) -> HashMap<String, String> {
    let mut labels = super::unit::labels(app, super::unit::Kind::XJob);
    labels.extend([
        (LABEL_NAME.to_owned(), job.to_string()),
        (LABEL_CONFIG.to_owned(), super::super::label::json(spec)),
    ]);
    labels
}

fn build_spec(
    app: &concept::app::App,
    job: &concept::name::Name,
    spec: &concept::xjob::Spec,
    replicas: u64,
) -> ServiceSpec {
    super::runner::Spec {
        name: service_name(app.name(), job),
        labels: labels(app.name(), job, spec),
        image: app.image().to_string(),
        network: super::super::group::resource_name(app.group()),
        command: spec.command().clone(),
        env: super::runner_env(app),
        configs: app
            .xconfigs()
            .iter()
            .map(super::xconfig::to_service_config)
            .collect(),
        log_driver: super::encode_log_driver(app.log_driver()),
        replicas,
    }
    .build()
}

pub(super) fn job_name(spec: &ServiceSpec) -> Result<concept::name::Name> {
    super::require(&spec.labels, LABEL_NAME, &spec.name)?
        .parse()
        .map_err(|_| super::bad_label(&spec.name, LABEL_NAME))
}

fn decode(service: &Service) -> Result<concept::xjob::Spec> {
    let spec: concept::xjob::Spec = super::json_label(service, LABEL_CONFIG)?;
    if !spec.valid() {
        return Err(super::bad_label(&service.spec.name, LABEL_CONFIG));
    }
    Ok(spec)
}

async fn find(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
    job: &concept::name::Name,
) -> Result<Option<Service>> {
    let service = docker
        .service_find(
            super::unit::Kind::XJob.for_app(app),
            &service_name(app, job),
        )
        .await?;
    if let Some(service) = &service {
        super::unit::owner(&service.spec, super::unit::Kind::XJob)?;
    }
    Ok(service)
}

pub(crate) async fn service_id(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
    job: &concept::name::Name,
) -> Result<Option<String>> {
    Ok(find(docker, app, job).await?.map(|service| service.id))
}

pub(crate) async fn count(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
) -> Result<usize> {
    Ok(docker
        .service_list(&super::unit::Kind::XJob.for_app(app))
        .await?
        .len())
}

pub(crate) async fn create(
    docker: &super::super::docker::Docker,
    app: &concept::app::App,
    job: &concept::name::Name,
    spec: &concept::xjob::Spec,
) -> Result<()> {
    let service_spec = build_spec(app, job, spec, 0);
    docker
        .service_create_unique(
            &service_spec,
            super::registry_auth(app).as_ref(),
            ErrorKind::XJobExists,
        )
        .await
}

pub(crate) async fn load(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
    job: &concept::name::Name,
) -> Result<Option<concept::versioned::Versioned<concept::xjob::Spec>>> {
    super::versioned(find(docker, app, job).await?, decode)
}

pub(crate) async fn save(
    docker: &super::super::docker::Docker,
    app: &concept::app::App,
    job: &concept::name::Name,
    spec: &concept::versioned::Versioned<concept::xjob::Spec>,
) -> Result<()> {
    if !spec.changed() {
        return Ok(());
    }
    let current = find(docker, app.name(), job)
        .await?
        .ok_or(ErrorKind::Conflict)?;
    docker
        .service_update_labels(
            &current,
            spec.version(),
            super::registry_auth(app).as_ref(),
            |current| *current = labels(app.name(), job, spec.get()),
        )
        .await
}

pub(crate) async fn trigger(
    docker: &super::super::docker::Docker,
    app: &concept::app::App,
    job: &concept::name::Name,
    spec: &concept::xjob::Spec,
    version: u64,
) -> Result<()> {
    let current = find(docker, app.name(), job)
        .await?
        .ok_or(ErrorKind::Conflict)?;
    super::runner::rerun(
        docker,
        &current,
        version,
        super::registry_auth(app).as_ref(),
        build_spec(app, job, spec, 1),
    )
    .await
}

pub(crate) async fn delete(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
    job: &concept::name::Name,
) -> Result<bool> {
    docker.service_delete(&service_name(app, job)).await
}

pub(crate) async fn stop(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
    job: &concept::name::Name,
    version: u64,
) -> Result<bool> {
    let Some(current) = find(docker, app, job).await? else {
        return Ok(false);
    };
    super::runner::stop(docker, current, version).await
}

pub(crate) async fn delete_all(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
) -> Result<()> {
    for service in docker
        .service_list(&super::unit::Kind::XJob.for_app(app))
        .await?
    {
        super::unit::owner(&service.spec, super::unit::Kind::XJob)?;
        docker.service_delete(&service.spec.name).await?;
    }
    Ok(())
}

async fn observe_service(
    docker: &super::super::docker::Docker,
    service: &Service,
    job: concept::name::Name,
    now: Timestamp,
) -> Result<(concept::xjob::XJob, u64)> {
    let spec = decode(service)?;
    let invalid = |detail: String| SpecProblem::BadRunner {
        service: service.spec.name.clone(),
        detail,
    };
    if !matches!(
        service.spec.mode,
        ServiceMode::Replicated { replicas: 0 | 1 }
    ) {
        return Err(invalid("expected zero or one replicas".to_owned()).into());
    }
    let tasks = docker.task_list(&service.spec.name).await?;
    if let Some(task) = super::runner::newest(service, &tasks)
        && let TaskState::Unknown(state) = &task.status.state
    {
        return Err(invalid(format!("unknown task state {state}")).into());
    }
    let last_run = tasks
        .iter()
        .filter(|t| t.service_id == service.id)
        .map(|t| t.created_at)
        .max();
    let state = super::runner::round_status(service, &tasks, now);
    let fallback_at = if service.spec.task_template.force_update == 0 {
        service.created_at
    } else {
        service.updated_at
    };
    Ok((
        concept::xjob::XJob::new(job, spec, state, last_run, fallback_at),
        service.version.index,
    ))
}

pub(crate) async fn observe(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
    job: &concept::name::Name,
    now: Timestamp,
) -> Result<Option<(concept::xjob::XJob, u64)>> {
    match find(docker, app, job).await? {
        Some(service) => Ok(Some(
            observe_service(docker, &service, job.clone(), now).await?,
        )),
        None => Ok(None),
    }
}

async fn scan_jobs(
    docker: &super::super::docker::Docker,
    filter: &Filter,
    now: Timestamp,
) -> Result<(
    Vec<(concept::name::Name, concept::xjob::XJob, u64)>,
    Vec<(String, Error)>,
)> {
    super::scan(docker, filter, |s| async move {
        let owner = super::unit::owner(&s.spec, super::unit::Kind::XJob)?;
        let (xjob, version) = observe_service(docker, &s, job_name(&s.spec)?, now).await?;
        Ok((owner, xjob, version))
    })
    .await
}

pub(crate) async fn list(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
    now: Timestamp,
) -> Result<Vec<concept::xjob::XJob>> {
    let (jobs, _) = scan_jobs(docker, &super::unit::Kind::XJob.for_app(app), now).await?;
    let mut jobs = jobs.into_iter().map(|(_, job, _)| job).collect::<Vec<_>>();
    jobs.sort_by(|a, b| a.name().as_str().cmp(b.name().as_str()));
    Ok(jobs)
}

pub(crate) async fn observe_all(
    docker: &super::super::docker::Docker,
    now: Timestamp,
) -> Result<(
    Vec<(concept::name::Name, concept::xjob::XJob, u64)>,
    Vec<(String, Error)>,
)> {
    scan_jobs(docker, &super::unit::Kind::XJob.filter(), now).await
}
