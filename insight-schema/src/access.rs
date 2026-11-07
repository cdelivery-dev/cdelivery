use std::num::NonZeroU16;
use std::sync::{Arc, LazyLock};

use arrow::array::{
    StringDictionaryBuilder, StringViewBuilder, TimestampNanosecondBuilder, UInt16Builder,
    UInt64Builder,
};
use arrow::datatypes::{DataType, Field, Schema, UInt8Type, UInt32Type};
use arrow::record_batch::RecordBatch;
use pingress_wire::access::{AccessRecord, Method};

use crate::error::Result;

pub const DOMAIN: &str = "domain";
pub const UNMATCHED_HOST: &str = "unmatched_host";
pub const CLIENT_IP: &str = "client_ip";
pub const METHOD: &str = "method";
pub const PATH: &str = "path";
pub const STATUS: &str = "status";
pub const OUTCOME: &str = "outcome";
pub const DURATION_US: &str = "duration_us";
pub const UPSTREAM_TTFB_US: &str = "upstream_ttfb_us";
pub const REQUEST_BODY_BYTES: &str = "request_body_bytes";
pub const RESPONSE_BODY_BYTES: &str = "response_body_bytes";
pub const APP_NAME: &str = "app_name";
pub const TARGET_PORT: &str = "target_port";
pub const SERVICE_ID: &str = "service_id";
pub const ERROR: &str = "error";

const ROW_FIXED_BYTES: usize = 64;

pub static PINGRESS_ACCESS_SCHEMA: LazyLock<Arc<Schema>> = LazyLock::new(|| {
    Arc::new(Schema::new(vec![
        super::nanos_field(super::TIMESTAMP),
        super::dict32(super::NODE_ID, false),
        super::dict32(DOMAIN, true),
        Field::new(UNMATCHED_HOST, DataType::Utf8View, true),
        Field::new(CLIENT_IP, DataType::Utf8View, true),
        super::dict8(METHOD, false),
        Field::new(PATH, DataType::Utf8View, false),
        Field::new(STATUS, DataType::UInt16, true),
        super::dict8(OUTCOME, false),
        Field::new(DURATION_US, DataType::UInt64, false),
        Field::new(UPSTREAM_TTFB_US, DataType::UInt64, true),
        Field::new(REQUEST_BODY_BYTES, DataType::UInt64, false),
        Field::new(RESPONSE_BODY_BYTES, DataType::UInt64, false),
        super::dict32(APP_NAME, true),
        Field::new(TARGET_PORT, DataType::UInt16, true),
        super::dict32(SERVICE_ID, true),
        super::dict32(ERROR, true),
    ]))
});

fn estimated_bytes(record: &AccessRecord<'_>) -> usize {
    let opt = |value: Option<&str>| value.map_or(0, str::len);
    ROW_FIXED_BYTES
        + opt(record.domain.as_deref())
        + opt(record.unmatched_host.as_deref())
        + opt(record.client_ip.as_deref())
        + record.path.len()
        + opt(record.app_name.as_deref())
        + opt(record.service_id.as_deref())
        + opt(record.error.as_deref())
}

fn append_dict32(
    builder: &mut StringDictionaryBuilder<UInt32Type>,
    value: Option<&str>,
) -> Result<()> {
    match value {
        Some(value) => {
            builder.append(value)?;
        }
        None => builder.append_null(),
    }
    Ok(())
}

#[expect(missing_debug_implementations)]
pub struct AccessBatchBuilder {
    timestamp: TimestampNanosecondBuilder,
    node_id: StringDictionaryBuilder<UInt32Type>,
    domain: StringDictionaryBuilder<UInt32Type>,
    unmatched_host: StringViewBuilder,
    client_ip: StringViewBuilder,
    method: StringDictionaryBuilder<UInt8Type>,
    path: StringViewBuilder,
    status: UInt16Builder,
    outcome: StringDictionaryBuilder<UInt8Type>,
    duration_us: UInt64Builder,
    upstream_ttfb_us: UInt64Builder,
    request_body_bytes: UInt64Builder,
    response_body_bytes: UInt64Builder,
    app_name: StringDictionaryBuilder<UInt32Type>,
    target_port: UInt16Builder,
    service_id: StringDictionaryBuilder<UInt32Type>,
    error: StringDictionaryBuilder<UInt32Type>,
    pending: super::Pending,
}

