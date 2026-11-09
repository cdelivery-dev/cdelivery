use serde::Deserialize;
use serde_with::{DefaultOnNull, serde_as};

#[serde_as]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Volume {
    pub name: String,
    pub driver: String,
    #[serde_as(deserialize_as = "DefaultOnNull")]
    #[serde(default)]
    pub options: std::collections::HashMap<String, String>,
    pub usage_data: Option<VolumeUsage>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct VolumeUsage {
    pub size: i64,
}
