use jiff::Timestamp;

use crate::concept;
use crate::config;
use crate::error::{AcmeProblem, CertProblem, ErrorKind, Result};

#[derive(strum::Display)]
enum ChallengeKind {
    Http01,
    DnsPersist01,
}

impl ChallengeKind {
    fn acme_type(&self) -> instant_acme::ChallengeType {
        match self {
            Self::Http01 => instant_acme::ChallengeType::Http01,
            Self::DnsPersist01 => instant_acme::ChallengeType::Unknown("dns-persist-01".to_owned()),
        }
    }
}

pub(crate) async fn register(ca: concept::acme::CA) -> Result<concept::acme::Account> {
    let new_account = instant_acme::NewAccount {
        contact: &[],
        terms_of_service_agreed: true,
        only_return_existing: false,
    };
    let (account, credentials) = builder(&ca)?
        .create(&new_account, ca.directory_url().to_owned(), None)
        .await?;
    concept::acme::Account::new(
        ca,
        cipher::Secret::new(serde_json::json!(credentials).to_string()),
        account.id().to_owned(),
    )
}

pub(crate) async fn renewal_window(
    account: &concept::acme::Account,
    cert_id: &str,
) -> Result<Option<Timestamp>> {
    match tokio::time::timeout(
        config::ACME_ARI_TIMEOUT.unsigned_abs(),
        renewal_window_flow(account, cert_id),
    )
    .await
    {
        Ok(inner) => inner,
        Err(source) => Err(AcmeProblem::Timeout(source).into()),
    }
}

pub(crate) async fn issue(
    account: &concept::acme::Account,
    domain: &concept::domain::DomainName,
    replaces: Option<&str>,
    challenges: &super::Challenges,
) -> Result<Option<(Option<String>, concept::acme::CertPem)>> {
    let mut receipt = None;
    let result = tokio::time::timeout(
        config::ACME_ISSUE_TIMEOUT.unsigned_abs(),
        issue_flow(account, domain, replaces, challenges, &mut receipt),
    )
    .await;
    if let Some(token) = receipt {
        challenges.remove_sync(&token);
    }
    match result {
        Ok(inner) => inner,
        Err(source) => Err(AcmeProblem::Timeout(source).into()),
    }
}

fn builder(ca: &concept::acme::CA) -> Result<instant_acme::AccountBuilder> {
    match ca {
        concept::acme::CA::Pebble { root_ca_pem, .. } => {
            use rustls_pki_types::pem::PemObject;
            let ders = rustls_pki_types::CertificateDer::pem_slice_iter(root_ca_pem.as_bytes())
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|_| CertProblem::PemDecode)?;
            let mut roots = rustls::RootCertStore::empty();
            let (added, _ignored) = roots.add_parsable_certificates(ders);
            if added == 0 {
                return Err(CertProblem::PemDecode.into());
            }
            let tls = rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth();
            let connector = hyper_rustls::HttpsConnectorBuilder::new()
                .with_tls_config(tls)
                .https_only()
                .enable_http1()
                .enable_http2()
                .build();
            let client: hyper_util::client::legacy::Client<
                _,
                instant_acme::BodyWrapper<bytes::Bytes>,
            > = hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
                .build(connector);
            Ok(instant_acme::Account::builder_with_http(Box::new(client)))
        }
        _ => Ok(instant_acme::Account::builder()?),
    }
}

async fn rebuild(account: &concept::acme::Account) -> Result<instant_acme::Account> {
    let credentials: instant_acme::AccountCredentials =
        serde_json::from_str(account.credentials().reveal())?;
    Ok(builder(account.ca())?.from_credentials(credentials).await?)
}

async fn preflight(
    session: &instant_acme::Account,
    domain: &concept::domain::DomainName,
) -> Vec<ChallengeKind> {
    let mut kinds = Vec::new();
    if super::probe::check_dns_persist(session.id(), domain.as_str()).await {
        kinds.push(ChallengeKind::DnsPersist01);
    }
    if super::probe::healthz_ok(domain.as_str()).await {
        kinds.push(ChallengeKind::Http01);
    }
    kinds
}

async fn renewal_window_flow(
    account: &concept::acme::Account,
    cert_id: &str,
) -> Result<Option<Timestamp>> {
    let session = rebuild(account).await?;
    let id = serde_json::from_str::<instant_acme::CertificateIdentifier>(cert_id)?.into_owned();
    match session.renewal_info(&id).await {
        Ok((info, _retry_after)) => {
            let second = info.suggested_window.start.unix_timestamp();
            Ok(Some(Timestamp::from_second(second).map_err(|_| {
                AcmeProblem::RenewalStartOutOfRange(second)
            })?))
        }
        Err(instant_acme::Error::Unsupported(reason)) => {
            tracing::warn!("ACME server does not support ARI: {reason}");
            Ok(None)
        }
        Err(e) => Err(e.into()),
    }
}

