use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use arrow::record_batch::RecordBatch;
use bytes::{Buf, BytesMut};
use insight_wire::event::Event;
use insight_wire::message::Message;
use pingress_wire::access::AccessRecord;
use rmpv::Value;
use rmpv::decode::read_value;
use rmpv::ext::from_value;
use serde::Deserialize;
use tokio::io::AsyncReadExt;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::{Duration, timeout};
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

use crate::error::{ErrorKind, Result};
use insight_schema::access::AccessBatchBuilder;
use insight_schema::event::EventBatchBuilder;
use insight_schema::log::{LogBatchBuilder, LogRecord};
use insight_schema::message::MessageBatchBuilder;
use insight_schema::{Batch, log_index, timestamp_within_window, valid_service_id};

const LOG_BATCH_FLUSH_INTERVAL: Duration = Duration::from_secs(3);
const MAX_IDLE_TIMEOUT: Duration = Duration::from_mins(5);
const SENDER_CHANNEL_CAPACITY: usize = 128;
const LOG_BATCH_BUILDER_CAPACITY: usize = 1024;
const MAX_SOCKET_BUFFER: usize = 2 * 1024 * 1024;

#[derive(Default)]
struct Batchers {
    log: scc::HashMap<String, LogBatchBuilder>,
    access: tokio::sync::Mutex<Option<AccessBatchBuilder>>,
    event: tokio::sync::Mutex<Option<EventBatchBuilder>>,
    message: tokio::sync::Mutex<Option<MessageBatchBuilder>>,
    out_of_window: AtomicU64,
    suspicious: AtomicU64,
    malformed: AtomicU64,
    sender_full: AtomicU64,
}

#[derive(Debug, Deserialize)]
struct LogLine {
    log: String,
    container_id: String,
    source: String,
    #[serde(rename = "com.docker.swarm.service.id")]
    service_id: Option<String>,
    #[serde(rename = "com.docker.swarm.node.id")]
    node_id: Option<String>,
    #[serde(rename = "com.docker.swarm.service.name")]
    service_name: Option<String>,
}

pub(crate) async fn start(cancel: CancellationToken) -> Result<()> {
    let socket_path = insight_wire::FLUENTD_SOCKET_PATH;
    if let Some(dir) = std::path::Path::new(socket_path).parent() {
        std::fs::create_dir_all(dir)?;
    }
    if std::fs::exists(socket_path)? {
        std::fs::remove_file(socket_path)?;
    }
    let listener = UnixListener::bind(socket_path)?;
    tracing::info!("fluentd agent listening on unix://{socket_path}");

    let batchers = Arc::new(Batchers::default());
    let (tx, rx) = mpsc::channel::<(String, RecordBatch)>(SENDER_CHANNEL_CAPACITY);
    let mut client_tasks = JoinSet::new();

    let sweeping_handle = AbortOnDropHandle::new(tokio::spawn(sweeping_task(
        Arc::clone(&batchers),
        tx.clone(),
        cancel.clone(),
    )));
    let mut sender_handle = AbortOnDropHandle::new(tokio::spawn(crate::sender::send_worker(
        format!(
            "{}:{}",
            insight_wire::SERVICE_NAME,
            insight_wire::INGEST_PORT
        ),
        rx,
    )));

    let mut accept_error = None;
    let sender_result = loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => {
                tracing::info!("stopping fluentd intake");
                break None;
            }
            res = listener.accept() => {
                let (stream, addr) = match res {
                    Ok(accepted) => accepted,
                    Err(e) => {
                        accept_error = Some(e);
                        break None;
                    }
                };
                let batchers = Arc::clone(&batchers);
                let tx = tx.clone();
                let cancel = cancel.clone();
                client_tasks.spawn(async move {
                    if let Err(e) = Box::pin(handle_client(batchers, stream, tx, cancel)).await {
                        tracing::warn!(error = ?e, "fluentd client {addr:?} error");
                    }
                });
            }
            Some(res) = client_tasks.join_next() => {
                if let Err(e) = res {
                    tracing::error!(error = ?e, "fluentd client task aborted");
                }
            }
            result = &mut sender_handle => {
                break Some(result);
            }
        }
    };

    cancel.cancel();
    while let Some(res) = client_tasks.join_next().await {
        if let Err(e) = res {
            tracing::error!(error = ?e, "fluentd client task aborted");
        }
    }
    let _ = sweeping_handle.await;
    if let Some(result) = sender_result {
        match result {
            Ok(()) => tracing::error!("insight sender task exited unexpectedly"),
            Err(e) => tracing::error!(error = ?e, "insight sender task failed"),
        }
        return Err(ErrorKind::SenderExited.into());
    }
    flush_final_batches(&batchers, &tx).await;
    drop(tx);
    if let Some(e) = accept_error {
        return Err(e.into());
    }
    crate::sender::finish_sender(sender_handle).await;
    tracing::info!("fluentd agent shut down cleanly");
    Ok(())
}

