use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Version {
    pub index: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct IdResponse {
    #[serde(rename = "ID")]
    pub id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ServiceCreateResponse {
    #[serde(rename = "ID")]
    pub id: String,
    pub warnings: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdateResponse {
    pub warnings: Option<Vec<String>>,
}

#[skip_serializing_none]
#[derive(Debug, Serialize)]
pub struct RegistryAuth {
    #[serde(rename = "username")]
    pub username: Option<String>,
    #[serde(rename = "password")]
    pub password: Option<String>,
    #[serde(rename = "identitytoken")]
    pub identity_token: Option<String>,
    #[serde(rename = "serveraddress")]
    pub server_address: String,
}

#[derive(Debug, Clone, Default)]
pub struct Filter(BTreeMap<String, Vec<String>>);

impl Filter {
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn add(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.0.entry(key.into()).or_default().push(value.into());
        self
    }

    #[must_use]
    pub fn label(self, label: impl Into<String>) -> Self {
        self.add("label", label)
    }

    #[must_use]
    pub fn name(self, name: impl Into<String>) -> Self {
        self.add("name", name)
    }

    pub(crate) fn to_query(&self) -> Option<String> {
        if self.0.is_empty() {
            None
        } else {
            Some(serde_json::json!(self.0).to_string())
        }
    }
}
