use std::collections::BTreeMap;
use std::num::NonZeroU16;

use serde::{Deserialize, Serialize};

pub mod access;

pub const CONFIG_PATH: &str = "/pingress/config";
pub const CHALLENGE_PATH_PREFIX: &str = "/pingress/challenge/";
pub const CONFIG_API_PORT: u16 = 9001;
pub const KEY_SECRET_NAME: &str = "cdelivery-pingress";
pub const HTTP01_PATH_PREFIX: &str = "/.well-known/acme-challenge/";
pub const HTTP01_HEALTHZ_PATH: &str = "/.well-known/acme-challenge/healthz";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub domain: String,
    pub https: bool,
    pub target: NonZeroU16,
    pub path_prefix: Option<String>,
    pub header: Option<MatchKV>,
    pub cookie: Option<MatchKV>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchKV {
    pub key: String,
    pub value: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CertBundle {
    pub cert_chain_pem: String,
    pub private_key_pem: cipher::Secret,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AppRoutes {
    pub service_id: String,
    pub routes: Vec<Route>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub etag: String,
    pub apps: BTreeMap<String, AppRoutes>,
    pub certs: BTreeMap<String, CertBundle>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Envelope {
    pub nonce: String,
    pub snapshot: Snapshot,
}