async fn handle_client(
    batchers: Arc<Batchers>,
    mut stream: UnixStream,
    tx: mpsc::Sender<(String, RecordBatch)>,
    cancel: CancellationToken,
) -> Result<()> {
    let mut buffer = BytesMut::with_capacity(16384);

    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => {
                break;
            }
            res = timeout(MAX_IDLE_TIMEOUT, stream.read_buf(&mut buffer)) => {
                let n = match res {
                    Ok(Ok(n)) => n,
                    Ok(Err(e)) => return Err(e.into()),
                    Err(_) => {
                        tracing::info!("fluentd client idle timeout, closing connection");
                        break;
                    }
                };
                if n == 0 {
                    break;
                }
                if buffer.len() > MAX_SOCKET_BUFFER {
                    return Err(ErrorKind::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "fluentd frame exceeds maximum buffer size",
                    ))
                    .into());
                }
                loop {
                    if buffer.is_empty() {
                        break;
                    }
                    let mut cursor = Cursor::new(buffer.as_ref());
                    match read_value(&mut cursor) {
                        Ok(val) => {
                            let consumed = usize::try_from(cursor.position()).unwrap_or(0);
                            if consumed == 0 {
                                return Err(ErrorKind::Io(std::io::Error::new(
                                    std::io::ErrorKind::InvalidData,
                                    "zero bytes consumed while decoding MessagePack",
                                ))
                                .into());
                            }
                            buffer.advance(consumed);
                            process_fluentd_value(
                                &batchers,
                                val,
                                &tx,
                                &cancel,
                                insight_schema::now_nanos(),
                            )
                            .await;
                            if cancel.is_cancelled() {
                                return Ok(());
                            }
                        }
                        Err(e) => match &e {
                            rmpv::decode::Error::InvalidMarkerRead(io_err)
                            | rmpv::decode::Error::InvalidDataRead(io_err)
                                if io_err.kind() == std::io::ErrorKind::UnexpectedEof =>
                            {
                                break;
                            }
                            _ => return Err(e.into()),
                        },
                    }
                }
            }
        }
    }
    Ok(())
}

async fn process_fluentd_value(
    batchers: &Arc<Batchers>,
    val: Value,
    tx: &mpsc::Sender<(String, RecordBatch)>,
    cancel: &CancellationToken,
    received_at: i64,
) {
    let Value::Array(arr) = val else {
        logging::rate_limited_warn(&batchers.malformed, 1, "malformed fluentd message dropped");
        return;
    };
    let mut iter = arr.into_iter();
    let _tag = iter.next();
    let (Some(time), Some(record)) = (iter.next(), iter.next()) else {
        logging::rate_limited_warn(&batchers.malformed, 1, "malformed fluentd message dropped");
        return;
    };
    if matches!(time, Value::Array(_) | Value::Binary(_)) {
        logging::rate_limited_warn(
            &batchers.malformed,
            1,
            "unsupported fluentd forwarding mode dropped",
        );
        return;
    }
    let Ok(log_record) = from_value::<LogLine>(record) else {
        logging::rate_limited_warn(&batchers.malformed, 1, "malformed fluentd record dropped");
        return;
    };
    let service_name = log_record.service_name.as_deref();
    if let Some(service_name) = service_name
        && matches!(
            service_name,
            pingress_wire::access::SERVICE_NAME | insight_wire::PLATFORM_SERVICE_NAME
        )
        && let Some(node_id) = log_record.node_id.as_deref()
        && let Ok(json) = serde_json::from_str::<serde_json::Value>(&log_record.log)
        && handle_marked_line(batchers, &json, service_name, node_id, tx, cancel).await
    {
        return;
    }
    if cancel.is_cancelled() {
        return;
    }
    let (Some(service_id), Some(node_id)) = (log_record.service_id, log_record.node_id) else {
        logging::rate_limited_warn(
            &batchers.malformed,
            1,
            "service log record without service or node id dropped",
        );
        return;
    };
    if !valid_service_id(&service_id) {
        logging::rate_limited_warn(
            &batchers.suspicious,
            1,
            "service log record with invalid service id dropped",
        );
        return;
    }
    let record = LogRecord {
        timestamp: parse_time_value(&time, received_at, &batchers.out_of_window),
        source: log_record.source,
        container_id: log_record.container_id,
        node_id,
        log: log_record.log,
    };
    append_service_log(batchers, log_index(&service_id), &record, tx, cancel).await;
}

