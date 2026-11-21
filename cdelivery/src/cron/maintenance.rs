use std::sync::Arc;

use jiff::Timestamp;

use crate::boot::App;
use crate::infra;

pub(super) async fn round(app: Arc<App>) {
    let (releases, sizes) = tokio::join!(
        app.releases.refresh(&app.docker),
        infra::dependency::sizes::refresh(),
    );
    if let Err(error) = releases {
        tracing::warn!(error = ?error, "release catalog refresh failed");
    }
    if let Err(error) = sizes {
        tracing::warn!(error = ?error, "PlanetScale size catalog refresh failed");
    }
    let now = Timestamp::now();
    match infra::group::sweep(&app.docker).await {
        Ok(n) if n > 0 => tracing::info!("collected {n} orphan group records"),
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(error = ?error, "group sweep failed");
        }
    }
    match infra::app::xconfig::sweep(&app.docker, &app.key, now).await {
        Ok(n) if n > 0 => tracing::info!("collected {n} orphan xconfigs"),
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(error = ?error, "xconfig sweep failed");
        }
    }
}
