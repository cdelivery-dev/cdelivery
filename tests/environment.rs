use std::collections::BTreeMap;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Environment {
    pub(crate) profile: String,
    pub(crate) owner: String,
    pub(crate) machines: Vec<String>,
    pub(crate) addresses: BTreeMap<String, String>,
    pub(crate) images: BTreeMap<String, Image>,
    #[serde(default)]
    pub(crate) upgrades: BTreeMap<String, Image>,
    #[serde(default)]
    pub(crate) failures: BTreeMap<String, Image>,
    pub(crate) base_url: String,
    pub(crate) support_ip: String,
    pub(crate) registry: String,
    pub(crate) ready: bool,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Image {
    pub(crate) reference: String,
    pub(crate) version: String,
    pub(crate) digest: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Credentials {
    pub(crate) email: String,
    pub(crate) password: String,
    pub(crate) api_key: String,
}
