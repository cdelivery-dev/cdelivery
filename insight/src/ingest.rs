use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use bytes::{Buf, Bytes};
use datafusion::arrow::array::RecordBatch;
use datafusion::arrow::ipc::reader::StreamReader;
use futures_util::StreamExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinSet;
use tokio::time::{Instant, interval_at, timeout};
use tokio_util::codec::{FramedRead, LengthDelimitedCodec};
use tokio_util::task::TaskTracker;

use crate::config;
use crate::error::Result;
use crate::storage::store::{PARQUET_MAX_ROW_GROUP_ROWS, Store};
use insight_schema::{IndexKind, TIMESTAMP, timestamp_within_window};

const CHANNEL_CAPACITY: usize = 1024;
const IPC_TO_PARQUET_SIZE_RATIO: usize = 8;

type Payload = (Bytes, tokio::sync::OwnedSemaphorePermit);
type Router = scc::HashMap<String, mpsc::Sender<Payload>>;

async fn accept_retry(listener: &TcpListener) -> (TcpStream, std::net::SocketAddr) {
    loop {
        match listener.accept().await {
            Ok(accepted) => return accepted,
            Err(e) => {
                tracing::warn!(error = ?e, "ingest listener accept failed, retrying");
                tokio::time::sleep(config::ACCEPT_RETRY_DELAY.unsigned_abs()).await;
            }
        }
    }
}

fn drop_out_of_window_rows(
    out_of_window: &AtomicU64,
    batch: &RecordBatch,
    now_nanos: i64,
) -> RecordBatch {
    let Some((idx, _)) = batch.schema().column_with_name(TIMESTAMP) else {
        return batch.clone();
    };
    let Some(ts) = batch
        .column(idx)
        .as_any()
        .downcast_ref::<datafusion::arrow::array::TimestampNanosecondArray>()
    else {
        return batch.clone();
    };
    let keep: datafusion::arrow::array::BooleanArray = ts
        .values()
        .iter()
        .map(|v| timestamp_within_window(*v, now_nanos))
        .collect();
    let violations = keep.false_count() as u64;
    if violations == 0 {
        return batch.clone();
    }
    logging::rate_limited_warn(
        out_of_window,
        violations,
        "dropping ingested rows with timestamps outside acceptable window",
    );
    datafusion::arrow::compute::filter_record_batch(batch, &keep).unwrap_or_else(|e| {
        tracing::error!(error = ?e, "failed to drop out-of-window rows, forwarding batch as is");
        batch.clone()
    })
}

fn close_and_drain(rx: &mut mpsc::Receiver<Payload>) -> Vec<Payload> {
    rx.close();
    let mut drained = Vec::new();
    while let Ok(payload) = rx.try_recv() {
        drained.push(payload);
    }
    drained
}

fn remove_entry(router: &Router, index_id: &str, tx: &mpsc::Sender<Payload>) {
    let _ = router.remove_if_sync(index_id, |stored| stored.same_channel(tx));
}

#[derive(Default)]
struct WriterState {
    roll_batches: Vec<RecordBatch>,
    total_rows: usize,
    total_ipc_bytes: usize,
    held_permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

impl WriterState {
    fn absorb(
        &mut self,
        out_of_window: &AtomicU64,
        cpu: &crate::cpu::Cache,
        index_id: &str,
        target_schema: &Arc<datafusion::arrow::datatypes::Schema>,
        ipc_bytes: Bytes,
        permit: tokio::sync::OwnedSemaphorePermit,
    ) {
        let real_bytes_size = ipc_bytes.len();
        let cursor = std::io::Cursor::new(ipc_bytes);
        let reader = match StreamReader::try_new(cursor, None) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = ?e, "writer {index_id} failed to create IPC stream reader");
                return;
            }
        };
        let mut found_any = false;
        let mut payload_rows = 0;
        let now_nanos = insight_schema::now_nanos();
        for batch_result in reader {
            let batch = match batch_result {
                Ok(b) => b,
                Err(e) => {
                    tracing::error!(error = ?e, "writer {index_id} failed to read batch from IPC stream");
                    continue;
                }
            };
            if batch.schema().as_ref() != target_schema.as_ref() {
                tracing::warn!("writer {index_id} dropping batch with incompatible schema");
                continue;
            }
            let batch = drop_out_of_window_rows(out_of_window, &batch, now_nanos);
            found_any = true;
            if batch.num_rows() == 0 {
                continue;
            }
            let recorded = match index_id {
                insight_wire::CONTAINER_METRIC_INDEX_ID => cpu.record_containers(&batch),
                insight_wire::HOST_METRIC_INDEX_ID => cpu.record_nodes(&batch),
                _ => Ok(()),
            };
            if let Err(error) = recorded {
                tracing::warn!(error = ?error, "latest CPU sample update failed");
            }
            payload_rows += batch.num_rows();
            self.roll_batches.push(batch);
        }

        if !found_any {
            tracing::warn!("writer {index_id} found no batch in IPC stream");
            return;
        }
        if payload_rows == 0 {
            return;
        }

        self.total_rows += payload_rows;
        self.total_ipc_bytes += real_bytes_size;
        match &mut self.held_permit {
            Some(h) => h.merge(permit),
            None => self.held_permit = Some(permit),
        }
    }

    fn clear(&mut self) {
        self.roll_batches.clear();
        self.total_rows = 0;
        self.total_ipc_bytes = 0;
        self.held_permit = None;
    }
}

