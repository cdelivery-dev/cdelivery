use std::collections::BTreeMap;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Environment {
    pub(crate) profile: String,
    pub(crate) owner: String,
    pub(crate) machines: Vec<String>,
    pub(crate) addresses: BTreeMap<String, String>,
    pub(crate) images: BTreeMap<String, String>,
    pub(crate) versions: BTreeMap<String, String>,
    pub(crate) digests: BTreeMap<String, String>,
    pub(crate) base_url: String,
    pub(crate) support_ip: String,
    pub(crate) registry: String,
    pub(crate) ready: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Credentials {
    pub(crate) email: String,
    pub(crate) password: String,
    pub(crate) api_key: String,
}
