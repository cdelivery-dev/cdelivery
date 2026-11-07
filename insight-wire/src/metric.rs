use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const SAMPLE_INTERVAL_MIN_SECS: u16 = 1;
pub const SAMPLE_INTERVAL_MAX_SECS: u16 = 60;
pub const FILESYSTEM_SAMPLE_INTERVAL_SECS: u16 = 300;
pub const STEP_SECS_MAX: u32 = 86_400;
pub const MAX_BUCKETS: u16 = 720;

#[derive(Debug, Deserialize)]
pub struct NodeReport {
    pub node_id: String,
    pub observed_at: jiff::Timestamp,
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum OverviewTarget {
    Service {
        service_id: String,
        name: String,
    },
    Rest,
    Host,
    Node {
        node_id: String,
    },
    Filesystem {
        node_id: String,
        mount_point: String,
    },
    Cluster,
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize, Deserialize)]
pub struct OverviewPoint {
    pub timestamp: jiff::Timestamp,
    pub value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<f64>,
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize, Deserialize)]
pub struct OverviewSeries {
    pub metric: String,
    pub target: OverviewTarget,
    pub points: Vec<OverviewPoint>,
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Stat {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize, Deserialize)]
pub struct HostBucket {
    pub timestamp: jiff::Timestamp,
    #[serde(flatten)]
    pub series: BTreeMap<String, Stat>,
}

macro_rules! filesystem_bucket {
    ($($family:ident),+) => {
        #[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub struct FilesystemBucket {
            pub timestamp: jiff::Timestamp,
            pub mount_point: String,
            $(
                #[serde(default)]
                pub $family: Stat,
            )+
        }

        impl FilesystemBucket {
            pub const FAMILIES: [&str; [$(stringify!($family)),+].len()] =
                [$(stringify!($family)),+];
        }
    };
}

filesystem_bucket!(
    utilization,
    used_bytes,
    available_bytes,
    total_bytes,
    inodes_utilization
);

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize, Deserialize)]
pub struct ContainerBucket {
    pub timestamp: jiff::Timestamp,
    pub replicas: u32,
    #[serde(flatten)]
    pub series: BTreeMap<String, Stat>,
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize, Deserialize)]
pub struct ContainerInstance {
    pub container_id: String,
    pub node_id: String,
    pub container_started_at: jiff::Timestamp,
}
