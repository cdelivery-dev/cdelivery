use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use macros_derive::route;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::boot::App as AppState;
use crate::error::Result;
use crate::operation;

#[route(
    get,
    "/apps/{name:app name}/guardian",
    ok(operation::guardian::typ::Guardian)
)]
async fn find(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::guardian::Get>,
    super::extract::Path(name): super::extract::Path<String>,
) -> Result<Json<operation::guardian::typ::Guardian>> {
}

#[route(post, "/apps/{name:app name}/guardian/retry", no_content)]
async fn retry(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::guardian::Retry>,
    super::extract::Path(name): super::extract::Path<String>,
) -> Result<StatusCode> {
}

#[route(post, "/apps/{name:app name}/guardian/skip", no_content)]
async fn skip(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::guardian::Skip>,
    super::extract::Path(name): super::extract::Path<String>,
) -> Result<StatusCode> {
}

#[route(delete, "/apps/{name:app name}/guardian", no_content)]
async fn cancel(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::guardian::Cancel>,
    super::extract::Path(name): super::extract::Path<String>,
) -> Result<StatusCode> {
}

pub(crate) fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        .routes(routes!(find, cancel))
        .routes(routes!(retry))
        .routes(routes!(skip))
}
