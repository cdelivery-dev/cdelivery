use docker::types::ServiceSpec;

use crate::concept;
use crate::error::Result;

pub(crate) async fn list(
    docker: &super::super::docker::Docker,
    app: &concept::name::Name,
) -> Result<Option<Vec<concept::app::LogSource>>> {
    let mut found = false;
    let mut sources = Vec::new();
    for service in docker.service_list(&super::unit::filter(app)).await? {
        match source(&service.spec) {
            Ok(source) => {
                if matches!(source, concept::app::LogSource::App) {
                    found = true;
                }
                if super::super::insight::is_fluentd(service.spec.task_template.log_driver.as_ref())
                {
                    sources.push(source);
                }
            }
            Err(error) => {
                tracing::warn!(error = ?error, "skipping invalid log source {} for app {app}", service.spec.name);
            }
        }
    }
    sources.sort();
    Ok(found.then_some(sources))
}

fn source(spec: &ServiceSpec) -> Result<concept::app::LogSource> {
    Ok(match super::unit::decode(spec)?.1 {
        super::unit::Kind::App => concept::app::LogSource::App,
        super::unit::Kind::XHealth => concept::app::LogSource::XHealth,
        super::unit::Kind::XJob => concept::app::LogSource::XJob {
            job: super::xjob::job_name(spec)?,
        },
        super::unit::Kind::GuardianApp => concept::app::LogSource::GuardianApp,
        super::unit::Kind::GuardianCheck => concept::app::LogSource::GuardianCheck,
    })
}