async fn append_service_log(
    batchers: &Batchers,
    index_id: String,
    record: &LogRecord,
    tx: &mpsc::Sender<(String, RecordBatch)>,
    cancel: &CancellationToken,
) {
    let ready = {
        let mut entry = batchers
            .log
            .entry_sync(index_id.clone())
            .or_insert_with(|| LogBatchBuilder::new(LOG_BATCH_BUILDER_CAPACITY));
        if entry.append(record) {
            match entry.flush() {
                Ok(batch) => Some(batch),
                Err(e) => {
                    tracing::error!(error = ?e, "failed to flush service log batch for {index_id}");
                    *entry = LogBatchBuilder::new(LOG_BATCH_BUILDER_CAPACITY);
                    None
                }
            }
        } else {
            None
        }
    };
    if let Some(batch) = ready {
        send_log_batch(index_id, batch, tx, cancel).await;
    }
}

async fn send_log_batch(
    index_id: String,
    batch: RecordBatch,
    tx: &mpsc::Sender<(String, RecordBatch)>,
    cancel: &CancellationToken,
) -> bool {
    let result = tokio::select! {
        biased;
        () = cancel.cancelled() => return false,
        result = tx.send((index_id.clone(), batch)) => result,
    };
    if let Err(e) = result {
        tracing::error!(error = ?e, "failed to send service log batch for {index_id}");
        false
    } else {
        true
    }
}

