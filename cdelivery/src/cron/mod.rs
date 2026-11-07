use std::sync::Arc;

use jiff::SignedDuration;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::boot::App;
use crate::config;

pub(crate) mod acme;
mod maintenance;
mod reconcile;
mod scaling;

pub(crate) fn start(tracker: &TaskTracker, cancel: &CancellationToken, app: &Arc<App>) {
    tracker.spawn(scaling::run(Arc::clone(app), cancel.child_token()));
    tracker.spawn(acme::run(Arc::clone(app), cancel.child_token()));
    tracker.spawn(every(
        config::MAINTENANCE_INTERVAL,
        cancel.child_token(),
        Arc::clone(app),
        maintenance::round,
    ));
    tracker.spawn(every(
        config::RECONCILE_INTERVAL,
        cancel.child_token(),
        Arc::clone(app),
        reconcile::round,
    ));
}

async fn every<F: Future<Output = ()> + Send>(
    interval: SignedDuration,
    cancel: CancellationToken,
    app: Arc<App>,
    round: impl Fn(Arc<App>) -> F + Send,
) {
    let mut tick = tokio::time::interval(interval.unsigned_abs());
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = cancel.cancelled() => break,
            _ = tick.tick() => {
                if cancel
                    .run_until_cancelled(Box::pin(round(Arc::clone(&app))))
                    .await
                    .is_none()
                {
                    break;
                }
            }
        }
    }
}
