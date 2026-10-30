pub mod name;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const PORT: u16 = 5011;
pub const NODE_ID_ENV: &str = "CDELIVERY_NODE_ID";
pub const VOLUME_PREFIX: &str = "cdelivery-";
pub const DISK_PATH: &str = "/volume";
pub const KEY_SECRET_NAME: &str = "cdelivery-agent";

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
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Report {
    pub volumes: BTreeMap<String, Option<u64>>,
    pub total_bytes: u64,
    pub available_bytes: u64,
}
