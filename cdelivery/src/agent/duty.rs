use std::fmt::Write as _;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::boot::App;
use crate::error::Result;
use crate::{concept, config, infra, usecase};

pub(crate) async fn alerts(app: Arc<App>, cancel: CancellationToken) {
    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => {
                discard(&infra::event::DUTY_RAW, "raw");
                return;
            }
            () = infra::event::DUTY_RAW.wake().notified() => {}
        }
        let (events, dropped) = infra::event::DUTY_RAW.drain();
        if dropped != 0 {
            tracing::warn!("platform alert queue dropped {dropped} events before drain");
        }
        if events.is_empty() {
            continue;
        }
        if let Err(error) = raw_batch(&app, &events, &cancel).await {
            tracing::warn!(error = ?error, "platform alert delivery failed");
        }
    }
}

pub(super) async fn run(app: Arc<App>, cancel: CancellationToken) {
    discard(&infra::event::DUTY_ANALYSIS, "analysis");
    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            () = async {
                infra::event::DUTY_ANALYSIS.wake().notified().await;
                tokio::time::sleep(config::AGENT_ANALYSIS_DEBOUNCE.unsigned_abs()).await;
            } => {}
        }
        while !cancel.is_cancelled() {
            let (events, dropped) = infra::event::DUTY_ANALYSIS.drain();
            if dropped != 0 {
                tracing::warn!("duty analysis queue dropped {dropped} events before drain");
            }
            if events.is_empty() {
                break;
            }
            if let Err(error) = analyze_batch(&app, &events, &cancel).await {
                tracing::warn!(error = ?error, "duty analysis failed");
            }
        }
    }
    discard(&infra::event::DUTY_ANALYSIS, "analysis");
}

fn discard(queue: &infra::event::DutyQueue, stage: &str) {
    let (queued, dropped) = queue.drain();
    if !queued.is_empty() || dropped != 0 {
        tracing::warn!(
            "duty {stage} discarded {} queued events and {dropped} overflowed events",
            queued.len()
        );
    }
}

async fn raw_batch(
    app: &App,
    events: &[insight_wire::event::Event],
    cancel: &CancellationToken,
) -> Result<()> {
    let settings = infra::setting::load(&app.docker, &app.key)
        .await?
        .into_inner();
    let adapters = infra::channel::Adapters::build(&settings)?;
    let users = usecase::user::list(&app.docker, &app.key).await?;
    let text = format!("Platform alert\n{}", lines(events));
    let text = infra::event::truncate_bytes(&text, config::AGENT_TEXT_MAX);
    if !dispatch(&users, &adapters, text, concept::user::Role::Reader, cancel).await
        && !cancel.is_cancelled()
    {
        tracing::warn!("platform alert skipped, no eligible recipient");
    }
    Ok(())
}

async fn analyze_batch(
    app: &App,
    events: &[insight_wire::event::Event],
    cancel: &CancellationToken,
) -> Result<()> {
    let settings = infra::setting::load(&app.docker, &app.key)
        .await?
        .into_inner();
    let Some(agent) = settings.duty_agent() else {
        tracing::info!("duty off, discarded {} analysis events", events.len());
        return Ok(());
    };
    let adapters = infra::channel::Adapters::build(&settings)?;
    let users = usecase::user::list(&app.docker, &app.key).await?;
    let language = agent.language();
    let role = agent.role();
    if !deliverable(&users, &adapters, role) {
        tracing::warn!("duty analysis skipped, no eligible recipient");
        return Ok(());
    }
    let llm = infra::llm::Llm::new(agent, config::AGENT_LLM_TIMEOUT)?;
    let access = concept::permission::Access::new(concept::actor::Actor::duty(), role);
    let mut tools = super::tool::Session::new(app, access);
    let system = format!(
        "{} Response policy: {:?}. Notification language: {}.",
        super::prompt::DUTY,
        agent.duty_policy(),
        language.tag()
    );
    let result = super::turn::run(&llm, &mut tools, &system, cancel, |budget| {
        vec![genai::chat::ChatMessage::user(compose(
            &infra::event::HUB.observe(),
            events,
            budget,
        ))]
    })
    .await;
    let reply = match result {
        Ok(Some(text)) => Some(text),
        Ok(None) => {
            tracing::info!("duty analysis cancelled for {} events", events.len());
            return Ok(());
        }
        Err(error) => {
            if tools.receipts().is_empty() {
                return Err(error);
            }
            tracing::warn!(error = ?error, "duty analysis failed after write attempts");
            None
        }
    };
    let reply = reply.filter(|text| text.trim() != super::prompt::SILENT);
    let notice = analysis_notice(reply.as_deref(), tools.receipts());
    if notice.is_empty() {
        tracing::info!("duty round silent for {} events", events.len());
        return Ok(());
    }
    let settings = infra::setting::load(&app.docker, &app.key)
        .await?
        .into_inner();
    if settings.duty_agent().is_none() {
        tracing::info!(
            "duty paused during analysis, dropped notice for {} events",
            events.len()
        );
        return Ok(());
    }
    let adapters = infra::channel::Adapters::build(&settings)?;
    let users = usecase::user::list(&app.docker, &app.key).await?;
    if cancel.is_cancelled() {
        return Ok(());
    }
    if !dispatch(&users, &adapters, &notice, role, cancel).await && !cancel.is_cancelled() {
        tracing::warn!("duty notice dropped, no current eligible recipient");
    }
    Ok(())
}

