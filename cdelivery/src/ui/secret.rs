use std::sync::Arc;

use askama::Template;
use axum::{extract::State, response::Response};
use http::StatusCode;

use crate::boot::App;
use crate::locale::tt;
use crate::operation;

#[derive(Template)]
#[template(path = "secret/list.html")]
struct ListPage {
    dashboard: super::dashboard::Dashboard,
    secrets: Vec<operation::secret::typ::Secret>,
}

pub(super) fn routes() -> axum::Router<Arc<App>> {
    axum::Router::new()
        .route("/secrets", axum::routing::get(list).post(create))
        .route("/secrets/{name}/delete", axum::routing::post(delete))
}

async fn list(
    State(app): State<Arc<App>>,
    authorized: super::auth::Authorized<operation::secret::List>,
) -> super::response::Result<Response> {
    let secrets = authorized
        .execute(&app, operation::secret::typ::Query { kind: None })
        .await?
        .secrets;
    let dashboard = super::dashboard::Dashboard::load(
        &app,
        authorized.access(),
        super::dashboard::Location::Secrets,
    )
    .await?;
    super::response::render(&ListPage { dashboard, secrets }, StatusCode::OK)
}

async fn create(
    State(app): State<Arc<App>>,
    authorized: super::auth::Authorized<operation::secret::Create>,
    super::command::Payload(payload): super::command::Payload<
        operation::secret::typ::CreateRequest,
    >,
) -> super::response::Result<Response> {
    authorized.execute(&app, payload).await?;
    Ok(super::command::complete("/secrets"))
}

async fn delete(
    State(app): State<Arc<App>>,
    authorized: super::auth::Authorized<operation::secret::Delete>,
    axum::extract::Path(name): axum::extract::Path<String>,
    super::command::Payload(_): super::command::Payload<operation::input::Empty>,
) -> super::response::Result<Response> {
    authorized
        .execute(&app, operation::input::Name { name })
        .await?;
    Ok(super::router::redirect("/secrets"))
}
