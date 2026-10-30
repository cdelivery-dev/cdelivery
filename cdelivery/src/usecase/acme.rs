use std::collections::{BTreeMap, BTreeSet};

use cipher::MasterKey;
use insight_wire::event::Kind;
use jiff::{SignedDuration, Timestamp};

use crate::concept;
use crate::error::{ErrorKind, Result};
use crate::infra;

pub(super) async fn check_capacity(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    exclude_service: Option<&str>,
    additions: impl IntoIterator<Item = concept::domain::DomainName>,
) -> Result<()> {
    let mut union: BTreeSet<concept::domain::DomainName> = BTreeSet::new();
    for routing in infra::app::list_domains(docker).await? {
        if exclude_service == Some(routing.name().as_str()) {
            continue;
        }
        union.extend(
            routing
                .rules()
                .iter()
                .filter(|rule| rule.https())
                .map(concept::domain::Rule::domain),
        );
    }
    if let Some(store) = infra::acme::load(docker, key).await? {
        union.extend(store.get().certs().keys().cloned());
    }
    let current = union.len();
    union.extend(additions);
    if union.len() > concept::acme::MAX_DOMAINS {
        return Err(ErrorKind::DomainLimit {
            limit: concept::acme::MAX_DOMAINS,
            current,
        }
        .into());
    }
    Ok(())
}

async fn load_or_default(
    docker: &infra::docker::Docker,
    key: &MasterKey,
) -> Result<concept::versioned::Versioned<concept::acme::Acme>> {
    Ok(infra::acme::load(docker, key)
        .await?
        .unwrap_or_else(|| concept::versioned::Versioned::new(0, concept::acme::Acme::default())))
}

pub(crate) async fn enable_account(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    ca: concept::acme::CA,
) -> Result<(String, bool)> {
    super::pingress::require_enabled(docker).await?;
    let mut store = load_or_default(docker, key).await?;
    if let Some(account) = store.get().account() {
        if *account.ca() == ca {
            return Ok((account.uri().clone(), false));
        }
        return Err(ErrorKind::AcmeCaMismatch.into());
    }
    let account = infra::acme::client::register(ca).await?;
    let uri = account.uri().clone();
    let note = format!("{:?}", account.ca());
    store.get_mut().enable(account)?;
    infra::acme::save(docker, key, &store).await?;
    infra::event::emit(Kind::AcmeEnabled, "account", &note);
    Ok((uri, true))
}

pub(crate) async fn disable_account(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    challenges: &infra::acme::Challenges,
) -> Result<()> {
    let Some(mut store) = infra::acme::load(docker, key).await? else {
        challenges.clear_sync();
        clear_issue_failures();
        return Ok(());
    };
    if account_disabled(store.get()) {
        challenges.clear_sync();
        clear_issue_failures();
        return Ok(());
    }
    store.get_mut().disable();
    infra::acme::save(docker, key, &store).await?;
    challenges.clear_sync();
    infra::event::emit(Kind::AcmeDisabled, "account", "");
    clear_issue_failures();
    Ok(())
}

pub(super) fn clear_issue_failures() {
    infra::event::HUB.retain(Kind::CertIssueFailed, &std::collections::HashSet::new());
}

fn account_disabled(store: &concept::acme::Acme) -> bool {
    !store.enabled() && store.certs().values().all(concept::acme::Cert::is_static)
}

pub(crate) async fn upload_cert(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    domain: concept::domain::DomainName,
    cert_chain_pem: String,
    private_key_pem: cipher::Secret,
) -> Result<()> {
    super::pingress::require_enabled(docker).await?;
    infra::acme::x509::validate_static_cert(&domain, &cert_chain_pem, private_key_pem.reveal())?;
    let pem = infra::acme::x509::parse_leaf(private_key_pem, cert_chain_pem)?;
    check_capacity(docker, key, None, [domain.clone()]).await?;
    let mut store = load_or_default(docker, key).await?;
    store.get_mut().upload_static(domain.clone(), pem)?;
    infra::acme::save(docker, key, &store).await?;
    infra::event::HUB.dismiss(Kind::CertIssueFailed, &domain.to_string());
    infra::event::emit(Kind::CertUploaded, &domain.to_string(), "");
    Ok(())
}

