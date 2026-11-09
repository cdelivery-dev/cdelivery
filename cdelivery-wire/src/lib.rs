pub mod name;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

pub const PORT: u16 = 5011;
pub const NODE_ID_ENV: &str = "CDELIVERY_NODE_ID";
pub const VOLUME_PREFIX: &str = "cdelivery-";
pub const DISK_PATH: &str = "/volume";
pub const KEY_SECRET_NAME: &str = "cdelivery-agent";
pub const ZFS_ENABLED_ENV: &str = "CDELIVERY_ZFS_ENABLED";
pub const ZFS_POOL: &str = "cdelivery";
pub const GUARDIAN_VOLUME_PREFIX: &str = "cdelivery-guardian.";

pub fn guardian_volume(app: &str, volume: &str) -> String {
    format!("{GUARDIAN_VOLUME_PREFIX}{app}.{volume}")
}

pub fn zfs_options(dataset: &str) -> HashMap<String, String> {
    HashMap::from([
        ("type".to_owned(), "zfs".to_owned()),
        ("device".to_owned(), dataset.to_owned()),
    ])
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub node_id: String,
    pub command: Command,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Command {
    List { sizes: bool },
    Create { name: String },
    Delete { name: String },
    PrepareGuardian { app: String, names: Vec<String> },
    DeleteGuardian { app: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Backend {
    Local,
    Zfs,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Volume {
    pub size_bytes: Option<u64>,
    pub backend: Option<Backend>,
    pub ready: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Report {
    pub volumes: BTreeMap<String, Volume>,
    pub guardians: BTreeSet<String>,
    pub zfs_enabled: bool,
    pub total_bytes: u64,
    pub available_bytes: u64,
}
