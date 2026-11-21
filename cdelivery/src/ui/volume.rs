use std::sync::Arc;

use axum::extract::State;
use axum::response::Response;

use crate::boot::App;
use crate::operation;

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

async fn get(_authorized: super::auth::Authorized<operation::volume::Get>) -> Response {
    super::router::redirect("/nodes#volume-management")
}

async fn update<O: operation::catalog::Catalogued>(
    authorized: super::auth::Authorized<O>,
    State(app): State<Arc<App>>,
    super::command::Payload(input): super::command::Payload<O::Input>,
) -> super::response::Result<Response> {
    authorized.execute(&app, input).await?;
    Ok(super::router::redirect("/nodes#volume-management"))
}
