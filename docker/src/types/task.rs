use jiff::Timestamp;
use serde::Deserialize;

use super::common::Version;
use super::service::TaskTemplate;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Task {
    pub version: Version,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub spec: TaskTemplate,
    #[serde(rename = "ServiceID")]
    pub service_id: String,
    #[serde(default, rename = "NodeID")]
    pub node_id: String,
    #[serde(default)]
    pub networks_attachments: Vec<TaskNetwork>,
    pub desired_state: TaskState,
    pub status: TaskStatus,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TaskNetwork {
    #[serde(default)]
    pub addresses: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TaskStatus {
    pub timestamp: Timestamp,
    pub state: TaskState,
    pub message: Option<String>,
    pub err: Option<String>,
    pub container_status: Option<ContainerStatus>,
}

#[derive(Debug, Deserialize)]
pub struct ContainerStatus {
    #[serde(default, rename = "ContainerID")]
    pub container_id: String,
}

impl TaskStatus {
    pub fn error(&self) -> Option<&str> {
        self.err.as_deref().filter(|error| !error.trim().is_empty())
    }

    pub fn diagnostic(&self) -> String {
        [self.err.as_deref(), self.message.as_deref()]
            .into_iter()
            .flatten()
            .find(|text| !text.trim().is_empty())
            .map_or_else(|| self.state.to_string(), str::to_owned)
    }
}

#[derive(
    Debug, PartialEq, Eq, strum::Display, strum::EnumString, serde_with::DeserializeFromStr,
)]
#[strum(serialize_all = "kebab-case")]
pub enum TaskState {
    New,
    Allocated,
    Pending,
    Assigned,
    Accepted,
    Preparing,
    Ready,
    Starting,
    Running,
    Complete,
    Shutdown,
    Failed,
    Rejected,
    Remove,
    Orphaned,
    #[strum(default)]
    Unknown(String),
}
