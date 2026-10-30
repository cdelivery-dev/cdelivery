use std::borrow::Cow;

use cipher::Secret;
use serde::{Deserialize, Serialize};

pub mod cpu;
pub mod event;
pub mod log;
pub mod message;
pub mod metric;

pub const SERVICE_NAME: &str = "cdelivery-insight";
pub const PLATFORM_SERVICE_NAME: &str = "cdelivery";
pub const INDEX_FIELD: &str = "_index";
pub const KEY_SECRET_NAME: &str = "cdelivery-insight";

pub const STORAGE_ENV: &str = "STORAGE";
pub const NODE_ID_ENV: &str = "NODE_ID";
pub const DATA_RETENTION_DAYS_ENV: &str = "DATA_RETENTION_DAYS";
pub const SAMPLE_INTERVAL_SECS_ENV: &str = "SAMPLE_INTERVAL_SECS";
pub const WRITER_FLUSH_INTERVAL_SECS_ENV: &str = "WRITER_FLUSH_INTERVAL_SECS";
pub const SINGLE_INSTANCE_ENV: &str = "SINGLE_INSTANCE";

pub fn valid_node_id(node_id: &str) -> bool {
    !(node_id.is_empty() || node_id.contains('/') || node_id.contains("{{"))
}

pub fn writer_flush_interval_from_secs(secs: i64) -> Option<jiff::SignedDuration> {
    let value = if secs % 3600 == 0 {
        secs / 3600
    } else if secs % 60 == 0 {
        secs / 60
    } else {
        secs
    };
    (1..=1_000_000)
        .contains(&value)
        .then(|| jiff::SignedDuration::from_secs(secs))
}

pub const LOCAL_DATA_DIR: &str = "/opt/insight";
pub const HOST_MOUNT_PREFIX: &str = "/host";
pub const FLUENTD_SOCKET_PATH: &str = "/run/cdelivery/fluentd.sock";

pub const HTTP_PORT: u16 = 9010;
pub const INGEST_PORT: u16 = 9011;

pub mod frame {
    pub const MAGIC: u16 = 0xC15A;
    pub const VERSION: u8 = 1;
    pub const MAX_LEN: usize = 4 * 1024 * 1024;

    pub fn write_header(buf: &mut Vec<u8>, index_id: &str) {
        buf.extend_from_slice(&MAGIC.to_be_bytes());
        buf.push(VERSION);
        #[expect(clippy::cast_possible_truncation)]
        buf.extend_from_slice(&(index_id.len() as u16).to_be_bytes());
        buf.extend_from_slice(index_id.as_bytes());
    }

    pub fn parse_header(frame: &[u8]) -> Option<(&str, usize)> {
        let (magic, rest) = frame.split_first_chunk::<2>()?;
        let (version, rest) = rest.split_first()?;
        let (len, rest) = rest.split_first_chunk::<2>()?;
        if u16::from_be_bytes(*magic) != MAGIC || *version != VERSION {
            return None;
        }
        let len = usize::from(u16::from_be_bytes(*len));
        let index_id = std::str::from_utf8(rest.get(..len)?).ok()?;
        Some((index_id, 5 + len))
    }
}

pub const HOST_METRIC_INDEX_ID: &str = "cdelivery-host/metric";
pub const FILESYSTEM_METRIC_INDEX_ID: &str = "cdelivery-filesystem/metric";
pub const CONTAINER_METRIC_INDEX_ID: &str = "cdelivery-container/metric";
pub const MANAGED_LABEL: &str = "dev.cdelivery.managed";

#[derive(Debug, Serialize, Deserialize)]
pub struct Indexed<T> {
    #[serde(rename = "_index")]
    index: Cow<'static, str>,
    #[serde(flatten)]
    payload: T,
}

impl<T> Indexed<T> {
    pub const fn new(index: &'static str, payload: T) -> Self {
        Self {
            index: Cow::Borrowed(index),
            payload,
        }
    }

    pub fn index(&self) -> &str {
        &self.index
    }

    pub fn into_inner(self) -> T {
        self.payload
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjectStoreConfig {
    Local,
    S3 {
        region: String,
        bucket_name: String,
        access_key_id: String,
        secret_access_key: Secret,
    },
    S3Compatible {
        endpoint: String,
        region: String,
        bucket_name: String,
        access_key_id: String,
        secret_access_key: Secret,
    },
    Gcs {
        bucket_name: String,
        credentials: Secret,
    },
    Azure {
        container_name: String,
        account_name: String,
        account_key: Secret,
    },
}
