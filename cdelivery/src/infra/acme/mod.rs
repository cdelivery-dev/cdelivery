pub(crate) mod client;
mod probe;
pub(crate) mod x509;

use std::collections::{BTreeMap, HashMap};

use cipher::{Domain, MasterKey};

use crate::concept;
use crate::error::{Result, SpecProblem};

const ACME_SECRET: &str = "cdelivery-acme";
const LABEL_ACCOUNT: &str = "dev.cdelivery.acme_account";
const CERT_LABEL_PREFIX: &str = "dev.cdelivery.cert.";

pub(crate) type Challenges = scc::HashMap<String, String>;

pub(crate) async fn load(
    docker: &super::docker::Docker,
    key: &MasterKey,
) -> Result<Option<concept::versioned::Versioned<concept::acme::Acme>>> {
    let Some(secret) = docker.secret_inspect(ACME_SECRET).await? else {
        return Ok(None);
    };
    let cipher = key.cipher_for(Domain::Acme);
    let account = secret
        .spec
        .labels
        .get(LABEL_ACCOUNT)
        .map(|sealed| cipher.open::<concept::acme::Account>(sealed))
        .transpose()?;
    let mut certs = BTreeMap::new();
    for (label, sealed) in &secret.spec.labels {
        let Some(raw) = label.strip_prefix(CERT_LABEL_PREFIX) else {
            continue;
        };
        let domain = raw
            .parse::<concept::domain::DomainName>()
            .map_err(|_| SpecProblem::BadAcmeCertLabel(label.clone()))?;
        if certs
            .insert(domain, cipher.open::<concept::acme::Cert>(sealed)?)
            .is_some()
        {
            return Err(SpecProblem::BadAcmeCertLabel(label.clone()).into());
        }
    }
    Ok(Some(concept::versioned::Versioned::new(
        secret.version.index,
        concept::acme::Acme::hydrate(account, certs),
    )))
}

pub(crate) async fn save(
    docker: &super::docker::Docker,
    key: &MasterKey,
    acme: &concept::versioned::Versioned<concept::acme::Acme>,
) -> Result<()> {
    if !acme.changed() {
        return Ok(());
    }
    let labels = encode(key, acme.get())?;
    if acme.version() == 0 {
        docker
            .secret_create(ACME_SECRET, labels, super::docker::PLACEHOLDER)
            .await?;
        return Ok(());
    }
    docker
        .secret_update_labels(ACME_SECRET, acme.version(), labels)
        .await
}

pub(crate) async fn delete(docker: &super::docker::Docker) -> Result<()> {
    docker.secret_delete(ACME_SECRET).await
}

fn encode(key: &MasterKey, acme: &concept::acme::Acme) -> Result<HashMap<String, String>> {
    let cipher = key.cipher_for(Domain::Acme);
    let mut labels = HashMap::new();
    if let Some(account) = acme.account() {
        labels.insert(LABEL_ACCOUNT.to_owned(), cipher.seal(account)?);
    }
    for (domain, cert) in acme.certs() {
        labels.insert(format!("{CERT_LABEL_PREFIX}{domain}"), cipher.seal(cert)?);
    }
    Ok(labels)
}
