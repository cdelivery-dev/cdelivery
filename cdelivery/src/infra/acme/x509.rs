use jiff::Timestamp;
use x509_parser::prelude::FromDer;

use crate::concept;
use crate::error::{CertProblem, Result};

fn dns_name_matches(pattern: &str, domain: &str) -> bool {
    pattern.strip_prefix("*.").map_or_else(
        || pattern.eq_ignore_ascii_case(domain),
        |suffix| {
            domain
                .split_once('.')
                .is_some_and(|(_, rest)| rest.eq_ignore_ascii_case(suffix))
        },
    )
}

pub(crate) fn validate_static_cert(
    domain: &concept::domain::DomainName,
    cert_chain_pem: &str,
    private_key_pem: &str,
) -> Result<()> {
    use rustls_pki_types::pem::PemObject;

    let certs: Vec<rustls_pki_types::CertificateDer> =
        rustls_pki_types::CertificateDer::pem_slice_iter(cert_chain_pem.as_bytes())
            .collect::<std::result::Result<_, _>>()
            .map_err(|_| CertProblem::PemDecode)?;
    let leaf = certs.first().ok_or(CertProblem::ChainEmpty)?;
    let key = rustls_pki_types::PrivateKeyDer::from_pem_slice(private_key_pem.as_bytes())
        .map_err(|_| CertProblem::PemDecode)?;
    let (_, x509) =
        x509_parser::parse_x509_certificate(leaf.as_ref()).map_err(|_| CertProblem::Parse)?;

    if !x509.validity().is_valid() {
        return Err(CertProblem::Expired.into());
    }

    let covered = x509
        .subject_alternative_name()
        .ok()
        .flatten()
        .is_some_and(|ext| {
            ext.value.general_names.iter().any(|name| {
                matches!(name, x509_parser::extensions::GeneralName::DNSName(dns)
                    if dns_name_matches(dns, domain.as_str()))
            })
        });
    if !covered {
        return Err(CertProblem::DomainNotCovered(domain.to_string()).into());
    }

    let signing_key = rustls::crypto::aws_lc_rs::sign::any_supported_type(&key)
        .map_err(|_| CertProblem::KeyUnsupported)?;
    let key_spki = signing_key
        .public_key()
        .ok_or(CertProblem::KeyUnsupported)?;
    let (_, derived_spki) = x509_parser::x509::SubjectPublicKeyInfo::from_der(key_spki.as_ref())
        .map_err(|_| CertProblem::Parse)?;
    if derived_spki.subject_public_key.data != x509.public_key().subject_public_key.data {
        return Err(CertProblem::KeyMismatch.into());
    }
    Ok(())
}

pub(crate) fn parse_leaf(
    private_key_pem: cipher::Secret,
    cert_chain_pem: String,
) -> Result<concept::acme::CertPem> {
    let (_, pem) = x509_parser::pem::parse_x509_pem(cert_chain_pem.as_bytes())
        .map_err(|_| CertProblem::PemDecode)?;
    let (_, leaf) =
        x509_parser::parse_x509_certificate(&pem.contents).map_err(|_| CertProblem::Parse)?;
    let not_before = Timestamp::from_second(leaf.validity().not_before.timestamp())
        .map_err(|_| CertProblem::Parse)?;
    let not_after = Timestamp::from_second(leaf.validity().not_after.timestamp())
        .map_err(|_| CertProblem::Parse)?;
    Ok(concept::acme::CertPem::new(
        private_key_pem,
        cert_chain_pem,
        not_before,
        not_after,
    ))
}

pub(crate) fn ari_cert_id(cert_chain_pem: &str) -> Option<String> {
    let (_, pem) = x509_parser::pem::parse_x509_pem(cert_chain_pem.as_bytes()).ok()?;
    let der = rustls_pki_types::CertificateDer::from(pem.contents);
    let id = instant_acme::CertificateIdentifier::try_from(&der).ok()?;
    Some(serde_json::json!(id).to_string())
}
