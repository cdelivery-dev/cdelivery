use std::sync::Arc;

use axum::extract::State;
use axum::response::Response;
use http::StatusCode;

use crate::boot::App;
use crate::locale::tt;
use crate::{concept, operation};

#[derive(askama::Template)]
#[template(path = "volume.html")]
struct Page {
    dashboard: super::dashboard::Dashboard,
    current_version: Option<String>,
    target_version: Option<String>,
    latest_unavailable: bool,
    can_manage: bool,
}

pub(super) fn routes() -> axum::Router<Arc<App>> {
    axum::Router::new()
        .route("/volume", axum::routing::get(get))
        .route(
            "/volume/enable",
            axum::routing::post(update::<operation::volume::Enable>),
        )
        .route(
            "/volume/disable",
            axum::routing::post(update::<operation::volume::Disable>),
        )
}

async fn get(
    authorized: super::auth::Authorized<operation::volume::Get>,
    State(app): State<Arc<App>>,
) -> super::response::Result<Response> {
    let can_manage = authorized
        .access()
        .allows(concept::permission::Requirement::Admin);
    let (dashboard, current, [latest]) = tokio::join!(
        super::dashboard::Dashboard::load(
            &app,
            authorized.access(),
            super::dashboard::Location::Volume,
        ),
        authorized.execute(&app, operation::input::Empty {}),
        super::latest_versions(
            &app,
            authorized.access(),
            can_manage,
            [concept::release::Component::CdeliveryAgent],
        ),
    );
    let image = current?.image;
    let current_version = image.as_ref().map(|image| image.tag().clone());
    let latest_unavailable = latest.is_none();
    let target_version = super::target_version(current_version.as_deref(), latest)?;
    super::response::render(
        &Page {
            dashboard: dashboard?,
            current_version,
            target_version,
            latest_unavailable,
            can_manage,
        },
        StatusCode::OK,
    )
}

async fn update<O: operation::catalog::Catalogued>(
    authorized: super::auth::Authorized<O>,
    State(app): State<Arc<App>>,
    super::command::Payload(input): super::command::Payload<O::Input>,
) -> super::response::Result<Response> {
    authorized.execute(&app, input).await?;
    Ok(super::router::redirect("/volume"))
}
