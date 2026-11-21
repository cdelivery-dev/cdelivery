use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use jiff::Timestamp;
use serde::Deserialize;
use tokio_tungstenite::tungstenite;
use tokio_util::sync::CancellationToken;

use crate::error::{ErrorKind, Result};
use crate::{concept, config};

pub(crate) async fn run<F, Fut>(rest: Arc<super::Rest>, cancel: CancellationToken, on_inbound: F)
where
    F: Fn(concept::message::Inbound) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (inbound, worker) = super::super::spawn_inbound(on_inbound);
    let mut dedup = super::super::dedup::Dedup::new(config::AGENT_EVENT_DEDUP);
    let mut failures = 0u32;
    loop {
        let ended = tokio::select! {
            () = cancel.cancelled() => break,
            ended = session(&rest, &mut dedup, &mut failures, &inbound) => ended,
        };
        if let Err(e) = ended {
            tracing::warn!(error = ?e, "slack ws session ended");
        }
        let wait = if failures == 0 {
            super::super::jitter(config::AGENT_SLACK_RECONNECT_NONCE)
        } else {
            config::AGENT_SLACK_RECONNECT.unsigned_abs()
        };
        failures = failures.saturating_add(1);
        tokio::select! {
            () = cancel.cancelled() => break,
            () = tokio::time::sleep(wait) => {}
        }
    }
    drop(inbound);
    super::super::drain(super::PLATFORM, worker).await;
}

async fn session(
    rest: &super::Rest,
    dedup: &mut super::super::dedup::Dedup,
    failures: &mut u32,
    inbound: &tokio::sync::mpsc::Sender<concept::message::Inbound>,
) -> Result<()> {
    let url = rest.connections_open().await?;
    let bot = rest.bot_user_id().await?;
    let (socket, _) = tokio_tungstenite::connect_async(url.as_str()).await?;
    tracing::info!("slack ws connected");
    let (mut sink, mut socket) = socket.split();
    let mut last_seen = Timestamp::now();
    let mut ping_timer = tokio::time::interval(config::AGENT_SLACK_PING.unsigned_abs());
    loop {
        tokio::select! {
            _ = ping_timer.tick() => {
                if Timestamp::now().duration_since(last_seen) > config::AGENT_SLACK_STALL {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "slack ws stalled",
                    )
                    .into());
                }
                sink.send(tungstenite::Message::Ping(bytes::Bytes::new())).await?;
            }
            incoming = socket.next() => {
                let incoming = incoming.ok_or(ErrorKind::Ws(tungstenite::Error::ConnectionClosed))??;
                last_seen = Timestamp::now();
                let tungstenite::Message::Text(raw) = incoming else { continue };
                let envelope: Envelope = match serde_json::from_str(raw.as_str()) {
                    Ok(envelope) => envelope,
                    Err(e) => {
                        tracing::warn!(error = ?e, "slack ws frame unparseable, skipped");
                        continue;
                    }
                };
                match envelope.kind.as_str() {
                    "hello" => {
                        *failures = 0;
                        tracing::info!("slack ws ready");
                    }
                    "disconnect" => {
                        tracing::info!(
                            "slack ws disconnect requested, reason {}",
                            envelope.reason.as_deref().unwrap_or("unknown")
                        );
                        return Ok(());
                    }
                    _ => {}
                }
                let Some(envelope_id) = envelope.id else { continue };
                if let Some(message) = gate(&envelope.kind, envelope.payload, &bot) {
                    dedup.admit(inbound, message, "slack").await?;
                }
                sink.send(tungstenite::Message::text(
                    serde_json::json!({"envelope_id": envelope_id}).to_string(),
                ))
                .await?;
            }
        }
    }
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "envelope_id")]
    id: Option<String>,
    #[serde(default)]
    payload: serde_json::Value,
    reason: Option<String>,
}

#[derive(Deserialize)]
struct Callback {
    event_id: String,
    event: Event,
}

#[derive(Deserialize)]
struct Event {
    #[serde(rename = "type")]
    kind: String,
    subtype: Option<String>,
    bot_id: Option<String>,
    #[serde(default)]
    user: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    channel: String,
    #[serde(default)]
    channel_type: String,
    #[serde(default)]
    ts: String,
}

fn gate(kind: &str, payload: serde_json::Value, bot: &str) -> Option<concept::message::Inbound> {
    if kind != "events_api" {
        return None;
    }
    let callback: Callback = match serde_json::from_value(payload) {
        Ok(callback) => callback,
        Err(e) => {
            tracing::debug!(error = ?e, "slack event unparseable, dropped");
            return None;
        }
    };
    if callback.event_id.is_empty() {
        tracing::warn!("slack event without event_id dropped");
        return None;
    }
    let event = callback.event;
    if event.subtype.is_some()
        || event.bot_id.is_some()
        || event.user.is_empty()
        || event.user == bot
    {
        tracing::debug!("slack event from bot or with subtype, dropped");
        return None;
    }
    let timestamp = event
        .ts
        .split('.')
        .next()
        .and_then(|seconds| seconds.parse::<i64>().ok())
        .and_then(|seconds| Timestamp::from_second(seconds).ok());
    let Some(timestamp) = timestamp else {
        tracing::debug!("slack event ts {} invalid, dropped", event.ts);
        return None;
    };
    let (group, text) = match (event.kind.as_str(), event.channel_type.as_str()) {
        ("message", "im") => (None, event.text),
        ("app_mention", _) => {
            if event.channel.is_empty() {
                tracing::debug!("slack mention without channel, dropped");
                return None;
            }
            (
                Some(event.channel),
                event.text.replace(&format!("<@{bot}>"), ""),
            )
        }
        (other, _) => {
            tracing::debug!("slack event type {other} ignored");
            return None;
        }
    };
    let text = normalize(&text);
    Some(concept::message::Inbound::new(
        callback.event_id,
        group,
        event.user,
        super::super::super::event::truncate_bytes(text.trim(), config::AGENT_INBOUND_TEXT_MAX)
            .to_owned(),
        timestamp,
    ))
}

fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        let (before, tail) = rest.split_at(start);
        out.push_str(before);
        let tail = tail.get(1..).unwrap_or_default();
        let Some(end) = tail.find('>') else {
            out.push('<');
            rest = tail;
            break;
        };
        let (token, after) = tail.split_at(end);
        out.push_str(&render(token));
        rest = after.get(1..).unwrap_or_default();
    }
    out.push_str(rest);
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn render(token: &str) -> String {
    let (head, label) = match token.split_once('|') {
        Some((head, label)) => (head, Some(label)),
        None => (token, None),
    };
    if let Some(id) = head.strip_prefix('@') {
        format!("@{}", label.unwrap_or(id))
    } else if let Some(name) = head.strip_prefix('#') {
        format!("#{}", label.unwrap_or(name))
    } else if let Some(command) = head.strip_prefix('!') {
        format!(
            "@{}",
            label.map_or(command, |label| label.trim_start_matches('@'))
        )
    } else {
        label.unwrap_or(head).to_owned()
    }
}
