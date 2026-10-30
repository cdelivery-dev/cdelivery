use std::collections::HashMap;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use jiff::Timestamp;
use prost::Message as _;
use serde::Deserialize;
use tokio_tungstenite::tungstenite;
use tokio_util::sync::CancellationToken;

use crate::error::{ErrorKind, Result};
use crate::{concept, config};

const CONTROL: i32 = 0;
const DATA: i32 = 1;
const ACK_PAYLOAD: &[u8] = br#"{"code":200,"headers":null,"data":null}"#;

#[derive(Clone, PartialEq, prost::Message)]
struct Header {
    #[prost(string, required, tag = "1")]
    key: String,
    #[prost(string, required, tag = "2")]
    value: String,
}

#[derive(Clone, PartialEq, prost::Message)]
struct Frame {
    #[prost(uint64, required, tag = "1")]
    seq_id: u64,
    #[prost(uint64, required, tag = "2")]
    log_id: u64,
    #[prost(int32, required, tag = "3")]
    service: i32,
    #[prost(int32, required, tag = "4")]
    method: i32,
    #[prost(message, repeated, tag = "5")]
    headers: Vec<Header>,
    #[prost(string, optional, tag = "6")]
    payload_encoding: Option<String>,
    #[prost(string, optional, tag = "7")]
    payload_type: Option<String>,
    #[prost(bytes = "vec", optional, tag = "8")]
    payload: Option<Vec<u8>>,
    #[prost(string, optional, tag = "9")]
    log_id_new: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub(super) struct ClientConfig {
    #[serde(rename = "ReconnectInterval")]
    reconnect_interval: u64,
    #[serde(rename = "ReconnectNonce")]
    reconnect_nonce: u64,
    #[serde(rename = "PingInterval")]
    ping_interval: u64,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            reconnect_interval: 120,
            reconnect_nonce: 30,
            ping_interval: 120,
        }
    }
}

pub(crate) async fn run<F, Fut>(rest: Arc<super::Rest>, cancel: CancellationToken, on_inbound: F)
where
    F: Fn(concept::message::Inbound) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (inbound, worker) = super::super::spawn_inbound(on_inbound);
    let platform = rest.platform;
    let mut conf = ClientConfig::default();
    let mut dedup = super::super::dedup::Dedup::new(config::AGENT_EVENT_DEDUP);
    let mut failures = 0u32;
    loop {
        let ended = tokio::select! {
            () = cancel.cancelled() => break,
            ended = session(&rest, &mut conf, &mut dedup, &mut failures, &inbound) => ended,
        };
        if let Err(e) = ended {
            tracing::warn!(error = ?e, "{platform} ws session ended");
        }
        let wait = if failures == 0 {
            super::super::jitter(conf.reconnect_nonce)
        } else {
            seconds(conf.reconnect_interval)
        };
        failures = failures.saturating_add(1);
        tokio::select! {
            () = cancel.cancelled() => break,
            () = tokio::time::sleep(wait) => {}
        }
    }
    drop(inbound);
    super::super::drain(platform, worker).await;
}

