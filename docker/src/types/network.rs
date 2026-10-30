use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Network {
    pub id: String,
    pub name: String,
    pub driver: NetworkDriver,
}

#[derive(Debug, strum::EnumString, serde_with::DeserializeFromStr)]
#[strum(serialize_all = "kebab-case")]
pub enum NetworkDriver {
    Bridge,
    Overlay,
    Host,
    Null,
    #[strum(default)]
    Unknown(String),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct NetworkCreateResponse {
    #[serde(rename = "Id")]
    pub id: String,
    #[serde(rename = "Warning", default)]
    pub warning: String,
}
