use std::borrow::Cow;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::num::NonZeroU16;

use serde::{Deserialize, Serialize};
use strum::{EnumString, IntoStaticStr, VariantArray};

pub const INDEX_ID: &str = "cdelivery-pingress/access";
pub const SERVICE_NAME: &str = "cdelivery-pingress";
pub const PATH_MAX_BYTES: usize = 2048;

const HOST_MAX_BYTES: usize = 253;
const LABEL_MAX_BYTES: usize = 63;

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, VariantArray, IntoStaticStr)]
pub enum Outcome {
    Proxied,
    UpstreamFailed,
    NoRoute,
    NoUpstream,
    InvalidHost,
    HttpsRedirect,
    AcmeChallenge,
    Aborted,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, VariantArray, IntoStaticStr,
)]
pub enum StatusClass {
    Informational,
    Success,
    Redirection,
    ClientError,
    ServerError,
    NoResponse,
}

impl StatusClass {
    pub const fn bounds(self) -> Option<(u16, Option<u16>)> {
        match self {
            Self::Informational => Some((0, Some(200))),
            Self::Success => Some((200, Some(300))),
            Self::Redirection => Some((300, Some(400))),
            Self::ClientError => Some((400, Some(500))),
            Self::ServerError => Some((500, None)),
            Self::NoResponse => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, EnumString, IntoStaticStr)]
#[serde(rename_all = "UPPERCASE")]
#[strum(serialize_all = "UPPERCASE")]
pub enum Method {
    Get,
    Head,
    Post,
    Put,
    Delete,
    Patch,
    Options,
    Trace,
    Connect,
    Other,
}

impl Method {
    pub fn normalize(raw: &str) -> Self {
        raw.parse().unwrap_or(Self::Other)
    }

    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

#[derive(Debug)]
pub struct Host(String);

impl Host {
    pub fn parse(raw: &str) -> Option<Self> {
        if let Some(rest) = raw.strip_prefix('[') {
            let (literal, tail) = rest.split_once(']')?;
            if !tail.is_empty() {
                tail.strip_prefix(':')?.parse::<u16>().ok()?;
            }
            let ip: Ipv6Addr = literal.parse().ok()?;
            return Some(Self(IpAddr::V6(ip).to_canonical().to_string()));
        }
        let name = match raw.split_once(':') {
            Some((name, port)) => {
                port.parse::<u16>().ok()?;
                name
            }
            None => raw,
        };
        let name = name.strip_suffix('.').unwrap_or(name);
        if name.is_empty() || name.len() > HOST_MAX_BYTES {
            return None;
        }
        if let Ok(ip) = name.parse::<Ipv4Addr>() {
            return Some(Self(ip.to_string()));
        }
        let lowered = name.to_ascii_lowercase();
        let label_ok = |label: &str| {
            !label.is_empty()
                && label.len() <= LABEL_MAX_BYTES
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        };
        lowered.split('.').all(label_ok).then_some(Self(lowered))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Filter {
    pub node_id: Option<String>,
    pub domain: Option<String>,
    pub unmatched_host: Option<String>,
    pub client_ip: Option<String>,
    pub method: Option<Method>,
    pub path: Option<String>,
    pub status: Option<u16>,
    pub status_class: Option<StatusClass>,
    pub outcome: Option<Outcome>,
    pub app_name: Option<String>,
    pub target_port: Option<u16>,
    pub service_id: Option<String>,
    pub error: Option<String>,
    pub search_path: Option<String>,
    pub search_client_ip: Option<String>,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Serialize, Deserialize)]
pub struct AccessRecord<'a> {
    pub timestamp: jiff::Timestamp,
    pub domain: Option<Cow<'a, str>>,
    pub unmatched_host: Option<Cow<'a, str>>,
    pub client_ip: Option<String>,
    pub method: Cow<'a, str>,
    pub path: Cow<'a, str>,
    pub status: Option<u16>,
    pub outcome: Outcome,
    pub duration_us: u64,
    pub upstream_ttfb_us: Option<u64>,
    pub request_body_bytes: u64,
    pub response_body_bytes: u64,
    pub app_name: Option<Cow<'a, str>>,
    pub target_port: Option<NonZeroU16>,
    pub service_id: Option<Cow<'a, str>>,
    pub error: Option<Cow<'a, str>>,
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize, Deserialize)]
pub struct Access {
    pub timestamp: jiff::Timestamp,
    pub node_id: String,
    pub domain: Option<String>,
    pub unmatched_host: Option<String>,
    pub client_ip: Option<String>,
    pub method: String,
    pub path: String,
    pub status: Option<u16>,
    pub outcome: Outcome,
    pub duration_us: u64,
    pub upstream_ttfb_us: Option<u64>,
    pub request_body_bytes: u64,
    pub response_body_bytes: u64,
    pub app_name: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<u16>, minimum = 1))]
    pub target_port: Option<NonZeroU16>,
    pub service_id: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StatsRow {
    pub timestamp: jiff::Timestamp,
    pub count: u64,
    pub request_bytes: u64,
    pub response_bytes: u64,
    pub duration_p50: Option<f64>,
    pub duration_p95: Option<f64>,
    pub duration_p99: Option<f64>,
    pub duration_max: Option<u64>,
    pub ttfb_count: u64,
    pub ttfb_p50: Option<f64>,
    pub ttfb_p95: Option<f64>,
    pub ttfb_p99: Option<f64>,
    pub ttfb_max: Option<u64>,
    #[serde(flatten)]
    pub counts: std::collections::HashMap<String, u64>,
}

impl StatsRow {
    pub fn count_of(&self, key: &str) -> u64 {
        self.counts.get(key).copied().unwrap_or_default()
    }
}

#[derive(Debug, Deserialize)]
pub struct StatusRow {
    pub timestamp: jiff::Timestamp,
    pub status: Option<u16>,
    pub count: u64,
}
