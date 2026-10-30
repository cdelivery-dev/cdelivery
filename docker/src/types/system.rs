use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DockerVersion {
    pub api_version: String,
    #[serde(rename = "MinAPIVersion")]
    pub min_api_version: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SystemInfo {
    pub swarm: Option<SwarmInfo>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SwarmInfo {
    #[serde(rename = "NodeID")]
    pub node_id: String,
    #[serde(default)]
    pub remote_managers: Vec<RemoteManager>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RemoteManager {
    #[serde(rename = "NodeID")]
    pub node_id: String,
    pub addr: std::net::SocketAddr,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct InspectSwarm {
    pub join_tokens: JoinTokens,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct JoinTokens {
    pub worker: String,
    pub manager: String,
}

impl std::fmt::Debug for JoinTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JoinTokens").finish_non_exhaustive()
    }
}
