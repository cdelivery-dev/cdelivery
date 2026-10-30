use std::sync::Arc;

use jiff::Timestamp;
use tokio_util::sync::CancellationToken;
use twilight_gateway::{Event, EventTypeFlags, Intents, Shard, ShardId, StreamExt as _};
use twilight_model::channel::message::MessageType;
use twilight_model::gateway::payload::incoming::MessageCreate;
use twilight_model::id::Id;
use twilight_model::id::marker::UserMarker;

use crate::error::{Error, Result};
use crate::{concept, config};

const EVENTS: EventTypeFlags = EventTypeFlags::MESSAGE_CREATE
    .union(EventTypeFlags::READY)
    .union(EventTypeFlags::RESUMED);

pub(crate) async fn run<F, Fut>(rest: Arc<super::Rest>, cancel: CancellationToken, on_inbound: F)
where
    F: Fn(concept::message::Inbound) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (inbound, worker) = super::super::spawn_inbound(on_inbound);
    loop {
        let ended = tokio::select! {
            () = cancel.cancelled() => break,
            ended = session(&rest, &inbound) => ended,
        };
        if let Err(e) = ended {
            tracing::warn!(error = ?e, "discord gateway session ended");
        } else {
            tracing::warn!("discord gateway session ended");
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
    inbound: &tokio::sync::mpsc::Sender<concept::message::Inbound>,
) -> Result<()> {
    let bot = rest.bot_user_id().await?;
    let intents = Intents::GUILD_MESSAGES | Intents::DIRECT_MESSAGES;
    let mut shard = Shard::new(ShardId::ONE, rest.token.clone(), intents);
    loop {
        let Some(event) = shard.next_event(EVENTS).await else {
            return Ok(());
        };
        let event = match event {
            Ok(event) => event,
            Err(source) => {
                let error = Error::from(source);
                tracing::warn!(error = ?error, "discord gateway receive failed");
                continue;
            }
        };
        match event {
            Event::Ready(_) => tracing::info!("discord gateway ready"),
            Event::Resumed => tracing::info!("discord gateway resumed"),
            Event::GatewayClose(frame) => {
                let code = frame.as_ref().map_or(1_005, |frame| frame.code);
                tracing::info!("discord gateway closed with code {code}");
            }
            Event::MessageCreate(message) => {
                let Some(message) = gate(*message, bot) else {
                    continue;
                };
                let event_id = message.event_id().clone();
                let wait = config::AGENT_IM_INBOUND_WAIT.unsigned_abs();
                match tokio::time::timeout(wait, inbound.send(message)).await {
                    Ok(Ok(())) => tracing::info!("discord text event {event_id} accepted"),
                    Ok(Err(_)) => return Ok(()),
                    Err(_) => {
                        tracing::warn!("discord inbound queue full, event {event_id} dropped");
                    }
                }
            }
            _ => {}
        }
    }
}

fn gate(message: MessageCreate, bot: Id<UserMarker>) -> Option<concept::message::Inbound> {
    let message = message.0;
    if !matches!(message.kind, MessageType::Regular | MessageType::Reply)
        || message.author.bot
        || message.author.system.unwrap_or(false)
        || message.author.id == bot
        || message.webhook_id.is_some()
    {
        return None;
    }
    let group = if message.guild_id.is_some() {
        if !message.mentions.iter().any(|mention| mention.id == bot) {
            return None;
        }
        Some(message.channel_id.to_string())
    } else {
        None
    };
    let bot = bot.get();
    let text = message
        .content
        .replace(&format!("<@{bot}>"), "")
        .replace(&format!("<@!{bot}>"), "");
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let timestamp = Timestamp::from_microsecond(message.timestamp.as_micros())
        .inspect_err(|e| {
            tracing::debug!(error = ?e, "discord message timestamp invalid, dropped");
        })
        .ok()?;
    Some(concept::message::Inbound::new(
        message.id.to_string(),
        group,
        message.author.id.to_string(),
        super::super::super::event::truncate_bytes(text, config::AGENT_INBOUND_TEXT_MAX).to_owned(),
        timestamp,
    ))
}