pub(crate) async fn remove_cert(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    domain: &concept::domain::DomainName,
) -> Result<()> {
    let Some(mut store) = infra::acme::load(docker, key).await? else {
        return Ok(());
    };
    if !store.get_mut().remove_static(domain) {
        return Ok(());
    }
    infra::acme::save(docker, key, &store).await?;
    infra::event::emit(Kind::CertRemoved, &domain.to_string(), "");
    Ok(())
}

pub(crate) async fn list_certs(
    docker: &infra::docker::Docker,
    key: &MasterKey,
) -> Result<
    Vec<(
        concept::domain::DomainName,
        Vec<String>,
        Option<concept::acme::Cert>,
    )>,
> {
    let mut inventory: BTreeMap<
        concept::domain::DomainName,
        (BTreeSet<String>, Option<concept::acme::Cert>),
    > = BTreeMap::new();
    for routing in infra::app::list_domains(docker).await? {
        for rule in routing.rules().iter().filter(|rule| rule.https()) {
            inventory
                .entry(rule.domain())
                .or_default()
                .0
                .insert(routing.name().clone());
        }
    }
    if let Some(store) = infra::acme::load(docker, key).await? {
        for (domain, cert) in store.get().certs() {
            inventory.entry(domain.clone()).or_default().1 = Some(cert.clone());
        }
    }
    Ok(inventory
        .into_iter()
        .map(|(domain, (apps, cert))| (domain, apps.into_iter().collect(), cert))
        .collect())
}

async fn desired_domains(
    docker: &infra::docker::Docker,
) -> Result<BTreeSet<concept::domain::DomainName>> {
    let routings = infra::app::list_domains(docker).await?;
    Ok(routings
        .iter()
        .flat_map(|routing| routing.rules().iter())
        .filter(|rule| rule.https())
        .map(concept::domain::Rule::domain)
        .collect())
}

async fn store_issued(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    domain: &concept::domain::DomainName,
    cert_id: Option<String>,
    pem: concept::acme::CertPem,
) -> Result<bool> {
    let Some(mut store) = infra::acme::load(docker, key).await? else {
        return Ok(false);
    };
    if !store.get().enabled() {
        return Ok(false);
    }
    if !store.get_mut().store_issued(domain.clone(), cert_id, pem)? {
        return Ok(false);
    }
    infra::acme::save(docker, key, &store).await?;
    Ok(true)
}

pub(crate) async fn issue_missing(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    challenges: &infra::acme::Challenges,
) -> Result<usize> {
    let Some(mut store) = infra::acme::load(docker, key).await? else {
        return Ok(0);
    };
    for (domain, cert) in store.get().certs() {
        if cert.is_static() {
            infra::event::HUB.dismiss(Kind::CertIssueFailed, &domain.to_string());
        }
    }
    let Some(account) = store.get().account().clone() else {
        return Ok(0);
    };
    let desired = desired_domains(docker).await?;

    let evictions = store.get().eviction_candidates(&desired);
    if !evictions.is_empty() {
        for domain in &evictions {
            tracing::warn!("evicting orphaned ACME cert for {domain} under capacity pressure");
            store.get_mut().remove_cert(domain);
        }
        infra::acme::save(docker, key, &store).await?;
        store = super::require(infra::acme::load(docker, key), ErrorKind::Conflict).await?;
    }

    let mut issued = 0;
    let mut keep: std::collections::HashSet<String> = store
        .get()
        .certs()
        .keys()
        .map(ToString::to_string)
        .collect();
    keep.extend(desired.iter().map(ToString::to_string));
    for domain in desired {
        if store.get().certs().contains_key(&domain) {
            continue;
        }
        issued += issue_one(docker, key, &account, &domain, None, challenges).await;
    }
    infra::event::HUB.retain(Kind::CertIssueFailed, &keep);
    Ok(issued)
}

