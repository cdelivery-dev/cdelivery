use std::sync::{Arc, LazyLock};

use arrow::array::{ArrayBuilder, StringViewBuilder, TimestampNanosecondBuilder};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use getset::CopyGetters;

use crate::error::Result;

pub const SOURCE: &str = "source";
pub const LOG: &str = "log";

const ROW_FIXED_BYTES: usize = 72;

pub static LOG_SCHEMA: LazyLock<Arc<Schema>> = LazyLock::new(|| {
    Arc::new(Schema::new(vec![
        super::nanos_field(super::TIMESTAMP),
        Field::new(SOURCE, DataType::Utf8View, false),
        Field::new(super::CONTAINER_ID, DataType::Utf8View, false),
        Field::new(super::NODE_ID, DataType::Utf8View, false),
        Field::new(LOG, DataType::Utf8View, false),
    ]))
});

#[derive(Debug)]
pub struct LogRecord {
    pub timestamp: i64,
    pub source: String,
    pub container_id: String,
    pub node_id: String,
    pub log: String,
}

#[derive(Debug, CopyGetters)]
pub struct LogBatchBuilder {
    timestamp: TimestampNanosecondBuilder,
    source: StringViewBuilder,
    container_id: StringViewBuilder,
    node_id: StringViewBuilder,
    log: StringViewBuilder,
    #[getset(get_copy = "pub")]
    last_active: std::time::Instant,
    #[getset(get_copy = "pub")]
    oldest_pending: Option<std::time::Instant>,
    rough_estimation_bytes: usize,
}

impl LogBatchBuilder {
    pub fn new(capacity: usize) -> Self {
        Self {
            timestamp: TimestampNanosecondBuilder::with_capacity(capacity),
            source: StringViewBuilder::with_capacity(capacity),
            container_id: StringViewBuilder::with_capacity(capacity),
            node_id: StringViewBuilder::with_capacity(capacity),
            log: StringViewBuilder::with_capacity(capacity),
            last_active: std::time::Instant::now(),
            oldest_pending: None,
            rough_estimation_bytes: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.timestamp.is_empty()
    }

    pub fn append(&mut self, record: &LogRecord) -> bool {
        self.timestamp.append_value(record.timestamp);
        self.source.append_value(&record.source);
        self.container_id.append_value(&record.container_id);
        self.node_id.append_value(&record.node_id);
        self.log.append_value(&record.log);

        self.rough_estimation_bytes += ROW_FIXED_BYTES
            + record.source.len()
            + record.container_id.len()
            + record.node_id.len()
            + record.log.len();
        self.last_active = std::time::Instant::now();
        self.oldest_pending.get_or_insert(self.last_active);
        self.rough_estimation_bytes >= super::FLUSH_BYTES
    }

    pub fn flush(&mut self) -> Result<RecordBatch> {
        self.oldest_pending = None;
        self.rough_estimation_bytes = 0;
        Ok(RecordBatch::try_new(
            LOG_SCHEMA.clone(),
            vec![
                Arc::new(self.timestamp.finish()),
                Arc::new(self.source.finish()),
                Arc::new(self.container_id.finish()),
                Arc::new(self.node_id.finish()),
                Arc::new(self.log.finish()),
            ],
        )?)
    }
}
