use cipher::MasterKey;
use insight_wire::ObjectStoreConfig;
use insight_wire::event::Kind;
use jiff::{SignedDuration, Timestamp};

use crate::concept;
use crate::config;
use crate::error::{self, ErrorKind, Result};
use crate::infra;

pub(crate) async fn require_collection(docker: &infra::docker::Docker) -> Result<()> {
    require_enabled(infra::insight::collection_ready(docker).await?)
}

pub(crate) async fn require_server(docker: &infra::docker::Docker) -> Result<()> {
    require_enabled(infra::insight::server::exists(docker).await?)
}

pub(crate) enum ContainerTarget<'a> {
    App(&'a concept::name::Name),
    XHealth(&'a concept::name::Name),
    XJob(&'a concept::name::Name, &'a concept::name::Name),
    GuardianApp(&'a concept::name::Name),
    GuardianCheck(&'a concept::name::Name),
    Pingress,
    Instance,
    InsightServer,
    InsightAgent,
}

async fn service_id(docker: &infra::docker::Docker, target: ContainerTarget<'_>) -> Result<String> {
    Ok(match target {
        ContainerTarget::App(name) => infra::app::service_id(docker, name)
            .await?
            .ok_or(ErrorKind::AppNotFound)?,
        ContainerTarget::XHealth(name) => infra::app::xhealth::service_id(docker, name)
            .await?
            .ok_or(ErrorKind::XHealthNotEnabled)?,
        ContainerTarget::XJob(name, job) => infra::app::xjob::service_id(docker, name, job)
            .await?
            .ok_or(ErrorKind::XJobNotFound)?,
        ContainerTarget::GuardianApp(name) => infra::app::guardian::app_service_id(docker, name)
            .await?
            .ok_or(ErrorKind::GuardianNotFound)?,
        ContainerTarget::GuardianCheck(name) => {
            infra::app::guardian::check_service_id(docker, name)
                .await?
                .ok_or(ErrorKind::GuardianNotFound)?
        }
        ContainerTarget::Pingress => infra::pingress::service_id(docker)
            .await?
            .ok_or(ErrorKind::PingressNotEnabled)?,
        ContainerTarget::Instance => infra::instance::service_id(docker).await?,
        ContainerTarget::InsightServer => infra::insight::server::service_id(docker)
            .await?
            .ok_or(ErrorKind::InsightNotEnabled)?,
        ContainerTarget::InsightAgent => infra::insight::agent::service_id(docker)
            .await?
            .ok_or(ErrorKind::InsightNotEnabled)?,
    })
}

pub(crate) async fn logs(
    docker: &infra::docker::Docker,
    api: &infra::insight::query::Api,
    target: ContainerTarget<'_>,
    params: infra::insight::query::LogParams,
) -> Result<(String, Vec<insight_wire::log::Log>)> {
    require_server(docker).await?;
    let service_id = service_id(docker, target).await?;
    let records = api.logs(&service_id, params).await?;
    Ok((service_id, records))
}

pub(crate) async fn container_metrics(
    docker: &infra::docker::Docker,
    api: &infra::insight::query::Api,
    target: ContainerTarget<'_>,
    range: (Timestamp, Timestamp),
    instance: Option<String>,
    step_secs: u32,
) -> Result<(
    Vec<insight_wire::metric::ContainerBucket>,
    Vec<insight_wire::metric::ContainerInstance>,
)> {
    require_server(docker).await?;
    let service_id = service_id(docker, target).await?;
    let (points, mut instances) = tokio::try_join!(
        api.container_series(range, service_id.clone(), instance, step_secs),
        api.container_instances(range, service_id),
    )?;
    instances.sort_by(|a, b| {
        b.container_started_at
            .cmp(&a.container_started_at)
            .then_with(|| a.container_id.cmp(&b.container_id))
    });
    Ok((points, instances))
}

fn require_enabled(enabled: bool) -> Result<()> {
    enabled
        .then_some(())
        .ok_or_else(|| ErrorKind::InsightNotEnabled.into())
}

pub(crate) async fn step_floor(docker: &infra::docker::Docker, step_secs: u32) -> u32 {
    match infra::insight::agent::observe(docker).await {
        Ok(Some((_, interval))) => config::step_floor(step_secs, interval),
        Ok(None) => step_secs,
        Err(error) => {
            tracing::warn!(error = ?error, "agent sample interval unavailable, keeping raw step");
            step_secs
        }
    }
}

#[expect(clippy::too_many_arguments)]
pub(crate) async fn enable(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    releases: &infra::release::Catalog,
    storage: Option<ObjectStoreConfig>,
    retention_days: Option<u16>,
    placement: Option<concept::node::Selector>,
    server_version: Option<String>,
    agent_version: Option<String>,
    sample_interval_secs: Option<u16>,
    flush_interval: Option<SignedDuration>,
) -> Result<concept::insight::Deployment> {
    infra::instance::require_network(docker).await?;
    let existing = infra::insight::server::load(docker, key).await?;
    let current = existing.as_ref().map(concept::versioned::Versioned::get);
    let storage = match storage {
        Some(new) => resolve_storage(new, current.map(concept::insight::Insight::storage))?,
        None => error::required(current.map(|c| c.storage().clone()), "storage")?,
    };
    let retention_days = error::required(
        retention_days.or(current.map(concept::insight::Insight::retention_days)),
        "retention_days",
    )?;
    let placement = error::required(
        placement.or_else(|| current.map(|c| c.placement().clone())),
        "placement",
    )?;
    if matches!(&storage, ObjectStoreConfig::Local)
        && !matches!(&placement, concept::node::Selector::NodeId(_))
    {
        return Err(ErrorKind::InsightLocalStorageRequiresNode.into());
    }
    if let concept::node::Selector::Label(label) = &placement
        && !infra::node::observe_all(docker)
            .await?
            .iter()
            .any(|node| node.labels().contains(label))
    {
        return Err(ErrorKind::InsightLabelMatchesNoNodes(label.clone()).into());
    }
    if let Some(current) = current
        && matches!(current.storage(), ObjectStoreConfig::Local)
        && current.placement() != &placement
    {
        return Err(ErrorKind::InsightStorageMigrationUnsupported.into());
    }
    let current_agent = infra::insight::agent::observe_for_reconcile(docker).await?;
    let server_image = super::release::select(
        releases,
        concept::release::Component::Insight,
        server_version.as_deref(),
        current.map(concept::insight::Insight::image),
        "server_version",
    )?;
    let agent_image = super::release::select(
        releases,
        concept::release::Component::InsightAgent,
        agent_version.as_deref(),
        current_agent.as_ref().map(|(image, _)| image),
        "agent_version",
    )?;
    let current_interval = current_agent.as_ref().and_then(|(_, interval)| *interval);
    let sample_interval_secs = sample_interval_secs
        .or(current_interval)
        .unwrap_or(config::INSIGHT_SAMPLE_INTERVAL_DEFAULT_SECS);
    let flush_interval = flush_interval
        .or(current.map(concept::insight::Insight::flush_interval))
        .unwrap_or(config::INSIGHT_WRITER_FLUSH_INTERVAL_DEFAULT);
    let secret_id = infra::keyring::ensure_subkey_secret(
        docker,
        key,
        insight_wire::KEY_SECRET_NAME,
        cipher::Domain::Insight,
    )
    .await?;
    infra::insight::agent::reconcile(docker, &agent_image, sample_interval_secs).await?;
    if let Some(current) = current_interval
        && current != sample_interval_secs
    {
        infra::event::emit(
            Kind::InsightUpdated,
            config::INSIGHT_AGENT_SERVICE_NAME,
            &format!("sample interval {current}s -> {sample_interval_secs}s"),
        );
    }
    let insight = concept::insight::Insight::new(
        storage,
        retention_days,
        flush_interval,
        placement,
        server_image,
    );
    if let Some(mut versioned) = existing {
        *versioned.get_mut() = insight;
        infra::insight::server::save(docker, key, &versioned, &secret_id).await?;
    } else {
        infra::insight::server::create(docker, key, &insight, &secret_id).await?;
    }
    set_pingress_logs(docker, true).await?;
    infra::instance::set_log_driver(docker, true).await?;
    overview(docker, key).await
}

pub(crate) async fn disable(docker: &infra::docker::Docker) -> Result<()> {
    let services = infra::app::list_insight_consumers(docker).await?;
    if !services.is_empty() {
        return Err(ErrorKind::InsightBusy(services.join(", ")).into());
    }
    set_pingress_logs(docker, false).await?;
    infra::insight::agent::delete(docker).await?;
    infra::insight::server::delete(docker).await?;
    infra::instance::set_log_driver(docker, false).await
}

pub(crate) async fn overview(
    docker: &infra::docker::Docker,
    key: &MasterKey,
) -> Result<concept::insight::Deployment> {
    let server = infra::insight::server::load(docker, key)
        .await?
        .map(concept::versioned::Versioned::into_inner);
    let agent = infra::insight::agent::observe(docker).await?;
    Ok(concept::insight::Deployment::new(server, agent))
}

async fn set_pingress_logs(docker: &infra::docker::Docker, on: bool) -> Result<()> {
    let Some(mut pingress) = infra::pingress::load(docker).await? else {
        return Ok(());
    };
    pingress.get_mut().set_insight_logs(on);
    infra::pingress::save(docker, &pingress).await
}

fn resolve_storage(
    mut new: ObjectStoreConfig,
    existing: Option<&ObjectStoreConfig>,
) -> Result<ObjectStoreConfig> {
    if let Some(old) = existing {
        match (&mut new, old) {
            (ObjectStoreConfig::Local, ObjectStoreConfig::Local) => {}
            (
                ObjectStoreConfig::S3 {
                    bucket_name,
                    access_key_id,
                    secret_access_key,
                    ..
                },
                ObjectStoreConfig::S3 {
                    bucket_name: old_bucket,
                    access_key_id: old_id,
                    secret_access_key: old_secret,
                    ..
                },
            ) if bucket_name == old_bucket => {
                if secret_access_key.is_empty() && access_key_id == old_id {
                    *secret_access_key = old_secret.clone();
                }
            }
            (
                ObjectStoreConfig::S3Compatible {
                    endpoint,
                    bucket_name,
                    access_key_id,
                    secret_access_key,
                    ..
                },
                ObjectStoreConfig::S3Compatible {
                    endpoint: old_endpoint,
                    bucket_name: old_bucket,
                    access_key_id: old_id,
                    secret_access_key: old_secret,
                    ..
                },
            ) if endpoint == old_endpoint && bucket_name == old_bucket => {
                if secret_access_key.is_empty() && access_key_id == old_id {
                    *secret_access_key = old_secret.clone();
                }
            }
            (
                ObjectStoreConfig::Gcs {
                    bucket_name,
                    credentials,
                },
                ObjectStoreConfig::Gcs {
                    bucket_name: old_bucket,
                    credentials: old_credentials,
                },
            ) if bucket_name == old_bucket => {
                if credentials.is_empty() {
                    *credentials = old_credentials.clone();
                }
            }
            (
                ObjectStoreConfig::Azure {
                    container_name,
                    account_name,
                    account_key,
                },
                ObjectStoreConfig::Azure {
                    container_name: old_container,
                    account_name: old_account,
                    account_key: old_key,
                },
            ) if container_name == old_container && account_name == old_account => {
                if account_key.is_empty() {
                    *account_key = old_key.clone();
                }
            }
            _ => return Err(ErrorKind::InsightStorageMigrationUnsupported.into()),
        }
    }
    if let Some((field, secret)) = secret(&new)
        && secret.is_empty()
    {
        return Err(ErrorKind::SecretRequired(field.to_owned()).into());
    }
    Ok(new)
}

fn secret(storage: &ObjectStoreConfig) -> Option<(&'static str, &cipher::Secret)> {
    match storage {
        ObjectStoreConfig::Local => None,
        ObjectStoreConfig::S3 {
            secret_access_key, ..
        }
        | ObjectStoreConfig::S3Compatible {
            secret_access_key, ..
        } => Some(("storage.secret_access_key", secret_access_key)),
        ObjectStoreConfig::Gcs { credentials, .. } => Some(("storage.credentials", credentials)),
        ObjectStoreConfig::Azure { account_key, .. } => Some(("storage.account_key", account_key)),
    }
}