async fn issue_one(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    account: &concept::acme::Account,
    domain: &concept::domain::DomainName,
    replaces: Option<&str>,
    challenges: &infra::acme::Challenges,
) -> usize {
    let (action, kind) = if replaces.is_some() {
        ("renewing", Kind::CertRenewed)
    } else {
        ("issuing", Kind::CertIssued)
    };
    let outcome: Result<bool> = async {
        let Some((cert_id, pem)) =
            infra::acme::client::issue(account, domain, replaces, challenges).await?
        else {
            tracing::warn!("no viable ACME challenge for {domain}; skipping this tick");
            infra::event::HUB.raise(
                Kind::CertIssueFailed,
                &domain.to_string(),
                "No viable ACME challenge",
                Some(SignedDuration::ZERO),
            );
            return Ok(false);
        };
        if !store_issued(docker, key, domain, cert_id, pem).await? {
            return Ok(false);
        }
        let subject = domain.to_string();
        if !infra::event::HUB.clear(Kind::CertIssueFailed, &subject) {
            infra::event::emit(kind, &subject, "");
        }
        Ok(true)
    }
    .await;
    match outcome {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(error) => {
            tracing::warn!(error = ?error, "{action} cert for {domain} failed; continuing");
            infra::event::HUB.raise(
                Kind::CertIssueFailed,
                &domain.to_string(),
                &error.chain(crate::locale::Locale::En),
                Some(SignedDuration::ZERO),
            );
            0
        }
    }
}

pub(crate) async fn renew_due(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    challenges: &infra::acme::Challenges,
    now: Timestamp,
    errors: &mut Vec<(String, String)>,
) -> Result<usize> {
    let Some(store) = infra::acme::load(docker, key).await? else {
        return Ok(0);
    };
    for (domain, cert) in store.get().certs() {
        if cert.is_static() {
            infra::event::HUB.dismiss(Kind::CertIssueFailed, &domain.to_string());
            if cert.expired(now) {
                tracing::warn!("static cert for {domain} has expired");
                errors.push((
                    domain.to_string(),
                    "Static certificate has expired".to_owned(),
                ));
            }
        }
    }
    let Some(account) = store.get().account().clone() else {
        return Ok(0);
    };
    let desired = desired_domains(docker).await?;

    let mut renewed = 0;
    for (domain, cert) in store.get().certs() {
        let concept::acme::Cert::Issued { cert_id, pem } = cert else {
            continue;
        };
        let window = match cert_id {
            Some(id) => match infra::acme::client::renewal_window(&account, id).await {
                Ok(window) => window,
                Err(error) => {
                    tracing::warn!(error = ?error, "ARI query for {domain} failed; using validity rule");
                    errors.push((
                        format!("acme/ari/{domain}"),
                        error.chain(crate::locale::Locale::En),
                    ));
                    None
                }
            },
            None => None,
        };
        if !pem.renew_due(window, now) {
            continue;
        }
        if desired.contains(domain) {
            renewed += issue_one(
                docker,
                key,
                &account,
                domain,
                cert_id.as_deref(),
                challenges,
            )
            .await;
        } else if let Some(mut fresh) = infra::acme::load(docker, key).await? {
            tracing::warn!("dropping ACME cert for undeclared domain {domain}");
            fresh.get_mut().remove_cert(domain);
            if let Err(error) = infra::acme::save(docker, key, &fresh).await {
                tracing::warn!(error = ?error, "dropping cert for {domain} failed; continuing");
                errors.push((
                    format!("acme/prune/{domain}"),
                    error.chain(crate::locale::Locale::En),
                ));
            }
        }
    }
    Ok(renewed)
}
