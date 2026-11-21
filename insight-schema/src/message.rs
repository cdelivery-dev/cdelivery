use std::sync::{Arc, LazyLock};

use arrow::array::{StringDictionaryBuilder, StringViewBuilder, TimestampNanosecondBuilder};
use arrow::datatypes::{DataType, Field, Schema, UInt8Type, UInt32Type};
use arrow::record_batch::RecordBatch;
use insight_wire::message::Message;

use crate::error::Result;

pub const TIMELINE: &str = "timeline";
pub const ROLE: &str = "role";
pub const AUTHOR: &str = "author";
pub const TEXT: &str = "text";

const ROW_FIXED_BYTES: usize = 64;

pub static MESSAGE_SCHEMA: LazyLock<Arc<Schema>> = LazyLock::new(|| {
    Arc::new(Schema::new(vec![
        super::nanos_field(super::TIMESTAMP),
        Field::new(super::ID, DataType::Utf8View, false),
        super::dict32(TIMELINE, false),
        super::dict8(ROLE, false),
        super::dict32(AUTHOR, false),
        Field::new(TEXT, DataType::Utf8View, false),
    ]))
});

#[expect(missing_debug_implementations)]
pub struct MessageBatchBuilder {
    timestamp: TimestampNanosecondBuilder,
    id: StringViewBuilder,
    timeline: StringDictionaryBuilder<UInt32Type>,
    role: StringDictionaryBuilder<UInt8Type>,
    author: StringDictionaryBuilder<UInt32Type>,
    text: StringViewBuilder,
    pending: super::Pending,
}

impl super::Batch for MessageBatchBuilder {
    type Record = Message;
    const INDEX_ID: &'static str = insight_wire::message::INDEX_ID;

    fn fresh(capacity: usize) -> Self {
        Self {
            timestamp: TimestampNanosecondBuilder::with_capacity(capacity),
            id: StringViewBuilder::with_capacity(capacity),
            timeline: StringDictionaryBuilder::<UInt32Type>::new(),
            role: StringDictionaryBuilder::<UInt8Type>::new(),
            author: StringDictionaryBuilder::<UInt32Type>::new(),
            text: StringViewBuilder::with_capacity(capacity),
            pending: super::Pending::default(),
        }
    }

    fn pending(&self) -> &super::Pending {
        &self.pending
    }

    fn timestamp_mut(record: &mut Message) -> &mut jiff::Timestamp {
        &mut record.timestamp
    }

    fn append(&mut self, _node_id: &str, record: &Message) -> Result<bool> {
        let timestamp = super::checked_nanos(Self::INDEX_ID, record.timestamp)?;
        self.id.append_value(record.id.to_string());
        self.timeline.append(&record.timeline)?;
        self.role.append(record.role.as_str())?;
        self.author.append(&record.author)?;

        self.timestamp.append_value(timestamp);
        self.text.append_value(&record.text);

        Ok(self.pending.mark(
            ROW_FIXED_BYTES + 36 + record.timeline.len() + record.author.len() + record.text.len(),
        ))
    }

    fn flush(&mut self) -> Result<RecordBatch> {
        self.pending.take();
        Ok(RecordBatch::try_new(
            MESSAGE_SCHEMA.clone(),
            vec![
                Arc::new(self.timestamp.finish()),
                Arc::new(self.id.finish()),
                Arc::new(self.timeline.finish()),
                Arc::new(self.role.finish()),
                Arc::new(self.author.finish()),
                Arc::new(self.text.finish()),
            ],
        )?)
    }
}
