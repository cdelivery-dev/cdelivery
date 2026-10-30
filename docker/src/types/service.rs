use std::collections::HashMap;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_with::{DefaultOnNull, serde_as, skip_serializing_none};

use super::common::Version;
use super::units::{NanoCpus, NanoDuration};

const SERVICE_DEFAULT_UPDATE_PARALLELISM: u64 = 1;
const SERVICE_DEFAULT_UPDATE_DELAY: NanoDuration = NanoDuration::from_secs(10);
const SERVICE_DEFAULT_UPDATE_MONITOR: NanoDuration = NanoDuration::from_secs(10);
const SERVICE_DEFAULT_UPDATE_MAX_FAILURE_RATIO: f32 = 0.0;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Service {
    #[serde(rename = "ID")]
    pub id: String,
    pub version: Version,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub spec: ServiceSpec,
    pub previous_spec: Option<ServiceSpec>,
    #[serde(rename = "ServiceStatus")]
    pub status: Option<ServiceStatus>,
    pub update_status: Option<ServiceUpdateStatus>,
}

#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ServiceSpec {
    pub name: String,
    #[serde_as(deserialize_as = "DefaultOnNull")]
    #[serde(default)]
    pub labels: HashMap<String, String>,
    pub task_template: TaskTemplate,
    pub mode: ServiceMode,
    pub update_config: Option<UpdateConfig>,
    pub rollback_config: Option<RollbackConfig>,
    pub endpoint_spec: Option<EndpointSpec>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TaskTemplate {
    #[serde(default)]
    pub container_spec: ContainerSpec,
    pub resources: Option<Resources>,
    pub restart_policy: Option<RestartPolicy>,
    pub placement: Option<Placement>,
    pub networks: Option<Vec<NetworkAttachment>>,
    pub log_driver: Option<LogDriver>,
    #[serde(default)]
    pub force_update: u64,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ContainerSpec {
    pub image: String,
    #[serde_as(deserialize_as = "DefaultOnNull")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub labels: HashMap<String, String>,
    pub stop_grace_period: Option<NanoDuration>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<String>,
    pub healthcheck: Option<HealthCheck>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mounts: Vec<Mount>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub configs: Vec<ServiceConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<ServiceSecret>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Mount {
    #[serde(rename = "Type")]
    pub kind: MountType,
    pub target: String,
    pub source: String,
    #[serde(default)]
    pub read_only: bool,
    pub bind_options: Option<BindOptions>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct BindOptions {
    pub propagation: Option<MountPropagation>,
    #[serde(default)]
    pub read_only_force_recursive: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MountPropagation {
    Private,
    Rprivate,
    Shared,
    Rshared,
    Slave,
    Rslave,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MountType {
    Bind,
    Volume,
    Tmpfs,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct HealthCheck {
    #[serde(default)]
    pub test: Vec<String>,
    #[serde(default)]
    pub interval: NanoDuration,
    #[serde(default)]
    pub timeout: NanoDuration,
    #[serde(default)]
    pub retries: u64,
    #[serde(default)]
    pub start_period: NanoDuration,
    pub start_interval: Option<NanoDuration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ServiceConfig {
    pub file: ServiceFileTarget,
    #[serde(rename = "ConfigID")]
    pub config_id: String,
    pub config_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ServiceSecret {
    pub file: ServiceFileTarget,
    #[serde(rename = "SecretID")]
    pub secret_id: String,
    pub secret_name: String,
}

#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ServiceFileTarget {
    pub name: String,
    #[serde(rename = "UID")]
    pub uid: Option<String>,
    #[serde(rename = "GID")]
    pub gid: Option<String>,
    pub mode: Option<u32>,
}

impl Default for ServiceFileTarget {
    fn default() -> Self {
        Self {
            name: String::new(),
            uid: Some("0".to_owned()),
            gid: Some("0".to_owned()),
            mode: Some(0o444),
        }
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Resources {
    pub limits: Option<ResourcesLimits>,
    pub reservations: Option<ResourcesReservations>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ResourcesLimits {
    #[serde(rename = "NanoCPUs")]
    pub nano_cpus: Option<NanoCpus>,
    pub memory_bytes: Option<u64>,
    pub pids: Option<u64>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ResourcesReservations {
    #[serde(rename = "NanoCPUs")]
    pub nano_cpus: Option<NanoCpus>,
    pub memory_bytes: Option<u64>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RestartPolicy {
    pub condition: RestartPolicyCondition,
    pub delay: Option<NanoDuration>,
    pub max_attempts: Option<u64>,
    pub window: Option<NanoDuration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RestartPolicyCondition {
    None,
    OnFailure,
    Any,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Placement {
    pub constraints: Option<Vec<String>>,
    pub max_replicas: Option<u64>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct NetworkAttachment {
    pub target: String,
    pub aliases: Option<Vec<String>>,
    pub driver_opts: Option<HashMap<String, String>>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LogDriver {
    pub name: String,
    pub options: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ServiceMode {
    Global {},
    #[serde(rename_all = "PascalCase")]
    Replicated {
        replicas: u64,
    },
    #[serde(rename_all = "PascalCase")]
    ReplicatedJob {
        max_concurrent: u64,
    },
    GlobalJob {},
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdateConfig {
    pub parallelism: Option<u64>,
    pub delay: Option<NanoDuration>,
    pub failure_action: Option<UpdateFailureAction>,
    pub monitor: Option<NanoDuration>,
    pub max_failure_ratio: Option<f32>,
    pub order: Option<UpdateOrder>,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            parallelism: Some(SERVICE_DEFAULT_UPDATE_PARALLELISM),
            delay: Some(SERVICE_DEFAULT_UPDATE_DELAY),
            failure_action: Some(UpdateFailureAction::Rollback),
            monitor: Some(SERVICE_DEFAULT_UPDATE_MONITOR),
            max_failure_ratio: Some(SERVICE_DEFAULT_UPDATE_MAX_FAILURE_RATIO),
            order: Some(UpdateOrder::StartFirst),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpdateFailureAction {
    Continue,
    Pause,
    Rollback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpdateOrder {
    StopFirst,
    StartFirst,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RollbackConfig {
    pub parallelism: Option<u64>,
    pub delay: Option<NanoDuration>,
    pub failure_action: Option<RollbackFailureAction>,
    pub monitor: Option<NanoDuration>,
    pub max_failure_ratio: Option<f32>,
    pub order: Option<UpdateOrder>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RollbackFailureAction {
    Continue,
    Pause,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EndpointSpec {
    pub mode: EndpointMode,
    pub ports: Option<Vec<EndpointPortConfig>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EndpointMode {
    Vip,
    Dnsrr,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EndpointPortConfig {
    pub protocol: PortProtocol,
    pub target_port: u16,
    pub published_port: Option<u16>,
    pub publish_mode: PortPublishMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PortProtocol {
    Tcp,
    Udp,
    Sctp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PortPublishMode {
    Ingress,
    Host,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ServiceUpdateStatus {
    pub state: ServiceUpdateState,
    pub message: Option<String>,
}

#[derive(Debug, strum::Display, strum::EnumString, serde_with::DeserializeFromStr)]
#[strum(serialize_all = "snake_case")]
pub enum ServiceUpdateState {
    Updating,
    Paused,
    Completed,
    RollbackStarted,
    RollbackPaused,
    RollbackCompleted,
    #[strum(default)]
    Unknown(String),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ServiceStatus {
    #[serde(rename = "RunningTasks")]
    pub running: u64,
    #[serde(rename = "DesiredTasks")]
    pub desired: u64,
}
