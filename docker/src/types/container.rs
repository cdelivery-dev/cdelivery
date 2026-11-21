use std::collections::HashMap;

use serde::Deserialize;
use serde_with::{DefaultOnNull, serde_as};

#[serde_as]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Container {
    #[serde(rename = "Id")]
    pub id: String,
    #[serde_as(deserialize_as = "DefaultOnNull")]
    #[serde(default)]
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ContainerDetail {
    #[serde(rename = "Id")]
    pub id: String,
    pub image: String,
    pub config: ContainerConfig,
    pub state: ContainerState,
}

#[serde_as]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ContainerConfig {
    #[serde_as(deserialize_as = "DefaultOnNull")]
    #[serde(default)]
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ContainerState {
    pub pid: u32,
    pub started_at: jiff::Timestamp,
}