impl super::Batch for AccessBatchBuilder {
    type Record = AccessRecord<'static>;
    const INDEX_ID: &'static str = pingress_wire::access::INDEX_ID;

    fn fresh(capacity: usize) -> Self {
        Self {
            timestamp: TimestampNanosecondBuilder::with_capacity(capacity),
            node_id: StringDictionaryBuilder::<UInt32Type>::new(),
            domain: StringDictionaryBuilder::<UInt32Type>::new(),
            unmatched_host: StringViewBuilder::with_capacity(capacity),
            client_ip: StringViewBuilder::with_capacity(capacity),
            method: StringDictionaryBuilder::<UInt8Type>::new(),
            path: StringViewBuilder::with_capacity(capacity),
            status: UInt16Builder::with_capacity(capacity),
            outcome: StringDictionaryBuilder::<UInt8Type>::new(),
            duration_us: UInt64Builder::with_capacity(capacity),
            upstream_ttfb_us: UInt64Builder::with_capacity(capacity),
            request_body_bytes: UInt64Builder::with_capacity(capacity),
            response_body_bytes: UInt64Builder::with_capacity(capacity),
            app_name: StringDictionaryBuilder::<UInt32Type>::new(),
            target_port: UInt16Builder::with_capacity(capacity),
            service_id: StringDictionaryBuilder::<UInt32Type>::new(),
            error: StringDictionaryBuilder::<UInt32Type>::new(),
            pending: super::Pending::default(),
        }
    }

    fn pending(&self) -> &super::Pending {
        &self.pending
    }

    fn timestamp_mut<'r>(record: &'r mut AccessRecord<'static>) -> &'r mut jiff::Timestamp {
        &mut record.timestamp
    }

    fn append(&mut self, node_id: &str, record: &AccessRecord<'static>) -> Result<bool> {
        let timestamp = super::checked_nanos(Self::INDEX_ID, record.timestamp)?;
        self.node_id.append(node_id)?;
        append_dict32(&mut self.domain, record.domain.as_deref())?;
        append_dict32(&mut self.app_name, record.app_name.as_deref())?;
        append_dict32(&mut self.service_id, record.service_id.as_deref())?;
        append_dict32(&mut self.error, record.error.as_deref())?;
        self.method
            .append(Method::normalize(&record.method).as_str())?;
        self.outcome.append(record.outcome.as_str())?;

        self.timestamp.append_value(timestamp);
        self.unmatched_host
            .append_option(record.unmatched_host.as_deref());
        self.client_ip.append_option(record.client_ip.as_deref());
        self.path.append_value(&record.path);
        self.status.append_option(record.status);
        self.duration_us.append_value(record.duration_us);
        self.upstream_ttfb_us.append_option(record.upstream_ttfb_us);
        self.request_body_bytes
            .append_value(record.request_body_bytes);
        self.response_body_bytes
            .append_value(record.response_body_bytes);
        self.target_port
            .append_option(record.target_port.map(NonZeroU16::get));

        Ok(self.pending.mark(estimated_bytes(record)))
    }

    fn flush(&mut self) -> Result<RecordBatch> {
        self.pending.take();
        Ok(RecordBatch::try_new(
            PINGRESS_ACCESS_SCHEMA.clone(),
            vec![
                Arc::new(self.timestamp.finish()),
                Arc::new(self.node_id.finish()),
                Arc::new(self.domain.finish()),
                Arc::new(self.unmatched_host.finish()),
                Arc::new(self.client_ip.finish()),
                Arc::new(self.method.finish()),
                Arc::new(self.path.finish()),
                Arc::new(self.status.finish()),
                Arc::new(self.outcome.finish()),
                Arc::new(self.duration_us.finish()),
                Arc::new(self.upstream_ttfb_us.finish()),
                Arc::new(self.request_body_bytes.finish()),
                Arc::new(self.response_body_bytes.finish()),
                Arc::new(self.app_name.finish()),
                Arc::new(self.target_port.finish()),
                Arc::new(self.service_id.finish()),
                Arc::new(self.error.finish()),
            ],
        )?)
    }
}