#[derive(Debug)]
pub struct Ingest {
    store: Arc<Store>,
    writer_flush_interval: jiff::SignedDuration,
    memory: Arc<tokio::sync::Semaphore>,
    router: Router,
    out_of_window: AtomicU64,
}

impl Ingest {
    pub fn new(
        store: Arc<Store>,
        writer_flush_interval: jiff::SignedDuration,
        memory_budget: usize,
    ) -> Self {
        Self {
            store,
            writer_flush_interval,
            memory: Arc::new(tokio::sync::Semaphore::new(memory_budget)),
            router: Router::new(),
            out_of_window: AtomicU64::new(0),
        }
    }

    pub async fn listen(
        self: Arc<Self>,
        listener: TcpListener,
        shutdown: impl Future<Output = ()>,
    ) {
        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let mut client_tasks = JoinSet::new();
        let writer_tasks = TaskTracker::new();

        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                biased;
                () = &mut shutdown => {
                    break;
                }
                Some(res) = client_tasks.join_next() => {
                    if let Err(e) = res {
                        tracing::error!(error = ?e, "ingest client task aborted");
                    }
                }
                (stream, addr) = accept_retry(&listener) => {
                    let shutdown_rx = shutdown_tx.subscribe();
                    let writer_tasks = writer_tasks.clone();
                    let ingest = Arc::clone(&self);
                    client_tasks.spawn(async move {
                        let _ = stream.set_nodelay(true);
                        if let Err(e) = ingest.handle_connection(stream, shutdown_rx, &writer_tasks).await {
                            tracing::warn!(error = ?e, "agent {addr} disconnected with error");
                        }
                    });
                }
            }
        }

        let _ = shutdown_tx.send(());
        while let Some(res) = client_tasks.join_next().await {
            if let Err(e) = res {
                tracing::error!(error = ?e, "ingest client task aborted");
            }
        }
        self.router.clear_sync();
        writer_tasks.close();
        writer_tasks.wait().await;
        tracing::info!("ingest listener shut down cleanly");
    }

    async fn handle_connection(
        self: Arc<Self>,
        stream: TcpStream,
        mut shutdown_rx: broadcast::Receiver<()>,
        writer_tasks: &TaskTracker,
    ) -> Result<()> {
        let codec = LengthDelimitedCodec::builder()
            .length_field_length(4)
            .max_frame_length(insight_wire::frame::MAX_LEN)
            .new_codec();

        let mut transport = FramedRead::new(stream, codec);
        let idle_timeout = config::WRITER_IDLE_TIMEOUT.unsigned_abs();

        loop {
            tokio::select! {
                biased;
                _ = shutdown_rx.recv() => {
                    break;
                }
                res = timeout(idle_timeout, transport.next()) => {
                    let Ok(frame_res) = res else {
                        tracing::info!("agent idle timeout");
                        break;
                    };
                    let mut frame = match frame_res {
                        Some(Ok(f)) => f,
                        Some(Err(e)) => {
                            tracing::warn!(error = ?e, "failed to read frame from agent");
                            return Err(e.into());
                        }
                        None => {
                            break;
                        }
                    };

                    let Some((index_id, header_len)) = insight_wire::frame::parse_header(&frame) else {
                        tracing::warn!("unrecognized ingest frame header, closing connection");
                        break;
                    };
                    let index_id = index_id.to_owned();
                    frame.advance(header_len);
                    let Some(index_kind) = IndexKind::of(&index_id) else {
                        tracing::warn!("unsupported index {index_id} from agent, closing connection");
                        break;
                    };
                    let target_schema = index_kind.schema();

                    let ipc_payload: Bytes = frame.freeze();
                    if !self
                        .enqueue(
                            index_id,
                            target_schema,
                            ipc_payload,
                            writer_tasks,
                            &mut shutdown_rx,
                        )
                        .await
                    {
                        break;
                    }
                }
            }
        }

        Ok(())
    }

    async fn enqueue(
        self: &Arc<Self>,
        index_id: String,
        target_schema: Arc<datafusion::arrow::datatypes::Schema>,
        ipc_payload: Bytes,
        writer_tasks: &TaskTracker,
        shutdown_rx: &mut broadcast::Receiver<()>,
    ) -> bool {
        let payload_size: u32 = ipc_payload.len().try_into().unwrap_or(u32::MAX);
        let permit = tokio::select! {
            biased;
            _ = shutdown_rx.recv() => return false,
            permit = self.memory.clone().acquire_many_owned(payload_size) => permit,
        };
        let Ok(permit) = permit else {
            tracing::warn!("memory semaphore closed, stopping reading from agent");
            return false;
        };

        let mut payload = (ipc_payload, permit);
        for attempt in 0..2 {
            let tx = self
                .router
                .entry_sync(index_id.clone())
                .or_insert_with(|| {
                    let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
                    let ingest = Arc::clone(self);
                    let index_id = index_id.clone();
                    let target_schema = Arc::clone(&target_schema);
                    let identity_tx = tx.downgrade();
                    drop(writer_tasks.spawn(async move {
                        ingest
                            .writer_actor(index_id, target_schema, rx, identity_tx)
                            .await;
                    }));
                    tx
                })
                .get()
                .clone();
            let sent = tokio::select! {
                biased;
                _ = shutdown_rx.recv() => return false,
                sent = tx.send(payload) => sent,
            };
            match sent {
                Ok(()) => break,
                Err(e) => {
                    remove_entry(&self.router, &index_id, &tx);
                    if attempt == 0 {
                        payload = e.0;
                    } else {
                        tracing::warn!(
                            "dropping payload for {index_id}: writer channel closed twice in a row"
                        );
                        break;
                    }
                }
            }
        }
        true
    }

    async fn writer_actor(
        self: Arc<Self>,
        index_id: String,
        target_schema: Arc<datafusion::arrow::datatypes::Schema>,
        mut rx: mpsc::Receiver<Payload>,
        identity_tx: mpsc::WeakSender<Payload>,
    ) {
        let flush_interval = self.writer_flush_interval.unsigned_abs();
        let mut state = WriterState::default();
        let mut flush_timer = interval_at(Instant::now() + flush_interval, flush_interval);
        let idle_timeout = config::WRITER_IDLE_TIMEOUT.unsigned_abs();
        let idle_timer = tokio::time::sleep(idle_timeout);
        tokio::pin!(idle_timer);
        loop {
            tokio::select! {
                msg = rx.recv() => {
                    let Some((ipc_bytes, permit)) = msg else {
                        break;
                    };
                    idle_timer.as_mut().reset(Instant::now() + idle_timeout);
                    state.absorb(&self.out_of_window, &self.store.cpu, &index_id, &target_schema, ipc_bytes, permit);
                    self.store.set_pending(&index_id, &state.roll_batches);

                    if state.total_rows >= PARQUET_MAX_ROW_GROUP_ROWS
                        || state.total_ipc_bytes
                            >= self.store.row_group_max_bytes() * IPC_TO_PARQUET_SIZE_RATIO
                    {
                        self.flush_and_handle(&index_id, &mut state).await;
                    }
                }

                _ = flush_timer.tick() => {
                    if !state.roll_batches.is_empty() {
                        self.flush_and_handle(&index_id, &mut state).await;
                    }
                    flush_timer.reset();
                }

                () = &mut idle_timer => {
                    tracing::info!("writer {index_id} idle for {idle_timeout:?}, shutting down");
                    if let Some(identity) = identity_tx.upgrade() {
                        remove_entry(&self.router, &index_id, &identity);
                    }
                    for (ipc_bytes, permit) in close_and_drain(&mut rx) {
                        state.absorb(&self.out_of_window, &self.store.cpu, &index_id, &target_schema, ipc_bytes, permit);
                    }
                    break;
                }
            }
        }

        if !state.roll_batches.is_empty()
            && let Err(e) = self.flush_once(&index_id, &mut state).await
        {
            tracing::error!(
                error = ?e,
                "writer {index_id} failed to flush {} final rows, dropping them",
                state.total_rows
            );
        }
        self.store.set_pending(&index_id, &[]);
    }

    async fn flush_and_handle(&self, index_id: &str, state: &mut WriterState) {
        if let Err(e) = self.flush_once(index_id, state).await {
            let delay = config::FLUSH_RETRY_DELAY.unsigned_abs();
            tracing::warn!(
                error = ?e,
                "writer {index_id} failed to flush batch, retrying once after {delay:?}"
            );
            tokio::time::sleep(delay).await;
            if let Err(e) = self.flush_once(index_id, state).await {
                tracing::error!(
                    error = ?e,
                    "writer {index_id} failed to flush {} buffered rows twice, dropping them",
                    state.total_rows,
                );
                state.clear();
                self.store.set_pending(index_id, &[]);
            }
        }
    }

    async fn flush_once(&self, index_id: &str, state: &mut WriterState) -> Result<()> {
        self.store.flush(index_id, &state.roll_batches).await?;
        state.clear();
        self.store.set_pending(index_id, &[]);
        Ok(())
    }
}
