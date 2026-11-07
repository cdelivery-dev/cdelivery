use std::sync::Arc;

use axum::extract::State;
use axum::response::Response;
use http::StatusCode;

use crate::boot::App;
use crate::locale::tt;
use crate::{concept, operation};

#[derive(askama::Template)]
#[template(path = "settings/version.html")]
struct Page {
    account: super::super::account::Account,
    current: operation::instance::typ::Instance,
    pingress_enabled: bool,
    target_version: Option<String>,
    latest_unavailable: bool,
}

pub(super) fn routes() -> axum::Router<Arc<App>> {
    axum::Router::new().route("/settings/cdelivery", axum::routing::get(get).post(update))
}

async fn get(
    authorized: super::super::auth::Authorized<operation::instance::Update>,
    State(app): State<Arc<App>>,
) -> super::super::response::Result<Response> {
    let (current, pingress, [latest]) = tokio::join!(
        super::super::read::<operation::instance::Get>(&app, authorized.access()),
        super::super::pingress::load_pingress(&app, authorized.access()),
        super::super::latest_versions(
            &app,
            authorized.access(),
            true,
            [concept::release::Component::Cdelivery],
        ),
    );
    let current = current?;
    let latest_unavailable = latest.is_none();
    let target_version = super::super::target_version(Some(&current.version), latest)?;
    super::super::response::render(
        &Page {
            account: super::super::account::Account::new(authorized.access()),
            current,
            pingress_enabled: pingress?.is_some(),
            target_version,
            latest_unavailable,
        },
        StatusCode::OK,
    )
}

async fn update(
    authorized: super::super::auth::Authorized<operation::instance::Update>,
    State(app): State<Arc<App>>,
    super::super::command::Payload(input): super::super::command::Payload<
        operation::instance::typ::UpdateInstanceRequest,
    >,
) -> super::super::response::Result<Response> {
    authorized.execute(&app, input).await?;
    Ok(super::super::router::redirect("/settings/cdelivery"))
}
