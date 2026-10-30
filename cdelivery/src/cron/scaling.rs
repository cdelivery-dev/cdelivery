use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::{boot, concept, config, usecase};

pub(super) async fn run(app: Arc<boot::App>, cancel: CancellationToken) {
    let mut controller = concept::scaling::Controller::default();
    let mut tick = tokio::time::interval(config::SCALING_INTERVAL.unsigned_abs());
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = cancel.cancelled() => break,
            _ = tick.tick() => {
                let Some(result) = cancel.run_until_cancelled(usecase::scaling::run_round(
                    &app.docker, &app.key, &app.insight, &mut controller,
                )).await else {
                    break;
                };
                if let Err(error) = result {
                    tracing::warn!(error = ?error, "automatic scaling round failed");
                }
            }
        }
    }
}