fn deliverable(
    users: &[concept::user::User],
    adapters: &infra::channel::Adapters,
    role: concept::user::Role,
) -> bool {
    users.iter().filter(|user| user.role() >= role).any(|user| {
        let preferences = user.preferences();
        preferences.alert_email() && adapters.email.is_some()
            || user
                .im()
                .keys()
                .any(|im| preferences.alerts().contains(im) && adapters.im.contains_key(im))
    })
}

async fn dispatch(
    users: &[concept::user::User],
    adapters: &infra::channel::Adapters,
    text: &str,
    role: concept::user::Role,
    cancel: &CancellationToken,
) -> bool {
    let mut attempted = false;
    for user in users.iter().filter(|user| user.role() >= role) {
        let preferences = user.preferences();
        for (im, id) in user.im() {
            if cancel.is_cancelled() {
                return attempted;
            }
            if !preferences.alerts().contains(im) {
                continue;
            }
            let Some(client) = adapters.im.get(im) else {
                tracing::warn!(
                    "{} alert to user {id} dropped, channel not configured",
                    im.key()
                );
                continue;
            };
            attempted = true;
            super::relay(
                client.send_dm(id, text).await,
                concept::message::Timeline::Dm {
                    im: *im,
                    id: id.clone(),
                    user: user.id(),
                },
                config::DUTY_OPERATOR,
                text,
                &format!("{} alert to user {id}", im.key()),
            );
        }
        if preferences.alert_email()
            && let Some(channel) = adapters.email.as_deref()
        {
            if cancel.is_cancelled() {
                return attempted;
            }
            attempted = true;
            super::relay(
                channel.send(user.email(), "[CDelivery] Notice", text).await,
                concept::message::Timeline::Email { user: user.id() },
                config::DUTY_OPERATOR,
                text,
                &format!("email alert to {}", user.email()),
            );
        }
    }
    attempted
}

fn analysis_notice(reply: Option<&str>, receipts: &[String]) -> String {
    let mut text = receipts.join("\n");
    if let Some(reply) = reply {
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(reply);
    }
    infra::event::truncate_bytes(&text, config::AGENT_TEXT_MAX).to_owned()
}

fn lines(events: &[insight_wire::event::Event]) -> String {
    let mut body = String::new();
    for event in events.iter().rev() {
        let _ = writeln!(
            body,
            "{} {:?} {} by {}: {}",
            event.timestamp,
            event.kind,
            event.subject,
            event.actor,
            event.detail.as_deref().unwrap_or("")
        );
    }
    body
}

fn compose(
    conditions: &[insight_wire::event::Condition],
    events: &[insight_wire::event::Event],
    budget: usize,
) -> String {
    let mut text = "New events:\n".to_owned();
    text.push_str(&lines(events));
    text.push_str("Known diagnostics (not proof of current availability or recovery):\n");
    if conditions.is_empty() {
        text.push_str("(none known)\n");
    }
    for condition in conditions {
        let _ = writeln!(
            text,
            "{:?} {} first observed {}, last observed {}: {}",
            condition.kind,
            condition.subject,
            condition.since,
            condition.observed_at,
            condition.detail
        );
    }
    infra::event::truncate_bytes(&text, budget).to_owned()
}
