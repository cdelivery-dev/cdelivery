use std::sync::Arc;

use insight_wire::event::{Event, Kind};
use jiff::{SignedDuration, Timestamp};
use tokio_util::sync::CancellationToken;

use crate::boot::App;
use crate::config;
use crate::error::Error;
use crate::{concept, infra, usecase};

pub(crate) async fn run(app: Arc<App>, cancel: CancellationToken) {
    loop {
        let settings = match infra::setting::load(&app.docker, &app.key).await {
            Ok(settings) => settings,
            Err(e) => {
                tracing::warn!(error = ?e, "agent settings load failed");
                if pause(&app, &cancel).await {
                    return;
                }
                continue;
            }
        };
        let adapters = match infra::channel::Adapters::build(settings.get()) {
            Ok(adapters) => Arc::new(adapters),
            Err(e) => {
                tracing::warn!(error = ?e, "agent adapters build failed");
                if pause(&app, &cancel).await {
                    return;
                }
                continue;
            }
        };
        let agent = settings.get().agent();
        let llm = match agent
            .map(|cfg| infra::llm::Llm::new(cfg, config::AGENT_LLM_TIMEOUT))
            .transpose()
        {
            Ok(llm) => llm.map(Arc::new),
            Err(e) => {
                tracing::warn!(error = ?e, "agent build failed");
                None
            }
        };
        let sub = cancel.child_token();
        let mut set = tokio::task::JoinSet::new();
        if let (Some(llm), Some(agent)) = (&llm, agent) {
            set.spawn(super::message::run(
                Arc::clone(&app),
                Arc::clone(llm),
                Arc::clone(&adapters),
                sub.clone(),
            ));
            if !settings.get().duty_paused() {
                set.spawn(super::duty::run(Arc::clone(&app), sub.clone()));
                set.spawn(patrol(
                    Arc::clone(&app),
                    agent.patrol_interval(),
                    sub.clone(),
                ));
            }
        }
        for (im, client) in &adapters.im {
            let im = *im;
            let handler_app = Arc::clone(&app);
            let handler_client = client.clone();
            let agent_enabled = llm.is_some();
            let on_inbound = move |inbound| {
                im_inbound(
                    Arc::clone(&handler_app),
                    im,
                    handler_client.clone(),
                    agent_enabled,
                    inbound,
                )
            };
            set.spawn(infra::channel::run(client.clone(), sub.clone(), on_inbound));
        }
        if llm.is_some()
            && settings
                .get()
                .email()
                .is_some_and(|email| email.imap().is_some())
            && let Some(channel) = adapters.email.clone()
        {
            set.spawn(poll_email(Arc::clone(&app), channel, sub.clone()));
        }
        tokio::select! {
            () = cancel.cancelled() => {
                teardown(&sub, &mut set).await;
                return;
            }
            () = app.agent_reload.notified() => teardown(&sub, &mut set).await,
            joined = set.join_next(), if !set.is_empty() => {
                tracing::warn!(error = ?joined, "agent task exited during run");
                teardown(&sub, &mut set).await;
                if pause(&app, &cancel).await {
                    return;
                }
            }
        }
    }
}

