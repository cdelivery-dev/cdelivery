use std::fmt::Write as _;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::boot::App;
use crate::error::Result;
use crate::{concept, config, infra, locale, usecase};

pub(crate) async fn run(app: Arc<App>, cancel: CancellationToken) {
    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            () = infra::event::DUTY.wake().notified() => {}
        }
        let (events, dropped) = infra::event::DUTY.drain();
        if dropped != 0 {
            tracing::warn!("duty queue dropped {dropped} events before drain");
        }
        if events.is_empty() {
            continue;
        }
        if let Some(Err(error)) = cancel
            .run_until_cancelled(Box::pin(batch(&app, events, &cancel)))
            .await
        {
            tracing::warn!(error = ?error, "duty notification failed");
        }
    }
    let (events, dropped) = infra::event::DUTY.drain();
    if !events.is_empty() || dropped != 0 {
        tracing::warn!(
            "duty stopped with {} queued events and {dropped} overflowed events",
            events.len()
        );
    }
}

async fn batch(
    app: &App,
    mut events: Vec<insight_wire::event::Event>,
    cancel: &CancellationToken,
) -> Result<()> {
    let generation = app
        .duty_run
        .lock()
        .clone()
        .filter(|(_, token)| !token.is_cancelled());
    events.retain(|event| {
        concept::event::attention(event.kind)
            || generation
                .as_ref()
                .is_some_and(|(since, _)| event.timestamp >= *since)
    });
    let Some(first) = events.first() else {
        return Ok(());
    };
    let batch = first.timestamp;
    for event in &events {
        let identity = (event.timestamp, event.kind, event.subject.as_str());
        tracing::info!("duty batch {batch}: {identity:?}");
    }
    let alerts: Vec<_> = events
        .iter()
        .filter(|event| concept::event::attention(event.kind))
        .collect();
    let raw = lines(&alerts);
    let deadline = config::expiry(
        alerts
            .iter()
            .map(|event| event.timestamp)
            .min()
            .unwrap_or_else(jiff::Timestamp::now),
        config::DUTY_ANALYSIS_TIMEOUT,
    );
    let settings = infra::setting::load(&app.docker, &app.key)
        .await?
        .into_inner();
    let mut adapters = infra::channel::Adapters::build(&settings)?;
    let mut users = usecase::user::list(&app.docker, &app.key).await?;
    let mut analysis = None;
    if let (Some(agent), Some((since, token))) = (settings.duty_agent(), &generation)
        && alerts.iter().all(|event| event.timestamp >= *since)
        && !token.is_cancelled()
        && deadline > jiff::Timestamp::now()
        && deliverable(&users, &adapters, agent.role())
    {
        let mut reply = None;
        let access = concept::permission::Access::new(concept::actor::Actor::duty(), agent.role());
        let mut tools = super::tool::Session::new(app, access);
        let remaining = deadline
            .duration_since(jiff::Timestamp::now())
            .max(jiff::SignedDuration::ZERO);
        let result = token
            .run_until_cancelled(tokio::time::timeout(
                remaining.unsigned_abs(),
                analyze(agent, &events, &mut tools, token, !alerts.is_empty()),
            ))
            .await;
        match result {
            Some(Ok(Ok(text))) => {
                reply = text.filter(|text| text.trim() != super::prompt::SILENT);
            }
            Some(Ok(Err(error))) => {
                tracing::warn!(error = ?error, "duty analysis failed, using original alerts");
            }
            Some(Err(error)) => {
                tracing::warn!(error = ?error, "duty analysis timed out, using original alerts");
            }
            None => tracing::info!("duty analysis cancelled, using original alerts"),
        }
        let current = infra::setting::load(&app.docker, &app.key)
            .await?
            .into_inner();
        if token.is_cancelled() || current.duty_agent() != Some(agent) {
            reply = None;
        }
        if reply.is_some() || !tools.receipts().is_empty() {
            let body = reply.as_deref().unwrap_or(&raw);
            let mut notice = tools.receipts().join("\n");
            if !notice.is_empty() && !body.is_empty() {
                notice.push_str("\n\n");
            }
            notice.push_str(body);
            analysis = Some((
                agent.role(),
                infra::event::truncate_bytes(&notice, config::AGENT_TEXT_MAX).to_owned(),
            ));
        }
        adapters = infra::channel::Adapters::build(&current)?;
        users = usecase::user::list(&app.docker, &app.key).await?;
    }
    if !dispatch(&users, &adapters, &raw, analysis.as_ref(), batch, cancel).await
        && (!raw.is_empty() || analysis.is_some())
        && !cancel.is_cancelled()
    {
        tracing::warn!("duty notification skipped, no eligible recipient");
    }
    Ok(())
}

async fn analyze(
    agent: &concept::setting::Agent,
    events: &[insight_wire::event::Event],
    tools: &mut super::tool::Session<'_>,
    cancel: &CancellationToken,
    alerts: bool,
) -> Result<Option<String>> {
    let llm = infra::llm::Llm::new(agent, config::AGENT_LLM_TIMEOUT)?;
    let system = format!(
        "{} Response policy: {:?}. Notification language: {}. Alert batch: {alerts}.",
        super::prompt::DUTY,
        agent.duty_policy(),
        agent.language().tag()
    );
    super::turn::run(&llm, tools, &system, cancel, alerts, |budget| {
        vec![genai::chat::ChatMessage::user(compose(
            infra::event::HUB.observe(),
            events,
            budget,
        ))]
    })
    .await
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
    raw: &str,
    analysis: Option<&(concept::user::Role, String)>,
    batch: jiff::Timestamp,
    cancel: &CancellationToken,
) -> bool {
    let mut attempted = false;
    for user in users {
        let (variant, text) = analysis
            .filter(|(role, _)| user.role() >= *role)
            .map_or(("original", raw), |(_, text)| ("analysis", text.as_str()));
        if text.is_empty() {
            continue;
        }
        let notice = format!("duty {variant} batch {batch} to user {}", user.id());
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
                &format!("{} {notice}", im.key()),
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
                &format!("email {notice}"),
            );
        }
    }
    attempted
}

fn lines(events: &[&insight_wire::event::Event]) -> String {
    let mut body = String::new();
    for event in events {
        let key = format!("event.{:?}", event.kind);
        let title = locale::tt!(locale = locale::Locale::En, &key);
        let _ = writeln!(body, "{} — {title}", event.subject);
        for detail in event
            .detail
            .as_deref()
            .unwrap_or("")
            .lines()
            .filter(|line| !line.is_empty())
            .take(2)
        {
            let _ = writeln!(body, "{detail}");
        }
        let _ = writeln!(
            body,
            "Observed: {}\n",
            event.timestamp.strftime("%Y-%m-%d %H:%M:%S UTC")
        );
    }
    infra::event::truncate_bytes(&body, config::AGENT_TEXT_MAX).to_owned()
}

fn compose(
    mut conditions: Vec<insight_wire::event::Condition>,
    events: &[insight_wire::event::Event],
    budget: usize,
) -> String {
    let patrol = events
        .iter()
        .any(|event| event.kind == insight_wire::event::Kind::Patrol);
    conditions.retain(|condition| {
        patrol
            || events
                .iter()
                .any(|event| event.subject == condition.subject)
    });
    let text = format!(
        "New events:\n{}\nRecorded diagnostics (check observation times):\n{}",
        serde_json::json!(events),
        serde_json::json!(conditions)
    );
    infra::event::truncate_bytes(&text, budget).to_owned()
}