async fn issue_flow(
    account: &concept::acme::Account,
    domain: &concept::domain::DomainName,
    replaces: Option<&str>,
    challenges: &super::Challenges,
    receipt: &mut Option<String>,
) -> Result<Option<(Option<String>, concept::acme::CertPem)>> {
    let session = rebuild(account).await?;
    let kinds = preflight(&session, domain).await;
    if kinds.is_empty() {
        return Ok(None);
    }
    let identifiers = [instant_acme::Identifier::Dns(domain.as_str().to_owned())];
    let mut order = match new_order(&session, &identifiers, replaces).await {
        Ok(order) => order,
        Err(e)
            if replaces.is_some()
                && matches!(
                    e.kind(),
                    ErrorKind::Serde(_)
                        | ErrorKind::AcmeClient(instant_acme::Error::Unsupported(_))
                ) =>
        {
            tracing::warn!(
                error = ?e,
                "ACME replaces for {domain} failed; retrying without replaces"
            );
            new_order(&session, &identifiers, None).await?
        }
        Err(e) => return Err(e),
    };
    drive(&mut order, domain, &kinds, challenges, receipt)
        .await
        .map(Some)
}

async fn new_order(
    session: &instant_acme::Account,
    identifiers: &[instant_acme::Identifier],
    replaces: Option<&str>,
) -> Result<instant_acme::Order> {
    let new_order = instant_acme::NewOrder::new(identifiers);
    let new_order = match replaces {
        Some(id) => new_order.replaces(
            serde_json::from_str::<instant_acme::CertificateIdentifier>(id)?.into_owned(),
        ),
        None => new_order,
    };
    Ok(session.new_order(&new_order).await?)
}

async fn drive(
    order: &mut instant_acme::Order,
    domain: &concept::domain::DomainName,
    kinds: &[ChallengeKind],
    challenges: &super::Challenges,
    receipt: &mut Option<String>,
) -> Result<(Option<String>, concept::acme::CertPem)> {
    if order.state().status == instant_acme::OrderStatus::Pending {
        answer(order, domain, kinds, challenges, receipt).await?;
    }
    order
        .poll_ready(
            &instant_acme::RetryPolicy::new()
                .timeout(config::ACME_POLL_READY_TIMEOUT.unsigned_abs()),
        )
        .await?;
    if order.state().status != instant_acme::OrderStatus::Ready {
        return Err(AcmeProblem::OrderNotReady(format!("{:?}", order.state().status)).into());
    }
    let private_key_pem = order.finalize().await?;
    let cert_chain_pem = order
        .poll_certificate(
            &instant_acme::RetryPolicy::new()
                .timeout(config::ACME_POLL_CERT_TIMEOUT.unsigned_abs()),
        )
        .await?;
    Ok((
        super::x509::ari_cert_id(&cert_chain_pem),
        super::x509::parse_leaf(cipher::Secret::new(private_key_pem), cert_chain_pem)?,
    ))
}

async fn answer(
    order: &mut instant_acme::Order,
    domain: &concept::domain::DomainName,
    kinds: &[ChallengeKind],
    challenges: &super::Challenges,
    receipt: &mut Option<String>,
) -> Result<()> {
    let mut authorizations = order.authorizations();
    let Some(authorization) = authorizations.next().await else {
        return Err(AcmeProblem::NoAuthorization(domain.to_string()).into());
    };
    let mut authorization = authorization?;
    if authorization.status == instant_acme::AuthorizationStatus::Valid {
        return Ok(());
    }
    let Some(kind) = kinds.iter().find(|kind| {
        let offered = authorization
            .challenges
            .iter()
            .any(|challenge| challenge.r#type == kind.acme_type());
        if !offered {
            tracing::warn!("ACME challenge {kind} not offered for {domain}");
        }
        offered
    }) else {
        let wanted = kinds
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(AcmeProblem::ChallengeNotOffered(wanted).into());
    };
    let mut challenge = authorization
        .challenge(kind.acme_type())
        .ok_or_else(|| AcmeProblem::ChallengeNotOffered(kind.to_string()))?;
    if let ChallengeKind::Http01 = kind {
        let token = challenge.token.clone();
        let key_authorization = challenge.key_authorization().as_str().to_owned();
        let _ = challenges.upsert_sync(token.clone(), key_authorization);
        *receipt = Some(token);
    }
    challenge.set_ready().await?;
    Ok(())
}
