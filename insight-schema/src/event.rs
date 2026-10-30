use std::sync::{Arc, LazyLock};

use arrow::array::{StringDictionaryBuilder, StringViewBuilder, TimestampNanosecondBuilder};
use arrow::datatypes::{DataType, Field, Schema, UInt8Type, UInt32Type};
use arrow::record_batch::RecordBatch;
use insight_wire::event::Event;

use crate::error::Result;

pub const KIND: &str = "kind";
pub const SUBJECT: &str = "subject";
pub const ACTOR: &str = "actor";
pub const DETAIL: &str = "detail";

const ROW_FIXED_BYTES: usize = 64;

pub static EVENT_SCHEMA: LazyLock<Arc<Schema>> = LazyLock::new(|| {
    Arc::new(Schema::new(vec![
        super::nanos_field(super::TIMESTAMP),
        super::dict8(KIND, false),
        super::dict32(SUBJECT, false),
        super::dict32(ACTOR, false),
        Field::new(DETAIL, DataType::Utf8View, true),
    ]))
});

#[expect(missing_debug_implementations)]
pub struct EventBatchBuilder {
    timestamp: TimestampNanosecondBuilder,
    kind: StringDictionaryBuilder<UInt8Type>,
    subject: StringDictionaryBuilder<UInt32Type>,
    actor: StringDictionaryBuilder<UInt32Type>,
    detail: StringViewBuilder,
    pending: super::Pending,
}

impl super::Batch for EventBatchBuilder {
    type Record = Event;
    const INDEX_ID: &'static str = insight_wire::event::INDEX_ID;

    fn fresh(capacity: usize) -> Self {
        Self {
            timestamp: TimestampNanosecondBuilder::with_capacity(capacity),
            kind: StringDictionaryBuilder::<UInt8Type>::new(),
            subject: StringDictionaryBuilder::<UInt32Type>::new(),
            actor: StringDictionaryBuilder::<UInt32Type>::new(),
            detail: StringViewBuilder::with_capacity(capacity),
            pending: super::Pending::default(),
        }
    }

    fn pending(&self) -> &super::Pending {
        &self.pending
    }

    fn timestamp_mut(record: &mut Event) -> &mut jiff::Timestamp {
        &mut record.timestamp
    }

    fn append(&mut self, _node_id: &str, record: &Event) -> Result<bool> {
        let timestamp = super::checked_nanos(Self::INDEX_ID, record.timestamp)?;
        self.kind.append(record.kind.as_str())?;
        self.subject.append(&record.subject)?;
        self.actor.append(&record.actor)?;

        self.timestamp.append_value(timestamp);
        self.detail.append_option(record.detail.as_deref());

        Ok(self.pending.mark(
            ROW_FIXED_BYTES
                + record.subject.len()
                + record.actor.len()
                + record.detail.as_deref().map_or(0, str::len),
        ))
    }

    fn flush(&mut self) -> Result<RecordBatch> {
        self.pending.take();
        Ok(RecordBatch::try_new(
            EVENT_SCHEMA.clone(),
            vec![
                Arc::new(self.timestamp.finish()),
                Arc::new(self.kind.finish()),
                Arc::new(self.subject.finish()),
                Arc::new(self.actor.finish()),
                Arc::new(self.detail.finish()),
            ],
        )?)
    }
}
