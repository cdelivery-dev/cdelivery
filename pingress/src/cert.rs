use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, LazyLock};

use arc_swap::ArcSwap;
use pingora::tls::ext;
use pingora::tls::pkey::{PKey, Private};
use pingora::tls::x509::X509;

use crate::error::{ErrorKind, Result};

#[derive(Debug)]
pub(crate) struct TlsBundle {
    leaf: X509,
    chain: Vec<X509>,
    private_key: PKey<Private>,
}

impl TryFrom<&pingress_wire::CertBundle> for TlsBundle {
    type Error = crate::error::Error;

    fn try_from(bundle: &pingress_wire::CertBundle) -> Result<Self> {
        let mut certs = X509::stack_from_pem(bundle.cert_chain_pem.as_bytes())?.into_iter();
        let leaf = certs.next().ok_or(ErrorKind::EmptyCertChain)?;
        let private_key = PKey::private_key_from_pem(bundle.private_key_pem.reveal().as_bytes())?;
        if !private_key.public_eq(&*leaf.public_key()?) {
            return Err(ErrorKind::CertKeyMismatch.into());
        }
        Ok(Self {
            leaf,
            chain: certs.collect(),
            private_key,
        })
    }
}

static DOMAIN_TLS: LazyLock<ArcSwap<HashMap<String, TlsBundle>>> =
    LazyLock::new(|| ArcSwap::from_pointee(HashMap::new()));

pub(crate) fn apply(certs: &BTreeMap<String, pingress_wire::CertBundle>) {
    let mut table = HashMap::new();
    for (domain, bundle) in certs {
        match TlsBundle::try_from(bundle) {
            Ok(tls) => {
                table.insert(domain.clone(), tls);
            }
            Err(error) => {
                tracing::error!(error = ?error, "failed to build TLS bundle for {domain}; skipping");
            }
        }
    }
    DOMAIN_TLS.store(Arc::new(table));
}

pub(crate) fn has_cert(domain: &str) -> bool {
    DOMAIN_TLS.load().contains_key(domain)
}

pub(crate) struct DynamicCert;

#[async_trait::async_trait]
impl pingora::listeners::TlsAccept for DynamicCert {
    async fn certificate_callback(&self, ssl: &mut pingora::tls::ssl::SslRef) {
        let sni = ssl
            .servername(pingora::tls::ssl::NameType::HOST_NAME)
            .and_then(crate::access::Host::parse);
        let table = DOMAIN_TLS.load();
        let Some(tls) = sni.as_ref().and_then(|domain| table.get(domain.as_str())) else {
            return;
        };
        ext::ssl_use_certificate(ssl, &tls.leaf)
            .inspect_err(|e| tracing::error!(error = ?e, "failed to install leaf certificate"))
            .ok();
        ext::ssl_use_private_key(ssl, &tls.private_key)
            .inspect_err(|e| tracing::error!(error = ?e, "failed to install private key"))
            .ok();
        for cert in &tls.chain {
            ext::ssl_add_chain_cert(ssl, cert)
                .inspect_err(|e| tracing::error!(error = ?e, "failed to add chain certificate"))
                .ok();
        }
    }
}
