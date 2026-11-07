use std::sync::Arc;

use jiff::Timestamp;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::error::Result;
use crate::{concept, config};

pub(crate) async fn run<F, Fut>(rest: Arc<super::Rest>, cancel: CancellationToken, on_inbound: F)
where
    F: Fn(concept::message::Inbound) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (inbound, worker) = super::super::spawn_inbound(on_inbound);
    let mut offset = 0;
    loop {
        let ended = tokio::select! {
            () = cancel.cancelled() => break,
            ended = session(&rest, &mut offset, &inbound) => ended,
        };
        if let Err(e) = ended {
            tracing::warn!(error = ?e, "telegram long poll session ended");
        }
        if inbound.is_closed() {
            break;
        }
        tokio::select! {
            () = cancel.cancelled() => break,
            () = tokio::time::sleep(config::AGENT_RETRY.unsigned_abs()) => {}
        }
    }
    drop(inbound);
    super::super::drain(super::PLATFORM, worker).await;
}

async fn session(
    rest: &super::Rest,
    offset: &mut u64,
    inbound: &tokio::sync::mpsc::Sender<concept::message::Inbound>,
) -> Result<()> {
    let identity = rest.identity().await?;
    rest.delete_webhook().await?;
    tracing::info!("telegram long poll ready");
    loop {
        let mut updates = rest.get_updates(*offset).await?;
        updates.sort_unstable_by_key(|update| update.update_id);
        for update in updates {
            if update.update_id < *offset {
                continue;
            }
            let update_id = update.update_id;
            if let Some(message) = gate(update, &identity) {
                let event_id = message.event_id().clone();
                if inbound.send(message).await.is_err() {
                    return Ok(());
                }
                tracing::info!("telegram text event {event_id} accepted");
            }
            *offset = update_id.saturating_add(1);
        }
    }
}

fn gate(update: Update, identity: &super::Identity) -> Option<concept::message::Inbound> {
    let message = update.message?;
    if message.message_thread_id.is_some() {
        return None;
    }
    let sender = message.from?;
    if sender.is_bot || sender.id <= 0 {
        return None;
    }
    let timestamp = Timestamp::from_second(message.date)
        .inspect_err(|e| {
            tracing::debug!(error = ?e, "telegram message timestamp invalid, dropped");
        })
        .ok()?;
    let text = message.text?;
    let (group_chat_id, text) = match message.chat.kind.as_str() {
        "private" => (None, text.trim().to_owned()),
        "group" | "supergroup" => {
            let reply_to_bot = message
                .reply_to_message
                .as_ref()
                .and_then(|reply| reply.from.as_ref())
                .is_some_and(|user| user.id == identity.id);
            let text = if reply_to_bot {
                text.trim().to_owned()
            } else {
                command_text(&text, &identity.username)?
            };
            (Some(message.chat.id.to_string()), text)
        }
        _ => return None,
    };
    if text.is_empty() {
        return None;
    }
    Some(concept::message::Inbound::new(
        format!("{}:{}", identity.id, update.update_id),
        group_chat_id,
        sender.id.to_string(),
        super::super::super::event::truncate_bytes(&text, config::AGENT_INBOUND_TEXT_MAX)
            .to_owned(),
        timestamp,
    ))
}

fn command_text(text: &str, username: &str) -> Option<String> {
    let (command, text) = text.trim().split_once(char::is_whitespace)?;
    let expected = format!("/cdelivery@{username}");
    command
        .eq_ignore_ascii_case(&expected)
        .then(|| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

#[derive(Deserialize)]
pub(super) struct Update {
    update_id: u64,
    message: Option<Incoming>,
}

#[derive(Deserialize)]
struct Incoming {
    date: i64,
    chat: Chat,
    from: Option<super::User>,
    text: Option<String>,
    message_thread_id: Option<i64>,
    reply_to_message: Option<Reply>,
}

#[derive(Deserialize)]
struct Chat {
    id: i64,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct Reply {
    from: Option<super::User>,
}
