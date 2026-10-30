use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use cipher::MasterKey;
use insight_wire::event::Kind;
use jiff::Timestamp;
use sha2::{Digest, Sha256};

use crate::concept;
use crate::config;
use crate::error::{ErrorKind, Result};
use crate::infra;

pub(super) async fn require_enabled(docker: &infra::docker::Docker) -> Result<()> {
    if infra::pingress::exists(docker).await? {
        Ok(())
    } else {
        Err(ErrorKind::PingressNotEnabled.into())
    }
}

pub(super) async fn check_routes(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    routes: &[concept::domain::Rule],
    replaced_service: Option<&str>,
) -> Result<()> {
    if routes.is_empty() {
        return Ok(());
    }
    require_enabled(docker).await?;
    let https: Vec<concept::domain::DomainName> = routes
        .iter()
        .filter(|rule| rule.https())
        .map(concept::domain::Rule::domain)
        .collect();
    if https.is_empty() {
        return Ok(());
    }
    super::acme::check_capacity(docker, key, replaced_service, https).await
}

pub(crate) async fn enable(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    releases: &infra::release::Catalog,
    version: &str,
    placement: concept::node::Selector,
) -> Result<concept::pingress::Deployment> {
    let insight_logs = infra::insight::collection_ready(docker).await?;
    infra::instance::require_network(docker).await?;
    let image = super::release::resolve(
        releases,
        concept::release::Component::Pingress,
        version,
        "version",
    )?;
    let pingress = concept::pingress::Pingress::hydrate(image, placement, insight_logs);
    infra::pingress::create(docker, key, &pingress).await?;
    infra::event::emit(
        Kind::PingressEnabled,
        config::PINGRESS_SERVICE_NAME,
        &format!("image {}", pingress.image()),
    );
    get(docker, key).await
}

pub(crate) async fn get(
    docker: &infra::docker::Docker,
    key: &MasterKey,
) -> Result<concept::pingress::Deployment> {
    let pingress =
        super::found(infra::pingress::load(docker), ErrorKind::PingressNotEnabled).await?;
    let instances = infra::pingress::instances(docker).await?;
    let acme_enabled = infra::acme::load(docker, key)
        .await?
        .is_some_and(|store| store.get().enabled());
    Ok(concept::pingress::Deployment::new(
        pingress,
        instances,
        acme_enabled,
    ))
}

pub(crate) async fn update(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    releases: &infra::release::Catalog,
    version: Option<&str>,
    placement: Option<concept::node::Selector>,
) -> Result<concept::pingress::Deployment> {
    let mut pingress =
        super::require(infra::pingress::load(docker), ErrorKind::PingressNotEnabled).await?;
    let image = if let Some(version) = version {
        Some(super::release::resolve(
            releases,
            concept::release::Component::Pingress,
            version,
            "version",
        )?)
    } else {
        None
    };
    pingress.get_mut().retarget(image, placement);
    if pingress.changed() {
        infra::pingress::save(docker, &pingress).await?;
        infra::event::emit(
            Kind::PingressUpdated,
            config::PINGRESS_SERVICE_NAME,
            &format!("image {}", pingress.get().image()),
        );
    }
    get(docker, key).await
}

pub(crate) async fn disable(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    challenges: &infra::acme::Challenges,
) -> Result<()> {
    if !infra::pingress::exists(docker).await? {
        return Ok(());
    }
    let apps = infra::app::list_domains(docker).await?;
    if !apps.is_empty() {
        let names: Vec<&str> = apps.iter().map(|routing| routing.name().as_str()).collect();
        return Err(ErrorKind::PingressBusy(names.join(", ")).into());
    }
    if let Some(store) = infra::acme::load(docker, key).await? {
        let statics: Vec<String> = store
            .get()
            .certs()
            .iter()
            .filter(|(_, cert)| cert.is_static())
            .map(|(domain, _)| domain.to_string())
            .collect();
        if !statics.is_empty() {
            return Err(ErrorKind::PingressBusy(statics.join(", ")).into());
        }
    }
    infra::acme::delete(docker).await?;
    challenges.clear_sync();
    super::acme::clear_issue_failures();
    infra::pingress::delete(docker).await?;
    infra::event::emit(Kind::PingressDisabled, config::PINGRESS_SERVICE_NAME, "");
    Ok(())
}

pub(crate) async fn access(
    docker: &infra::docker::Docker,
    api: &infra::insight::query::Api,
    range: (Timestamp, Timestamp),
    skip: u32,
    limit: u16,
    filter: pingress_wire::access::Filter,
) -> Result<Vec<pingress_wire::access::Access>> {
    super::insight::require_server(docker).await?;
    api.access(range, skip, limit, &filter).await
}

pub(crate) async fn access_stats(
    docker: &infra::docker::Docker,
    api: &infra::insight::query::Api,
    range: (Timestamp, Timestamp),
    step_secs: u32,
    group_by_status: bool,
    filter: pingress_wire::access::Filter,
) -> Result<(
    Vec<pingress_wire::access::StatsRow>,
    Option<Vec<pingress_wire::access::StatusRow>>,
)> {
    super::insight::require_server(docker).await?;
    if group_by_status {
        let (rows, status_rows) = tokio::try_join!(
            api.access_stats(range, step_secs, &filter),
            api.access_status_stats(range, step_secs, &filter)
        )?;
        Ok((rows, Some(status_rows)))
    } else {
        Ok((api.access_stats(range, step_secs, &filter).await?, None))
    }
}

pub(crate) async fn snapshot(
    docker: &infra::docker::Docker,
    key: &MasterKey,
) -> Result<pingress_wire::Snapshot> {
    let routings = infra::app::list_domains(docker).await?;
    let platform_domain = routings
        .iter()
        .find(|routing| routing.name() == config::SERVICE_NAME)
        .and_then(|routing| routing.rules().first())
        .map(|rule| rule.route().domain.clone());
    let apps: BTreeMap<String, pingress_wire::AppRoutes> = routings
        .into_iter()
        .map(|routing| {
            let routes = routing
                .rules()
                .iter()
                .filter(|rule| {
                    routing.name() == config::SERVICE_NAME
                        || platform_domain.as_deref() != Some(rule.route().domain.as_str())
                })
                .map(|rule| rule.route().clone())
                .collect();
            (
                routing.name().clone(),
                pingress_wire::AppRoutes {
                    service_id: routing.service_id().clone(),
                    routes,
                },
            )
        })
        .collect();
    let mut certs = BTreeMap::new();
    if let Some(store) = infra::acme::load(docker, key).await? {
        for (domain, cert) in store.get().certs() {
            certs.insert(
                domain.to_string(),
                pingress_wire::CertBundle {
                    cert_chain_pem: cert.pem().cert_chain_pem().clone(),
                    private_key_pem: cert.pem().private_key_pem().clone(),
                },
            );
        }
    }
    let etag = etag(&apps, &certs);
    Ok(pingress_wire::Snapshot { etag, apps, certs })
}

fn etag(
    apps: &BTreeMap<String, pingress_wire::AppRoutes>,
    certs: &BTreeMap<String, pingress_wire::CertBundle>,
) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(serde_json::json!((apps, certs)).to_string()))
}
