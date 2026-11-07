pub mod access;
mod error;
pub mod event;
pub mod log;
pub mod message;
pub mod metric;
pub mod sample;

pub use error::{Error, Result};

use std::sync::Arc;

use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use arrow::record_batch::RecordBatch;
use jiff::SignedDuration;

pub const FLUSH_BYTES: usize = 2 * 1024 * 1024;

pub const TIMESTAMP: &str = "timestamp";
pub const ID: &str = "id";
pub const CONTAINER_ID: &str = "container_id";
pub const NODE_ID: &str = "node_id";

#[derive(Debug, Clone, Copy, strum::VariantArray)]
pub enum IndexKind {
    Log,
    HostMetric,
    FilesystemMetric,
    ContainerMetric,
    PingressAccess,
    Event,
    Message,
}

impl IndexKind {
    pub fn of(index_id: &str) -> Option<Self> {
        match index_id {
            insight_wire::HOST_METRIC_INDEX_ID => Some(Self::HostMetric),
            insight_wire::FILESYSTEM_METRIC_INDEX_ID => Some(Self::FilesystemMetric),
            insight_wire::CONTAINER_METRIC_INDEX_ID => Some(Self::ContainerMetric),
            pingress_wire::access::INDEX_ID => Some(Self::PingressAccess),
            insight_wire::event::INDEX_ID => Some(Self::Event),
            insight_wire::message::INDEX_ID => Some(Self::Message),
            _ => match index_id.split_once('/') {
                Some((owner, "log")) if valid_service_id(owner) => Some(Self::Log),
                _ => None,
            },
        }
    }

    pub fn schema(self) -> Arc<Schema> {
        match self {
            Self::Log => log::LOG_SCHEMA.clone(),
            Self::HostMetric => metric::HOST_METRIC_SCHEMA.clone(),
            Self::FilesystemMetric => metric::FILESYSTEM_METRIC_SCHEMA.clone(),
            Self::ContainerMetric => metric::CONTAINER_METRIC_SCHEMA.clone(),
            Self::PingressAccess => access::PINGRESS_ACCESS_SCHEMA.clone(),
            Self::Event => event::EVENT_SCHEMA.clone(),
            Self::Message => message::MESSAGE_SCHEMA.clone(),
        }
    }
}

pub fn valid_service_id(service_id: &str) -> bool {
    (1..=64).contains(&service_id.len())
        && service_id.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

pub fn log_index(service_id: &str) -> String {
    format!("{service_id}/log")
}

pub trait Batch: Sized {
    type Record;
    const INDEX_ID: &'static str;
    fn fresh(capacity: usize) -> Self;
    fn append(&mut self, node_id: &str, record: &Self::Record) -> Result<bool>;
    fn flush(&mut self) -> Result<RecordBatch>;
    fn pending(&self) -> &Pending;
    fn timestamp_mut(record: &mut Self::Record) -> &mut jiff::Timestamp;
}

#[derive(Debug, Default)]
pub struct Pending {
    oldest: Option<std::time::Instant>,
    bytes: usize,
}

impl Pending {
    pub(crate) fn mark(&mut self, bytes: usize) -> bool {
        self.bytes += bytes;
        self.oldest.get_or_insert_with(std::time::Instant::now);
        self.bytes >= FLUSH_BYTES
    }

    pub(crate) fn take(&mut self) {
        self.oldest = None;
        self.bytes = 0;
    }

    pub fn oldest(&self) -> Option<std::time::Instant> {
        self.oldest
    }

    pub fn is_empty(&self) -> bool {
        self.bytes == 0
    }
}

pub(crate) fn checked_nanos(label: &'static str, timestamp: jiff::Timestamp) -> Result<i64> {
    i64::try_from(timestamp.as_nanosecond())
        .map_err(|_| error::ErrorKind::TimestampRange { label, timestamp }.into())
}

#[expect(clippy::cast_possible_truncation)]
const TIMESTAMP_MAX_PAST_NANOS: i64 = SignedDuration::from_hours(30 * 24).as_nanos() as i64;
#[expect(clippy::cast_possible_truncation)]
const TIMESTAMP_MAX_FUTURE_NANOS: i64 = SignedDuration::from_hours(1).as_nanos() as i64;

pub fn timestamp_within_window(ts_ns: i64, now_ns: i64) -> bool {
    ts_ns >= now_ns.saturating_sub(TIMESTAMP_MAX_PAST_NANOS)
        && ts_ns <= now_ns.saturating_add(TIMESTAMP_MAX_FUTURE_NANOS)
}

#[expect(clippy::cast_possible_truncation)]
pub fn timestamp_nanos(ts: jiff::Timestamp) -> i64 {
    ts.as_nanosecond() as i64
}

pub fn now_nanos() -> i64 {
    timestamp_nanos(jiff::Timestamp::now())
}

pub(crate) fn nanos_field(name: &str) -> Field {
    Field::new(name, DataType::Timestamp(TimeUnit::Nanosecond, None), false)
}

pub(crate) fn dict8(name: &str, nullable: bool) -> Field {
    Field::new(
        name,
        DataType::Dictionary(Box::new(DataType::UInt8), Box::new(DataType::Utf8)),
        nullable,
    )
}

pub(crate) fn dict32(name: &str, nullable: bool) -> Field {
    Field::new(
        name,
        DataType::Dictionary(Box::new(DataType::UInt32), Box::new(DataType::Utf8)),
        nullable,
    )
}
