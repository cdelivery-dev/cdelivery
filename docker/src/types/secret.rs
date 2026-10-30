use std::collections::HashMap;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_with::{DefaultOnNull, serde_as, skip_serializing_none};

use super::common::Version;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Secret {
    #[serde(rename = "ID")]
    pub id: String,
    pub version: Version,
    pub created_at: Timestamp,
    pub spec: SecretSpec,
}

#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SecretSpec {
    pub name: String,
    #[serde_as(deserialize_as = "DefaultOnNull")]
    #[serde(default)]
    pub labels: HashMap<String, String>,
    pub data: Option<String>,
}
