use std::sync::Arc;

use datafusion::arrow::array::{Array, TimestampNanosecondArray};
use datafusion::arrow::record_batch::RecordBatch;
use insight::error::{ErrorKind, Result};
use insight_schema::log::LogBatchBuilder;

#[path = "scenario.rs"]
mod scenario;

const CORPUS_BYTES: u64 = 3 * 1024 * 1024 * 1024;

#[derive(Debug)]
struct CorpusBatch {
    batch: RecordBatch,
    raw_bytes: u64,
    query_rows: u64,
    trace_rows: u64,
}

#[derive(Debug)]
pub(crate) struct Corpus {
    batches: Vec<CorpusBatch>,
    pub(crate) raw_bytes: u64,
    pub(crate) rows: u64,
    query_words: [String; 3],
    trace_id: String,
    span_nanos: i64,
}

#[derive(Debug)]
pub(crate) struct Samples {
    pub(crate) batches: Vec<RecordBatch>,
    pub(crate) raw_bytes: u64,
    pub(crate) rows: u64,
    pub(crate) query_rows: u64,
    pub(crate) query_words: [String; 3],
    pub(crate) trace_id: String,
    pub(crate) trace_rows: u64,
}

fn checked_add(left: u64, right: u64) -> Result<u64> {
    left.checked_add(right)
        .ok_or_else(|| ErrorKind::Invariant("log sample size overflowed".to_owned()).into())
}

fn advance(left: i64, right: i64) -> Result<i64> {
    left.checked_add(right)
        .ok_or_else(|| ErrorKind::Invariant("log sample timestamp overflowed".to_owned()).into())
}

pub(crate) fn generate(target_bytes: u64, seed: u64) -> Result<Corpus> {
    if target_bytes == 0 {
        return Err(ErrorKind::Config("log sample size must be positive".to_owned()).into());
    }
    let target_bytes = target_bytes.min(CORPUS_BYTES);
    let mut generator = scenario::Generator::new(seed);
    let mut corpus = Corpus {
        batches: Vec::new(),
        raw_bytes: 0,
        rows: 0,
        query_words: generator.query_words.clone(),
        trace_id: generator.trace_id.clone(),
        span_nanos: 0,
    };
    let mut builder = LogBatchBuilder::new(1_024);
    let mut batch_bytes = 0;
    let mut query_rows = 0;
    let mut trace_rows = 0;
    let mut next_timestamp = 0;
    'windows: loop {
        for mut record in generator.window()? {
            record.timestamp = record.timestamp.max(next_timestamp);
            next_timestamp = advance(record.timestamp, 1)?;
            let bytes = record.log.len() as u64;
            query_rows += u64::from(
                corpus
                    .query_words
                    .iter()
                    .all(|word| record.log.contains(word)),
            );
            trace_rows += u64::from(record.log.contains(&corpus.trace_id));
            builder.append(&record);
            batch_bytes = checked_add(batch_bytes, bytes)?;
            corpus.raw_bytes = checked_add(corpus.raw_bytes, bytes)?;
            corpus.rows = checked_add(corpus.rows, 1)?;
            if batch_bytes >= super::BATCH_BYTES as u64 || corpus.raw_bytes >= target_bytes {
                corpus.batches.push(CorpusBatch {
                    batch: builder.flush()?,
                    raw_bytes: batch_bytes,
                    query_rows,
                    trace_rows,
                });
                batch_bytes = 0;
                query_rows = 0;
                trace_rows = 0;
            }
            if corpus.raw_bytes >= target_bytes {
                break 'windows;
            }
        }
    }
    corpus.span_nanos = advance(next_timestamp, 50_000)?;
    Ok(corpus)
}

fn timestamps(batch: &RecordBatch) -> Result<&TimestampNanosecondArray> {
    batch
        .column_by_name(insight_schema::TIMESTAMP)
        .and_then(|array| array.as_any().downcast_ref())
        .ok_or_else(|| {
            ErrorKind::Invariant("log sample timestamp column was invalid".to_owned()).into()
        })
}

impl Corpus {
    pub(crate) fn replay(&self, target_bytes: u64) -> Result<Samples> {
        if target_bytes == 0 {
            return Err(ErrorKind::Config("log replay size must be positive".to_owned()).into());
        }
        let mut samples = Samples {
            batches: Vec::new(),
            raw_bytes: 0,
            rows: 0,
            query_rows: 0,
            query_words: self.query_words.clone(),
            trace_id: self.trace_id.clone(),
            trace_rows: 0,
        };
        let mut selected = Vec::new();
        let mut cycle_offset = 0;
        let mut last = 0;
        while samples.raw_bytes < target_bytes {
            for sample in &self.batches {
                samples.raw_bytes = checked_add(samples.raw_bytes, sample.raw_bytes)?;
                samples.rows = checked_add(samples.rows, sample.batch.num_rows() as u64)?;
                samples.query_rows = checked_add(samples.query_rows, sample.query_rows)?;
                samples.trace_rows = checked_add(samples.trace_rows, sample.trace_rows)?;
                last = advance(
                    *timestamps(&sample.batch)?.values().last().ok_or_else(|| {
                        ErrorKind::Invariant("log sample batch was empty".to_owned())
                    })?,
                    cycle_offset,
                )?;
                if i128::from(last) >= super::RETENTION.as_nanos() {
                    return Err(ErrorKind::Config(
                        "log replay time range exceeds benchmark retention".to_owned(),
                    )
                    .into());
                }
                selected.push((sample.batch.clone(), cycle_offset));
                if samples.raw_bytes >= target_bytes {
                    break;
                }
            }
            if samples.raw_bytes < target_bytes {
                cycle_offset = advance(cycle_offset, self.span_nanos)?;
            }
        }
        let now = insight_schema::now_nanos();
        let base = now
            .checked_sub(last)
            .ok_or_else(|| ErrorKind::Invariant("log replay time range overflowed".to_owned()))?;
        for (batch, offset) in selected {
            let shifted = timestamps(&batch)?
                .values()
                .iter()
                .map(|timestamp| advance(base, advance(*timestamp, offset)?))
                .collect::<Result<Vec<_>>>()?;
            let shifted: Arc<dyn Array> = Arc::new(TimestampNanosecondArray::from(shifted));
            let columns = batch
                .schema()
                .fields()
                .iter()
                .zip(batch.columns())
                .map(|(field, column)| {
                    Arc::clone(if field.name() == insight_schema::TIMESTAMP {
                        &shifted
                    } else {
                        column
                    })
                })
                .collect();
            samples
                .batches
                .push(RecordBatch::try_new(batch.schema(), columns)?);
        }
        Ok(samples)
    }
}
