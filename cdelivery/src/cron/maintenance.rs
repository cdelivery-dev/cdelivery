use std::sync::Arc;

use jiff::Timestamp;

use crate::boot::App;
use crate::infra;

pub(super) async fn round(app: Arc<App>) {
    if let Err(error) = app.releases.refresh(&app.docker).await {
        tracing::warn!(error = ?error, "release catalog refresh failed");
    }
    let now = Timestamp::now();
    match infra::group::sweep(&app.docker).await {
        Ok(n) if n > 0 => tracing::info!("collected {n} orphan group records"),
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(error = ?error, "group sweep failed");
        }
    }
    match infra::app::xconfig::sweep(&app.docker, now).await {
        Ok(n) if n > 0 => tracing::info!("collected {n} orphan xconfigs"),
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(error = ?error, "xconfig sweep failed");
        }
    }
}