enum MarkedLine {
    NotMarked,
    Access(Box<AccessRecord<'static>>),
    Event(Box<Event>),
    Message(Box<Message>),
    Malformed,
    Suspicious,
}

async fn handle_marked_line(
    batchers: &Arc<Batchers>,
    json: &serde_json::Value,
    service_name: &str,
    node_id: &str,
    tx: &mpsc::Sender<(String, RecordBatch)>,
    cancel: &CancellationToken,
) -> bool {
    let stored = match classify_marked_line(json, service_name) {
        MarkedLine::NotMarked => return false,
        MarkedLine::Access(record) => {
            push(&batchers.access, batchers, *record, node_id, tx, cancel).await
        }
        MarkedLine::Event(record) => {
            push(&batchers.event, batchers, *record, node_id, tx, cancel).await
        }
        MarkedLine::Message(record) => {
            push(&batchers.message, batchers, *record, node_id, tx, cancel).await
        }
        MarkedLine::Malformed => {
            logging::rate_limited_warn(
                &batchers.malformed,
                1,
                "malformed marked ingest line falling back to service log",
            );
            return false;
        }
        MarkedLine::Suspicious => {
            logging::rate_limited_warn(
                &batchers.suspicious,
                1,
                "ingest index marker without matching service identity falling back to service log",
            );
            return false;
        }
    };
    if !stored && !cancel.is_cancelled() {
        logging::rate_limited_warn(
            &batchers.malformed,
            1,
            "marked ingest row could not be stored columnar, falling back to service log",
        );
    }
    stored
}

fn classify_marked_line(json: &serde_json::Value, service_name: &str) -> MarkedLine {
    let Some(marker) = json.get(insight_wire::INDEX_FIELD) else {
        return MarkedLine::NotMarked;
    };
    match marker.as_str() {
        Some(pingress_wire::access::INDEX_ID)
            if service_name == pingress_wire::access::SERVICE_NAME =>
        {
            match decode_indexed(json, pingress_wire::access::INDEX_ID) {
                Some(record) => MarkedLine::Access(Box::new(record)),
                None => MarkedLine::Malformed,
            }
        }
        Some(insight_wire::event::INDEX_ID)
            if service_name == insight_wire::PLATFORM_SERVICE_NAME =>
        {
            match decode_indexed(json, insight_wire::event::INDEX_ID) {
                Some(record) => MarkedLine::Event(Box::new(record)),
                None => MarkedLine::Malformed,
            }
        }
        Some(insight_wire::message::INDEX_ID)
            if service_name == insight_wire::PLATFORM_SERVICE_NAME =>
        {
            match decode_indexed(json, insight_wire::message::INDEX_ID) {
                Some(record) => MarkedLine::Message(Box::new(record)),
                None => MarkedLine::Malformed,
            }
        }
        _ => MarkedLine::Suspicious,
    }
}

fn decode_indexed<'de, T: Deserialize<'de>>(
    json: &'de serde_json::Value,
    index: &str,
) -> Option<T> {
    let indexed = insight_wire::Indexed::<T>::deserialize(json).ok()?;
    (indexed.index() == index).then(|| indexed.into_inner())
}

async fn push<B: Batch>(
    slot: &tokio::sync::Mutex<Option<B>>,
    batchers: &Batchers,
    mut record: B::Record,
    node_id: &str,
    tx: &mpsc::Sender<(String, RecordBatch)>,
    cancel: &CancellationToken,
) -> bool {
    let now_ns = insight_schema::now_nanos();
    let ts = B::timestamp_mut(&mut record);
    let ts_ns = i64::try_from(ts.as_nanosecond()).unwrap_or(i64::MAX);
    if !timestamp_within_window(ts_ns, now_ns) {
        logging::rate_limited_warn(
            &batchers.out_of_window,
            1,
            &format!(
                "{} timestamp outside acceptable window, using receipt time",
                B::INDEX_ID
            ),
        );
        *ts = jiff::Timestamp::now();
    }

    let mut guard = tokio::select! {
        biased;
        () = cancel.cancelled() => return false,
        guard = slot.lock() => guard,
    };
    let builder = guard.get_or_insert_with(|| B::fresh(LOG_BATCH_BUILDER_CAPACITY));
    let full = if let Ok(full) = builder.append(node_id, &record) {
        full
    } else {
        if !flush(builder, batchers, tx, cancel).await {
            return false;
        }
        match builder.append(node_id, &record) {
            Ok(full) => full,
            Err(e) => {
                tracing::error!(error = ?e, "{} row rejected even by a fresh batch builder", B::INDEX_ID);
                return false;
            }
        }
    };
    if full {
        flush(builder, batchers, tx, cancel).await;
    }
    true
}

async fn flush<B: Batch>(
    builder: &mut B,
    batchers: &Batchers,
    tx: &mpsc::Sender<(String, RecordBatch)>,
    cancel: &CancellationToken,
) -> bool {
    if builder.pending().is_empty() {
        return true;
    }
    let permit = match tx.try_reserve() {
        Ok(permit) => permit,
        Err(mpsc::error::TrySendError::Full(())) => {
            logging::rate_limited_warn(
                &batchers.sender_full,
                1,
                "sender queue full, waiting with completed ingest batch retained",
            );
            if let Ok(permit) = tokio::select! {
                biased;
                () = cancel.cancelled() => return false,
                result = tx.reserve() => result,
            } {
                permit
            } else {
                tracing::error!("sender queue closed, retaining completed ingest batch");
                return false;
            }
        }
        Err(mpsc::error::TrySendError::Closed(())) => {
            tracing::error!("sender queue closed, retaining completed ingest batch");
            return false;
        }
    };
    match builder.flush() {
        Ok(batch) => {
            if batch.num_rows() > 0 {
                permit.send((B::INDEX_ID.to_owned(), batch));
            }
        }
        Err(e) => {
            tracing::error!(error = ?e, "failed to flush {} batch, discarding builder state", B::INDEX_ID);
            *builder = B::fresh(LOG_BATCH_BUILDER_CAPACITY);
        }
    }
    true
}

async fn flush_slot<B: Batch>(
    slot: &tokio::sync::Mutex<Option<B>>,
    batchers: &Batchers,
    tx: &mpsc::Sender<(String, RecordBatch)>,
    cancel: &CancellationToken,
    due: impl Fn(&B) -> bool,
) -> bool {
    let mut guard = tokio::select! {
        biased;
        () = cancel.cancelled() => return false,
        guard = slot.lock() => guard,
    };
    match guard.as_mut() {
        Some(builder) if due(builder) => flush(builder, batchers, tx, cancel).await,
        _ => true,
    }
}

fn stale<B: Batch>(builder: &B) -> bool {
    builder
        .pending()
        .oldest()
        .is_some_and(|t| t.elapsed() >= LOG_BATCH_FLUSH_INTERVAL)
}

fn parse_time_value(time: &Value, received_at: i64, out_of_window: &AtomicU64) -> i64 {
    let parsed = match time {
        Value::Integer(seconds) => seconds
            .as_i64()
            .and_then(|value| value.checked_mul(1_000_000_000)),
        Value::Ext(0, bytes) if bytes.len() == 8 => {
            let seconds = bytes
                .get(..4)
                .and_then(|value| value.try_into().ok())
                .map(u32::from_be_bytes);
            let nanos = bytes
                .get(4..)
                .and_then(|value| value.try_into().ok())
                .map(u32::from_be_bytes)
                .filter(|value| *value < 1_000_000_000);
            seconds.zip(nanos).and_then(|(seconds, nanos)| {
                i64::from(seconds)
                    .checked_mul(1_000_000_000)
                    .and_then(|value| value.checked_add(i64::from(nanos)))
            })
        }
        _ => None,
    };
    if let Some(timestamp) = parsed
        && timestamp_within_window(timestamp, received_at)
    {
        return timestamp;
    }
    logging::rate_limited_warn(
        out_of_window,
        1,
        "invalid or out-of-window service log timestamp replaced with receipt time",
    );
    received_at
}

fn drain_log_batches(batchers: &Batchers, all: bool) -> Vec<(String, RecordBatch)> {
    let mut flushed = Vec::new();
    batchers.log.retain_sync(|index_id, builder| {
        let due = if all {
            !builder.is_empty()
        } else {
            builder
                .oldest_pending()
                .is_some_and(|time| time.elapsed() >= LOG_BATCH_FLUSH_INTERVAL)
        };
        if due {
            match builder.flush() {
                Ok(batch) => flushed.push((index_id.clone(), batch)),
                Err(e) => {
                    tracing::error!(error = ?e, "failed to flush service log batch for {index_id}");
                    return false;
                }
            }
        }
        if all {
            return false;
        }
        if builder.is_empty() && builder.last_active().elapsed() >= MAX_IDLE_TIMEOUT {
            tracing::info!("service log builder for {index_id} idle, removing it");
            return false;
        }
        true
    });
    flushed
}

async fn send_log_batches(
    batches: Vec<(String, RecordBatch)>,
    tx: &mpsc::Sender<(String, RecordBatch)>,
    cancel: &CancellationToken,
) -> bool {
    for (index_id, batch) in batches {
        if !send_log_batch(index_id, batch, tx, cancel).await {
            return false;
        }
    }
    true
}

async fn flush_final_batches(batchers: &Batchers, tx: &mpsc::Sender<(String, RecordBatch)>) {
    let open = CancellationToken::new();
    flush_slot(&batchers.access, batchers, tx, &open, |_| true).await;
    flush_slot(&batchers.event, batchers, tx, &open, |_| true).await;
    flush_slot(&batchers.message, batchers, tx, &open, |_| true).await;
    send_log_batches(drain_log_batches(batchers, true), tx, &open).await;
}

async fn sweeping_task(
    batchers: Arc<Batchers>,
    tx: mpsc::Sender<(String, RecordBatch)>,
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => {
                break;
            }
            () = tokio::time::sleep(Duration::from_secs(1)) => {
                if !flush_slot(&batchers.access, &batchers, &tx, &cancel, stale).await
                    || !flush_slot(&batchers.event, &batchers, &tx, &cancel, stale).await
                    || !flush_slot(&batchers.message, &batchers, &tx, &cancel, stale).await
                    || !send_log_batches(
                        drain_log_batches(&batchers, false),
                        &tx,
                        &cancel,
                    )
                    .await
                {
                    break;
                }
            }
        }
    }
}
