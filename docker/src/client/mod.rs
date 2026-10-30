mod config;
mod container;
mod network;
mod node;
mod secret;
mod service;
mod system;
mod task;
mod volume;

pub use config::Configs;
pub use container::Containers;
pub use network::Networks;
pub use node::Nodes;
pub use secret::Secrets;
pub use service::Services;
pub use system::System;
pub use task::Tasks;
pub use volume::Volumes;

use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::URL_SAFE};
use bytes::Bytes;
use http::Method;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::{Error, ErrorKind, Result};
use crate::types::RegistryAuth;

pub const MAX_CONFIG_DATA_SIZE: usize = 1000 * 1024 - 1;

const DEFAULT_API_VERSION: &str = "v1.48";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_SECRET_DATA_SIZE: usize = 500 * 1024 - 1;

const VERSION_PATH: &str = "/version";
const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

#[derive(Debug)]
pub struct Client {
    api_version: String,
    http: reqwest::Client,
}

impl Client {
    pub async fn connect(socket: impl Into<String>) -> Result<Self> {
        let requested = semver::Version::parse(&normalize_api_version(DEFAULT_API_VERSION))?;
        let http = build_http(&socket.into(), DEFAULT_TIMEOUT)?;
        let client = Self {
            api_version: canonical_api_version(&requested),
            http,
        };
        let version = client.system().version().await?;
        let minimum = semver::Version::parse(&normalize_api_version(&version.min_api_version))?;
        if requested < minimum {
            return Err(Error::new(ErrorKind::VersionTooLow {
                requested: requested.to_string(),
                minimum: minimum.to_string(),
            }));
        }
        let maximum = semver::Version::parse(&normalize_api_version(&version.api_version))?;
        if requested > maximum {
            return Err(Error::new(ErrorKind::VersionTooHigh {
                requested: requested.to_string(),
                maximum: maximum.to_string(),
            }));
        }
        Ok(client)
    }

    pub fn system(&self) -> System<'_> {
        System(self)
    }

    pub fn nodes(&self) -> Nodes<'_> {
        Nodes(self)
    }

    pub fn networks(&self) -> Networks<'_> {
        Networks(self)
    }

    pub fn services(&self) -> Services<'_> {
        Services(self)
    }

    pub fn configs(&self) -> Configs<'_> {
        Configs(self)
    }

    pub fn secrets(&self) -> Secrets<'_> {
        Secrets(self)
    }

    pub fn tasks(&self) -> Tasks<'_> {
        Tasks(self)
    }

    pub fn containers(&self) -> Containers<'_> {
        Containers(self)
    }

    pub fn volumes(&self) -> Volumes<'_> {
        Volumes(self)
    }

    async fn get_opt<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<Option<T>> {
        let bytes = match self.request(Method::GET, path, query, None, None).await {
            Ok(bytes) => bytes,
            Err(e) if e.is_not_found() => return Ok(None),
            Err(e) => return Err(e),
        };
        Ok(Some(serde_json::from_slice(&bytes)?))
    }

    async fn list<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<Vec<T>> {
        let bytes = self.request(Method::GET, path, query, None, None).await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn post_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
        body: Option<String>,
        auth: Option<&RegistryAuth>,
    ) -> Result<T> {
        let bytes = self.request(Method::POST, path, query, body, auth).await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn post_maybe<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
        body: Option<String>,
        auth: Option<&RegistryAuth>,
    ) -> Result<Option<T>> {
        let bytes = self.request(Method::POST, path, query, body, auth).await?;
        if bytes.is_empty() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_slice(&bytes)?))
    }

    async fn post_discard(
        &self,
        path: &str,
        query: &[(&str, String)],
        body: Option<String>,
        auth: Option<&RegistryAuth>,
    ) -> Result<()> {
        self.request(Method::POST, path, query, body, auth).await?;
        Ok(())
    }

    async fn delete_idempotent(&self, path: &str) -> Result<bool> {
        match self.request(Method::DELETE, path, &[], None, None).await {
            Ok(_) => Ok(true),
            Err(e) if e.is_not_found() => Ok(false),
            Err(e) => Err(e),
        }
    }

    async fn delete_not_found_ok(&self, path: &str, query: &[(&str, String)]) -> Result<()> {
        match self.request(Method::DELETE, path, query, None, None).await {
            Ok(_) => Ok(()),
            Err(e) if e.is_not_found() => Ok(()),
            Err(e) => Err(e),
        }
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<String>,
        auth: Option<&RegistryAuth>,
    ) -> Result<Bytes> {
        let full_path = if path == VERSION_PATH {
            path.to_owned()
        } else {
            format!("/{}{path}", self.api_version)
        };
        let url = format!("http://localhost{full_path}");
        let mut req = self
            .http
            .request(method, url)
            .header("Accept", "application/json")
            .header("User-Agent", USER_AGENT);
        if !query.is_empty() {
            req = req.query(query);
        }
        if let Some(auth) = auth {
            req = req.header("X-Registry-Auth", URL_SAFE.encode(to_json_body(auth)));
        }
        if let Some(body) = body {
            req = req.header("Content-Type", "application/json").body(body);
        }
        let resp = req.send().await?;
        let status = resp.status();
        let bytes = resp.bytes().await?;
        if status.is_success() {
            return Ok(bytes);
        }
        let msg = serde_json::from_slice::<DockerErrorBody>(&bytes)
            .map_or_else(|_| snippet(&bytes), |body| body.message);
        Err(Error::new(ErrorKind::Docker { code: status, msg }))
    }
}

pub(crate) fn encode_immutable_data(data: &[u8], max: usize) -> Result<String> {
    use base64::engine::general_purpose::STANDARD;
    if data.len() > max {
        return Err(Error::new(ErrorKind::DataTooLarge {
            size: data.len(),
            max,
        }));
    }
    Ok(STANDARD.encode(data))
}

pub(crate) fn to_json_body<T: Serialize + ?Sized>(value: &T) -> String {
    serde_json::json!(value).to_string()
}

pub(crate) fn filter_query(filter: &crate::types::Filter) -> Vec<(&'static str, String)> {
    match filter.to_query() {
        Some(json) => vec![("filters", json)],
        None => Vec::new(),
    }
}

#[derive(Deserialize)]
struct DockerErrorBody {
    message: String,
}

fn snippet(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).chars().take(512).collect()
}

fn canonical_api_version(version: &semver::Version) -> String {
    format!("v{}.{}", version.major, version.minor)
}

fn normalize_api_version(input: &str) -> String {
    let trimmed = input.trim();
    let trimmed = trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('V'))
        .unwrap_or(trimmed);
    match trimmed.matches('.').count() {
        0 => format!("{trimmed}.0.0"),
        1 => format!("{trimmed}.0"),
        _ => trimmed.to_owned(),
    }
}

fn build_http(socket: &str, timeout: Duration) -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .unix_socket(socket)
        .timeout(timeout)
        .build()?)
}
