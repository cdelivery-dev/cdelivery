pub(crate) mod acme;
pub(crate) mod app;
pub(crate) mod channel;
pub(crate) mod docker;
pub(crate) mod event;
pub(crate) mod group;
pub(crate) mod insight;
pub(crate) mod instance;
pub(crate) mod keyring;
pub(crate) mod label;
pub(crate) mod llm;
pub(crate) mod message;
pub(crate) mod node;
pub(crate) mod pingress;
pub(crate) mod platform;
pub(crate) mod release;
pub(crate) mod setting;
pub(crate) mod user;
pub(crate) mod volume;

use std::sync::LazyLock;

use tokio::io::AsyncWriteExt as _;
use tokio_util::sync::CancellationToken;

use crate::config;

struct Records {
    sender: tokio::sync::mpsc::Sender<Vec<u8>>,
    receiver: parking_lot::Mutex<Option<tokio::sync::mpsc::Receiver<Vec<u8>>>>,
}

static RECORDS: LazyLock<Records> = LazyLock::new(|| {
    let (sender, receiver) = tokio::sync::mpsc::channel(config::RECORD_QUEUE);
    Records {
        sender,
        receiver: parking_lot::Mutex::new(Some(receiver)),
    }
});

pub(crate) fn record<T: serde::Serialize>(index_id: &'static str, payload: T) {
    let mut line = serde_json::json!(insight_wire::Indexed::new(index_id, payload))
        .to_string()
        .into_bytes();
    line.push(b'\n');
    if RECORDS.sender.try_send(line).is_err() {
        tracing::warn!("{index_id} record dropped, stdout queue full or closed");
    }
}

pub(crate) async fn write_records(cancel: CancellationToken) {
    let Some(mut receiver) = RECORDS.receiver.lock().take() else {
        return;
    };
    let mut stdout = tokio::io::stdout();
    loop {
        let next = tokio::select! {
            biased;
            line = receiver.recv() => line,
            () = cancel.cancelled() => receiver.try_recv().ok(),
        };
        let Some(line) = next else {
            break;
        };
        if let Err(error) = stdout.write_all(&line).await {
            tracing::warn!(error = ?error, "record stdout write failed");
        }
    }
    if let Err(error) = stdout.flush().await {
        tracing::warn!(error = ?error, "record stdout flush failed");
    }
}
