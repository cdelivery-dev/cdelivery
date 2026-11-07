use std::collections::HashMap;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_with::{DefaultOnNull, serde_as, skip_serializing_none};

use super::common::Version;
use super::units::NanoCpus;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Node {
    #[serde(rename = "ID")]
    pub id: String,
    pub version: Version,
    pub spec: NodeSpec,
    pub description: NodeDescription,
    pub status: NodeStatus,
    pub manager_status: Option<NodeManagerStatus>,
    pub created_at: Timestamp,
}

#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct NodeSpec {
    #[serde(default)]
    pub name: String,
    #[serde_as(deserialize_as = "DefaultOnNull")]
    #[serde(default)]
    pub labels: HashMap<String, String>,
    pub role: NodeRole,
    pub availability: NodeAvailability,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeRole {
    Worker,
    Manager,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeAvailability {
    Active,
    Pause,
    Drain,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct NodeDescription {
    #[serde(default)]
    pub hostname: String,
    pub resources: NodeResources,
    pub engine: EngineDescription,
    pub platform: Option<Platform>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Platform {
    pub architecture: Option<String>,
    #[serde(rename = "OS")]
    pub os: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct NodeResources {
    #[serde(rename = "NanoCPUs")]
    pub nano_cpus: NanoCpus,
    pub memory_bytes: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EngineDescription {
    #[serde(default)]
    pub engine_version: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct NodeStatus {
    pub state: NodeState,
    #[serde(default)]
    pub addr: String,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, strum::EnumString, serde_with::DeserializeFromStr)]
#[strum(serialize_all = "kebab-case")]
pub enum NodeState {
    Ready,
    Down,
    Disconnected,
    #[strum(default)]
    Unknown(String),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct NodeManagerStatus {
    pub reachability: NodeReachability,
    #[serde(default)]
    pub leader: bool,
}

#[derive(Debug, strum::EnumString, serde_with::DeserializeFromStr)]
#[strum(serialize_all = "kebab-case")]
pub enum NodeReachability {
    Unreachable,
    Reachable,
    #[strum(default)]
    Unknown(String),
}