async fn session(
    rest: &super::Rest,
    conf: &mut ClientConfig,
    dedup: &mut super::super::dedup::Dedup,
    failures: &mut u32,
    inbound: &tokio::sync::mpsc::Sender<concept::message::Inbound>,
) -> Result<()> {
    let platform = rest.platform;
    let (url, refreshed) = discover(rest).await?;
    if let Some(refreshed) = refreshed {
        *conf = refreshed;
    }
    let bot = rest.bot_open_id().await?;
    let (socket, _) = tokio_tungstenite::connect_async(url.as_str()).await?;
    tracing::info!("{platform} ws connected");
    let service = service_id(&url);
    let (mut sink, mut socket) = socket.split();
    let mut assembler = Assembler::default();
    let mut ping_timer = tokio::time::interval(seconds(conf.ping_interval));
    loop {
        tokio::select! {
            _ = ping_timer.tick() => {
                sink.send(tungstenite::Message::binary(ping(service).encode_to_vec())).await?;
            }
            incoming = socket.next() => {
                let incoming = incoming.ok_or(ErrorKind::Ws(tungstenite::Error::ConnectionClosed))??;
                let tungstenite::Message::Binary(bytes) = incoming else { continue };
                let frame = match decode(bytes) {
                    Ok(frame) => frame,
                    Err(e) => {
                        tracing::warn!(error = ?e, "{platform} ws frame decode failed, skipped");
                        continue;
                    }
                };
                *failures = 0;
                match frame.method {
                    CONTROL => {
                        if let Some(refreshed) = pong_config(&frame, platform) {
                            if refreshed.ping_interval != conf.ping_interval {
                                ping_timer = tokio::time::interval(seconds(refreshed.ping_interval));
                            }
                            *conf = refreshed;
                        }
                    }
                    DATA => {
                        tracing::debug!(
                            "{platform} ws data frame received, type {}, message {}, part {}/{}",
                            header(&frame, "type").unwrap_or("unknown"),
                            header(&frame, "message_id").unwrap_or("unknown"),
                            header(&frame, "seq").unwrap_or("unknown"),
                            header(&frame, "sum").unwrap_or("unknown")
                        );
                        if let Some((ack, payload)) = accept(frame, &mut assembler, Timestamp::now(), platform) {
                            if let Some(message) = gate(&payload, &bot, platform) {
                                dedup.admit(inbound, message, platform).await?;
                            }
                            sink.send(tungstenite::Message::binary(ack.encode_to_vec())).await?;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

pub(super) async fn discover(rest: &super::Rest) -> Result<(String, Option<ClientConfig>)> {
    let body = rest
        .client
        .post(format!("{}/callback/ws/endpoint", rest.base))
        .json(&serde_json::json!({"AppID": rest.app_id, "AppSecret": rest.app_secret}))
        .send()
        .await?
        .bytes()
        .await?;
    parse_endpoint(rest.platform, &body)
}

fn parse_endpoint(platform: &'static str, body: &[u8]) -> Result<(String, Option<ClientConfig>)> {
    #[derive(Deserialize)]
    struct Body {
        code: i64,
        #[serde(default)]
        msg: String,
        data: Option<Data>,
    }
    #[derive(Deserialize)]
    struct Data {
        #[serde(rename = "URL", default)]
        url: String,
        #[serde(rename = "ClientConfig")]
        client_config: Option<ClientConfig>,
    }
    let parsed: Body = serde_json::from_slice(body)?;
    if parsed.code != 0 {
        return Err(ErrorKind::im_api(platform, format!("{} {}", parsed.code, parsed.msg)).into());
    }
    let Some(data) = parsed.data.filter(|data| !data.url.is_empty()) else {
        return Err(ErrorKind::im_api(platform, "ws endpoint response has no URL").into());
    };
    Ok((data.url, data.client_config))
}

#[derive(Deserialize)]
struct Envelope {
    header: EnvelopeHeader,
    #[serde(default)]
    event: serde_json::Value,
}

#[derive(Deserialize)]
struct EnvelopeHeader {
    event_id: String,
    event_type: String,
}

#[derive(Deserialize)]
struct Received {
    sender: Sender,
    message: Incoming,
}

#[derive(Deserialize)]
struct Sender {
    sender_id: SenderId,
}

#[derive(Deserialize)]
struct SenderId {
    open_id: String,
}

#[derive(Deserialize)]
struct Incoming {
    chat_id: String,
    chat_type: String,
    message_type: String,
    content: String,
    create_time: String,
    #[serde(default)]
    mentions: Vec<Mention>,
}

#[derive(Deserialize)]
struct Mention {
    key: String,
    id: MentionId,
}

#[derive(Deserialize)]
struct MentionId {
    open_id: String,
}

#[derive(Deserialize)]
struct Content {
    text: String,
}

fn gate(payload: &[u8], bot: &str, platform: &'static str) -> Option<concept::message::Inbound> {
    let envelope: Envelope = match serde_json::from_slice(payload) {
        Ok(envelope) => envelope,
        Err(e) => {
            tracing::debug!(error = ?e, "{platform} event unparseable, dropped");
            return None;
        }
    };
    if envelope.header.event_type != "im.message.receive_v1" {
        tracing::debug!(
            "{platform} event type {} ignored",
            envelope.header.event_type
        );
        return None;
    }
    if envelope.header.event_id.is_empty() {
        tracing::warn!("{platform} message event without event_id dropped");
        return None;
    }
    let received: Received = match serde_json::from_value(envelope.event) {
        Ok(received) => received,
        Err(e) => {
            tracing::debug!(error = ?e, "{platform} message event unparseable, dropped");
            return None;
        }
    };
    if received.message.message_type != "text" {
        tracing::debug!(
            "{platform} message type {} ignored",
            received.message.message_type
        );
        return None;
    }
    let timestamp = received
        .message
        .create_time
        .parse::<i64>()
        .ok()
        .and_then(|value| Timestamp::from_millisecond(value).ok());
    let Some(timestamp) = timestamp else {
        tracing::debug!(
            "{platform} message create_time {} invalid, dropped",
            received.message.create_time
        );
        return None;
    };
    let content: Content = match serde_json::from_str(&received.message.content) {
        Ok(content) => content,
        Err(e) => {
            tracing::debug!(error = ?e, "{platform} text content unparseable, dropped");
            return None;
        }
    };
    let text = match received.message.chat_type.as_str() {
        "p2p" => content.text,
        "group" => {
            if !received
                .message
                .mentions
                .iter()
                .any(|mention| mention.id.open_id == bot)
            {
                tracing::debug!("{platform} group message without bot mention, dropped");
                return None;
            }
            received
                .message
                .mentions
                .iter()
                .fold(content.text, |text, mention| text.replace(&mention.key, ""))
                .trim()
                .to_owned()
        }
        other => {
            tracing::debug!("{platform} chat type {other} ignored");
            return None;
        }
    };
    Some(concept::message::Inbound::new(
        envelope.header.event_id,
        (received.message.chat_type == "group").then_some(received.message.chat_id),
        received.sender.sender_id.open_id,
        super::super::super::event::truncate_bytes(&text, config::AGENT_INBOUND_TEXT_MAX)
            .to_owned(),
        timestamp,
    ))
}

fn accept(
    mut frame: Frame,
    assembler: &mut Assembler,
    now: Timestamp,
    platform: &'static str,
) -> Option<(Frame, Vec<u8>)> {
    if header(&frame, "type") != Some("event") {
        tracing::debug!("{platform} ws data frame without event type, ignored");
        return None;
    }
    let payload = frame.payload.take().unwrap_or_default();
    let sum = header_count(&frame, "sum");
    let payload = if sum > 1 {
        let id = header(&frame, "message_id").unwrap_or_default().to_owned();
        let seq = header_count(&frame, "seq");
        assembler.add(now, &id, sum, seq, payload)?
    } else {
        payload
    };
    Some((ack(frame), payload))
}

fn ack(mut frame: Frame) -> Frame {
    frame.payload = Some(ACK_PAYLOAD.to_vec());
    frame
}

fn ping(service: i32) -> Frame {
    Frame {
        service,
        method: CONTROL,
        headers: vec![Header {
            key: "type".to_owned(),
            value: "ping".to_owned(),
        }],
        ..Frame::default()
    }
}

fn pong_config(frame: &Frame, platform: &'static str) -> Option<ClientConfig> {
    if header(frame, "type") != Some("pong") {
        return None;
    }
    let payload = frame
        .payload
        .as_deref()
        .filter(|payload| !payload.is_empty())?;
    match serde_json::from_slice(payload) {
        Ok(refreshed) => Some(refreshed),
        Err(e) => {
            tracing::warn!(error = ?e, "{platform} ws pong config unparseable, kept previous");
            None
        }
    }
}

fn header<'a>(frame: &'a Frame, key: &str) -> Option<&'a str> {
    frame
        .headers
        .iter()
        .find(|header| header.key == key)
        .map(|header| header.value.as_str())
}

fn header_count(frame: &Frame, key: &str) -> usize {
    header(frame, key)
        .and_then(|value| value.parse().ok())
        .unwrap_or_default()
}

fn decode(bytes: bytes::Bytes) -> Result<Frame> {
    Ok(Frame::decode(bytes)?)
}

fn service_id(url: &str) -> i32 {
    url.split_once('?')
        .map_or("", |(_, query)| query)
        .split('&')
        .find_map(|pair| pair.strip_prefix("service_id="))
        .and_then(|value| value.parse().ok())
        .unwrap_or_default()
}

fn seconds(value: u64) -> std::time::Duration {
    std::time::Duration::from_secs(value.max(1))
}

#[derive(Default)]
struct Assembler {
    pending: HashMap<String, Partial>,
}

struct Partial {
    born: Timestamp,
    parts: Vec<Option<Vec<u8>>>,
}

impl Assembler {
    fn add(
        &mut self,
        now: Timestamp,
        id: &str,
        sum: usize,
        seq: usize,
        payload: Vec<u8>,
    ) -> Option<Vec<u8>> {
        if sum > config::AGENT_FEISHU_FRAGMENT_MAX || seq >= sum {
            return None;
        }
        self.pending.retain(|_, partial| {
            now.duration_since(partial.born) < config::AGENT_FEISHU_FRAGMENT_TTL
        });
        let partial = self
            .pending
            .entry(id.to_owned())
            .or_insert_with(|| Partial {
                born: now,
                parts: vec![None; sum],
            });
        *partial.parts.get_mut(seq)? = Some(payload);
        if partial.parts.iter().any(Option::is_none) {
            return None;
        }
        let partial = self.pending.remove(id)?;
        Some(partial.parts.into_iter().flatten().flatten().collect())
    }
}
