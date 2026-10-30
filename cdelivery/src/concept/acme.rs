use std::collections::{BTreeMap, BTreeSet};

use getset::Getters;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::error::{AcmeProblem, ErrorKind, Result};

pub(crate) const MAX_DOMAINS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum CA {
    LetsEncrypt,
    LetsEncryptStaging,
    Pebble { url: String, root_ca_pem: String },
}

impl CA {
    pub(crate) fn directory_url(&self) -> &str {
        match self {
            Self::LetsEncrypt => "https://acme-v02.api.letsencrypt.org/directory",
            Self::LetsEncryptStaging => "https://acme-staging-v02.api.letsencrypt.org/directory",
            Self::Pebble { url, .. } => url,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters)]
#[getset(get = "pub(crate)")]
pub(crate) struct Account {
    ca: CA,
    credentials: cipher::Secret,
    uri: String,
}

impl Account {
    pub(crate) fn new(ca: CA, credentials: cipher::Secret, uri: String) -> Result<Self> {
        if !url::Url::parse(&uri)
            .is_ok_and(|parsed| parsed.scheme() == "https" && parsed.has_host())
        {
            return Err(AcmeProblem::AccountUriInvalid(uri).into());
        }
        Ok(Self {
            ca,
            credentials,
            uri,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, macros_derive::Ctor)]
#[getset(get = "pub(crate)")]
pub(crate) struct CertPem {
    private_key_pem: cipher::Secret,
    cert_chain_pem: String,
    not_before: Timestamp,
    not_after: Timestamp,
}

impl CertPem {
    pub(crate) fn renew_due(&self, ari_start: Option<Timestamp>, now: Timestamp) -> bool {
        if let Some(start) = ari_start {
            return now >= start;
        }
        let total = self.not_after.duration_since(self.not_before).as_secs();
        let renew_at = self.not_after.as_second().saturating_sub(total / 3);
        now.as_second() >= renew_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Cert {
    Static(CertPem),
    Issued {
        cert_id: Option<String>,
        pem: CertPem,
    },
}

impl Cert {
    pub(crate) fn pem(&self) -> &CertPem {
        let (Self::Static(pem) | Self::Issued { pem, .. }) = self;
        pem
    }

    pub(crate) fn is_static(&self) -> bool {
        matches!(self, Self::Static(_))
    }

    pub(crate) fn expired(&self, now: Timestamp) -> bool {
        now >= *self.pem().not_after()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Getters, macros_derive::Ctor)]
#[ctor(hydrate)]
pub(crate) struct Acme {
    #[getset(get = "pub(crate)")]
    account: Option<Account>,
    #[getset(get = "pub(crate)")]
    certs: BTreeMap<super::domain::DomainName, Cert>,
}

impl Acme {
    pub(crate) fn enable(&mut self, account: Account) -> Result<()> {
        match &self.account {
            Some(existing) if existing.ca != account.ca => Err(ErrorKind::AcmeCaMismatch.into()),
            Some(_) => Ok(()),
            None => {
                self.account = Some(account);
                Ok(())
            }
        }
    }

    pub(crate) fn disable(&mut self) {
        self.account = None;
        self.certs.retain(|_, cert| cert.is_static());
    }

    pub(crate) fn enabled(&self) -> bool {
        self.account.is_some()
    }

    pub(crate) fn upload_static(
        &mut self,
        domain: super::domain::DomainName,
        pem: CertPem,
    ) -> Result<()> {
        self.check_capacity(&domain)?;
        self.certs.insert(domain, Cert::Static(pem));
        Ok(())
    }

    pub(crate) fn store_issued(
        &mut self,
        domain: super::domain::DomainName,
        cert_id: Option<String>,
        pem: CertPem,
    ) -> Result<bool> {
        if self.certs.get(&domain).is_some_and(Cert::is_static) {
            return Ok(false);
        }
        self.check_capacity(&domain)?;
        self.certs.insert(domain, Cert::Issued { cert_id, pem });
        Ok(true)
    }

    pub(crate) fn remove_static(&mut self, domain: &super::domain::DomainName) -> bool {
        matches!(self.certs.get(domain), Some(Cert::Static(_))) && self.remove_cert(domain)
    }

    pub(crate) fn remove_cert(&mut self, domain: &super::domain::DomainName) -> bool {
        self.certs.remove(domain).is_some()
    }

    pub(crate) fn eviction_candidates(
        &self,
        desired: &BTreeSet<super::domain::DomainName>,
    ) -> Vec<super::domain::DomainName> {
        let missing = desired
            .iter()
            .filter(|domain| !self.certs.contains_key(*domain))
            .count();
        let free = MAX_DOMAINS.saturating_sub(self.certs.len());
        let shortfall = missing.saturating_sub(free);
        self.certs
            .iter()
            .filter(|(domain, cert)| !cert.is_static() && !desired.contains(*domain))
            .map(|(domain, _)| domain.clone())
            .take(shortfall)
            .collect()
    }

    fn check_capacity(&self, domain: &super::domain::DomainName) -> Result<()> {
        if !self.certs.contains_key(domain) && self.certs.len() >= MAX_DOMAINS {
            return Err(ErrorKind::DomainLimit {
                limit: MAX_DOMAINS,
                current: self.certs.len(),
            }
            .into());
        }
        Ok(())
    }
}

impl super::versioned::Aggregate for Acme {}
