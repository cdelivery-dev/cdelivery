use std::sync::Arc;

use jiff::Timestamp;

use crate::boot::App;
use crate::infra;
use crate::usecase;

pub(super) async fn round(app: Arc<App>) {
    let now = Timestamp::now();
    let (triggered, mut diagnostics) = usecase::xhealth::sweep(&app.docker, &app.key, now).await;
    if triggered > 0 {
        tracing::info!("started {triggered} health check rounds");
    }
    let (jobs, job_diagnostics) = usecase::xjob::sweep(&app.docker, &app.key, now).await;
    diagnostics.extend(job_diagnostics);
    if jobs > 0 {
        tracing::info!("started {jobs} xjob runs");
    }
    diagnostics.extend(usecase::guardian::sweep(&app.docker, &app.key, now).await);
    diagnostics.extend(infra::event::sweep(&app.docker, now).await);
    infra::event::HUB.replace(insight_wire::event::Kind::ReconcileError, diagnostics);
}