async fn im_inbound(
    app: Arc<App>,
    im: concept::message::Im,
    client: infra::channel::Client,
    agent_enabled: bool,
    inbound: concept::message::Inbound,
) {
    let user = 'lookup: {
        for attempt in 0..3 {
            match usecase::user::find_by_im(&app.docker, &app.key, im, inbound.sender_id()).await {
                Ok(Some(user)) => break 'lookup Some(user),
                Ok(None) => break 'lookup None,
                Err(e) if attempt < 2 => {
                    tracing::warn!(error = ?e, "{} sender lookup failed, retrying", im.key());
                    tokio::time::sleep(config::AGENT_INBOUND_RETRY.unsigned_abs()).await;
                }
                Err(e) => {
                    tracing::warn!(error = ?e, "{} sender lookup failed, message dropped", im.key());
                    return;
                }
            }
        }
        None
    };
    let Some(user) = user else {
        match inbound.group_chat_id() {
            None => {
                let text = format!("Add this ID in Account settings:\n{}", inbound.sender_id());
                if let Err(e) = client.send_dm(inbound.sender_id(), &text).await {
                    tracing::warn!(error = ?e, "{} unbound reply failed", im.key());
                } else {
                    tracing::info!("{} unbound reply sent", im.key());
                }
            }
            Some(chat_id) => {
                tracing::debug!(
                    "{} group {chat_id} message from unbound sender, dropped",
                    im.key()
                );
            }
        }
        return;
    };
    let source = source_id(im, inbound.event_id());
    let timeline = match inbound.group_chat_id() {
        None => concept::message::Timeline::Dm {
            im,
            id: inbound.sender_id().clone(),
            user: user.id(),
        },
        Some(chat_id) => concept::message::Timeline::Group {
            im,
            chat_id: chat_id.clone(),
            user: user.id(),
        },
    };
    if !agent_enabled {
        if let concept::message::Timeline::Dm { id, .. } = &timeline
            && let Err(e) = client.send_dm(id, "AI Agent is disabled.").await
        {
            tracing::warn!(error = ?e, "{} disabled-agent reply failed", im.key());
        }
        return;
    }
    infra::message::MESSAGES.emit(&concept::message::Message::with_id(
        source,
        timeline,
        insight_wire::message::Role::User,
        user.name(),
        inbound.text().clone(),
        inbound.timestamp(),
    ));
}

fn source_id(im: concept::message::Im, event_id: &str) -> uuid::Uuid {
    let source = format!("{}:{event_id}", im.key());
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, source.as_bytes())
}

async fn pause(app: &App, cancel: &CancellationToken) -> bool {
    tokio::select! {
        () = cancel.cancelled() => true,
        () = app.agent_reload.notified() => false,
        () = tokio::time::sleep(config::AGENT_RETRY.unsigned_abs()) => false,
    }
}

async fn teardown(sub: &CancellationToken, set: &mut tokio::task::JoinSet<()>) {
    sub.cancel();
    while set.join_next().await.is_some() {}
}

async fn patrol_event(app: &App) -> Event {
    let timestamp = Timestamp::now();
    let detail = match usecase::overview::summary(&app.docker, &app.key).await {
        Ok(summary) => format!("Scheduled overview: {summary:?}"),
        Err(error) => {
            tracing::warn!(error = ?error, "agent patrol overview unavailable");
            "Scheduled overview unavailable.".to_owned()
        }
    };
    Event {
        timestamp,
        kind: Kind::Patrol,
        subject: "platform".to_owned(),
        actor: config::DUTY_OPERATOR.to_owned(),
        detail: Some(infra::event::truncate_bytes(&detail, config::EVENT_DETAIL_MAX).to_owned()),
    }
}

async fn patrol(app: Arc<App>, interval: SignedDuration, cancel: CancellationToken) {
    let rounds = async {
        loop {
            tokio::time::sleep(interval.unsigned_abs()).await;
            let event = patrol_event(&app).await;
            infra::event::DUTY_ANALYSIS.push([event]);
        }
    };
    let _ = cancel.run_until_cancelled(rounds).await;
}

async fn poll_email(
    app: Arc<App>,
    channel: Arc<infra::channel::email::Channel>,
    cancel: CancellationToken,
) {
    let mut timer = tokio::time::interval(config::AGENT_IMAP_POLL.unsigned_abs());
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut cursor = None;
    tokio::select! {
        biased;
        () = cancel.cancelled() => {}
        () = async {
            loop {
                timer.tick().await;
                let poll = channel.poll(&app.docker, &app.key, &mut cursor);
                match tokio::time::timeout(config::AGENT_IMAP_DEADLINE.unsigned_abs(), poll).await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => tracing::warn!(error = ?e, "email poll failed"),
                    Err(source) => {
                        let error = Error::from(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            source,
                        ));
                        tracing::warn!(error = ?error, "email poll timed out");
                    }
                }
            }
        } => {}
    }
}
