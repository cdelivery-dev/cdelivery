use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize, Deserialize)]
pub struct Log {
    pub timestamp: jiff::Timestamp,
    pub source: String,
    pub container_id: String,
    pub node_id: String,
    pub log: String,
}
