use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub const MAX_SERVICES: usize = 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerSample {
    pub container_started_at: jiff::Timestamp,
    pub observed_at: jiff::Timestamp,
    pub node_uptime_ms: u64,
    pub cpu_usage_usec_total: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSample {
    pub observed_at: jiff::Timestamp,
    pub boot_id: String,
    pub node_uptime_ms: u64,
    pub cpu_cores: u16,
    pub cpu_user_ticks: u64,
    pub cpu_system_ticks: u64,
    pub cpu_idle_ticks: u64,
    pub cpu_iowait_ticks: u64,
    pub cpu_steal_ticks: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub service_ids: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Response {
    pub services: HashMap<String, HashMap<String, ContainerSample>>,
    pub nodes: HashMap<String, NodeSample>,
}
