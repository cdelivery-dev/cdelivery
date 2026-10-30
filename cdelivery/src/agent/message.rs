use std::cmp::Reverse;
use std::collections::HashSet;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::boot::App;
use crate::config;
use crate::{concept, infra, usecase};

pub(super) async fn run(
    app: Arc<App>,
    llm: Arc<infra::llm::Llm>,
    adapters: Arc<infra::channel::Adapters>,
    cancel: CancellationToken,
) {
    let hub = &infra::message::MESSAGES;
    let mut handles = tokio::task::JoinSet::new();
    while !cancel.is_cancelled() {
        let available = config::AGENT_CONCURRENCY.saturating_sub(handles.len());
        for timeline in hub.claim(available) {
            handles.spawn(handle(
                Release { timeline },
                Arc::clone(&app),
                Arc::clone(&llm),
                Arc::clone(&adapters),
                cancel.clone(),
            ));
        }
        tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            joined = handles.join_next(), if !handles.is_empty() => {
                if let Some(Err(e)) = joined {
                    tracing::warn!(error = ?e, "message handle crashed");
                }
            }
            () = hub.wake().notified(), if handles.len() < config::AGENT_CONCURRENCY => {}
        }
    }
    while let Some(joined) = handles.join_next().await {
        if let Err(e) = joined {
            tracing::warn!(error = ?e, "message handle crashed during shutdown");
        }
    }
}

struct Release {
    timeline: concept::message::Timeline,
}

impl Drop for Release {
    fn drop(&mut self) {
        infra::message::MESSAGES.release(&self.timeline);
    }
}

async fn handle(
    release: Release,
    app: Arc<App>,
    llm: Arc<infra::llm::Llm>,
    adapters: Arc<infra::channel::Adapters>,
    cancel: CancellationToken,
) {
    let timeline = &release.timeline;
    let user = match usecase::user::find_by_id(&app.docker, &app.key, timeline.user()).await {
        Ok(Some(user)) => user,
        Ok(None) => {
            tracing::warn!("message user of {timeline} not found, turn skipped");
            return;
        }
        Err(e) => {
            tracing::warn!(error = ?e, "message user of {timeline} lookup failed");
            return;
        }
    };
    let access = usecase::access::user_access(&user, None, concept::actor::Origin::Agent);
    let now = jiff::Timestamp::now();
    let start = now
        .saturating_sub(config::AGENT_CHAT_WINDOW)
        .unwrap_or(jiff::Timestamp::MIN);
    let history =
        match usecase::chat::history(&app.docker, &app.insight, timeline, (start, now)).await {
            Ok(history) => history,
            Err(e) => {
                tracing::warn!(error = ?e, "message history query failed for {timeline}");
                return;
            }
        };
    let mut tools = super::tool::Session::new(&app, access);
    let text = match super::turn::run(
        &llm,
        &mut tools,
        super::prompt::MESSAGE,
        &cancel,
        |budget| context(history, timeline, start, budget),
    )
    .await
    {
        Ok(Some(text)) => text,
        Ok(None) => {
            tracing::info!("message turn cancelled for {timeline}");
            return;
        }
        Err(e) => {
            tracing::warn!(error = ?e, "message turn failed for {timeline}");
            "Reply failed. Actions may have completed. Check the current state before retrying."
                .to_owned()
        }
    };
    if !cancel.is_cancelled() {
        deliver(timeline, &adapters, user.email(), &text).await;
    }
}

fn context(
    mut history: Vec<concept::message::Message>,
    timeline: &concept::message::Timeline,
    start: jiff::Timestamp,
    mut budget: usize,
) -> Vec<genai::chat::ChatMessage> {
    history.extend(
        infra::message::MESSAGES
            .take_recent(timeline)
            .into_iter()
            .filter(|message| message.timestamp() >= start),
    );
    let mut seen = HashSet::new();
    history.retain(|message| seen.insert(message.id()));
    history.sort_by_key(|message| Reverse((message.timestamp(), message.id())));
    history.truncate(usize::from(config::AGENT_MESSAGE_RING));
    history.reverse();
    let mut picked = Vec::new();
    for message in history.iter().rev() {
        let line = match message.role() {
            insight_wire::message::Role::User => {
                format!("[{}] {}", message.author(), message.text())
            }
            insight_wire::message::Role::Agent => message.text().clone(),
        };
        if line.len() > budget {
            break;
        }
        budget -= line.len();
        picked.push(match message.role() {
            insight_wire::message::Role::User => genai::chat::ChatMessage::user(line),
            insight_wire::message::Role::Agent => genai::chat::ChatMessage::assistant(line),
        });
    }
    picked.reverse();
    picked
}

async fn deliver(
    timeline: &concept::message::Timeline,
    out: &infra::channel::Adapters,
    email_to: &str,
    text: &str,
) {
    match timeline {
        concept::message::Timeline::Dm { im, id, .. }
        | concept::message::Timeline::Group {
            im, chat_id: id, ..
        } => match out.im.get(im) {
            Some(client) => {
                let sent = match timeline {
                    concept::message::Timeline::Group { .. } => client.send_group(id, text).await,
                    _ => client.send_dm(id, text).await,
                };
                super::relay(
                    sent,
                    timeline.clone(),
                    "agent",
                    text,
                    &format!("agent reply to {} {id}", im.key()),
                );
            }
            None => {
                tracing::warn!("{} channel not configured, reply to {id} dropped", im.key());
            }
        },
        concept::message::Timeline::Email { .. } => {
            let Some(channel) = out.email.as_deref() else {
                tracing::warn!("email channel not configured, reply to {email_to} dropped");
                return;
            };
            super::relay(
                channel.send(email_to, "[CDelivery]", text).await,
                timeline.clone(),
                "agent",
                text,
                &format!("agent reply to email {email_to}"),
            );
        }
    }
}
