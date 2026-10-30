use std::sync::Arc;

use jiff::Timestamp;
use tokio_util::sync::CancellationToken;

use crate::boot::App;
use crate::config;
use crate::infra;
use crate::usecase;

pub(crate) async fn run(app: Arc<App>, cancel: CancellationToken) {
    let mut issue = tokio::time::interval(config::ACME_ISSUE_INTERVAL.unsigned_abs());
    let mut renew = tokio::time::interval(config::ACME_RENEW_INTERVAL.unsigned_abs());
    issue.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    renew.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = cancel.cancelled() => break,
            _ = issue.tick() => {
                let mut errors = Vec::new();
                let round = usecase::acme::issue_missing(&app.docker, &app.key, &app.challenges);
                match cancel.run_until_cancelled(round).await {
                    None => break,
                    Some(Ok(issued)) if issued > 0 => tracing::info!("issued {issued} ACME certificates"),
                    Some(Ok(_)) => {}
                    Some(Err(error)) => errors.push(failed("issue", &error)),
                }
                infra::event::HUB.replace(insight_wire::event::Kind::AcmeIssueError, errors);
            }
            _ = renew.tick() => {
                let now = Timestamp::now();
                let mut errors = Vec::new();
                let round = usecase::acme::renew_due(&app.docker, &app.key, &app.challenges, now, &mut errors);
                match cancel.run_until_cancelled(round).await {
                    None => break,
                    Some(Ok(renewed)) if renewed > 0 => tracing::info!("renewed {renewed} ACME certificates"),
                    Some(Ok(_)) => {}
                    Some(Err(error)) => errors.push(failed("renew", &error)),
                }
                infra::event::HUB.replace(insight_wire::event::Kind::AcmeRenewError, errors);
            }
        }
    }
}

fn failed(action: &str, error: &crate::error::Error) -> (String, String) {
    tracing::warn!(error = ?error, "ACME {action} tick failed; retrying next tick");
    (
        format!("acme/{action}"),
        error.chain(crate::locale::Locale::En),
    )
}
