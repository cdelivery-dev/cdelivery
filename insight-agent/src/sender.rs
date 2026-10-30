use std::collections::VecDeque;

use arrow::ipc::writer::StreamWriter;
use arrow::record_batch::RecordBatch;
use bytes::Bytes;
use futures_util::SinkExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::Duration;
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tokio_util::task::AbortOnDropHandle;

const CONNECT_RETRY_INTERVAL: Duration = Duration::from_secs(5);
const NETWORK_IO_TIMEOUT: Duration = Duration::from_secs(10);

fn encode_ingest_frame(
    buf: &mut Vec<u8>,
    index_id: &str,
    batch: &RecordBatch,
) -> std::result::Result<(), arrow::error::ArrowError> {
    buf.clear();
    insight_wire::frame::write_header(buf, index_id);
    let mut writer = StreamWriter::try_new(buf, &batch.schema())?;
    writer.write(batch)?;
    writer.finish()
}

pub(crate) async fn send_worker(addr: String, mut rx: mpsc::Receiver<(String, RecordBatch)>) {
    let mut pending = VecDeque::<(String, RecordBatch)>::new();
    let mut buf = Vec::with_capacity(1024 * 1024);

    loop {
        if pending.is_empty() {
            match rx.recv().await {
                Some(batch) => pending.push_back(batch),
                None => return,
            }
        }
        let connect = tokio::time::timeout(NETWORK_IO_TIMEOUT, TcpStream::connect(&addr))
            .await
            .map_err(std::io::Error::from)
            .and_then(|res| res);
        let stream = match connect {
            Ok(stream) => {
                let _ = stream.set_nodelay(true);
                stream
            }
            Err(error) => {
                tracing::warn!(
                    error = ?error,
                    "insight connection to {addr} failed; retrying in {CONNECT_RETRY_INTERVAL:?}"
                );
                tokio::time::sleep(CONNECT_RETRY_INTERVAL).await;
                continue;
            }
        };

        let codec = LengthDelimitedCodec::builder()
            .length_field_length(4)
            .max_frame_length(insight_wire::frame::MAX_LEN)
            .new_codec();
        let mut transport = Framed::new(stream, codec);

        loop {
            let (index_id, batch) = match pending.pop_front() {
                Some(batch) => batch,
                None => match rx.recv().await {
                    Some(b) => b,
                    None => return,
                },
            };

            if let Err(e) = encode_ingest_frame(&mut buf, &index_id, &batch) {
                tracing::error!(error = ?e, "serialization failed, dropping batch for {index_id}");
                continue;
            }

            if buf.len() > insight_wire::frame::MAX_LEN {
                let rows = batch.num_rows();
                if rows < 2 {
                    tracing::error!(
                        encoded_bytes = buf.len(),
                        "dropping oversized {index_id} batch with {rows} row"
                    );
                    continue;
                }
                let midpoint = rows / 2;
                pending.push_front((index_id.clone(), batch.slice(midpoint, rows - midpoint)));
                pending.push_front((index_id, batch.slice(0, midpoint)));
                continue;
            }

            let sent = tokio::time::timeout(
                NETWORK_IO_TIMEOUT,
                transport.send(Bytes::copy_from_slice(&buf)),
            )
            .await
            .map_err(std::io::Error::from)
            .and_then(|res| res);
            if let Err(error) = sent {
                tracing::warn!(
                    error = ?error,
                    "network send failed, preserving data and reconnecting"
                );
                pending.push_front((index_id, batch));
                break;
            }
        }
    }
}

pub(crate) async fn finish_sender(handle: AbortOnDropHandle<()>) {
    if let Err(e) = handle.await {
        tracing::error!(error = ?e, "insight sender task failed");
    }
}
