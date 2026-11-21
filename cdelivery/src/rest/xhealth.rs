use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use macros_derive::route;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::boot::App as AppState;
use crate::error::Result;
use crate::operation;

#[route(
    put,
    "/apps/{name:app name}/xhealth",
    created(operation::xhealth::typ::XHealth)
)]
async fn create(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::xhealth::Create>,
    super::extract::Path(name): super::extract::Path<String>,
    super::extract::Body(dto): super::extract::Body<operation::xhealth::typ::CreateXHealthRequest>,
) -> Result<(StatusCode, Json<operation::xhealth::typ::XHealth>)> {
}

#[route(
    patch,
    "/apps/{name:app name}/xhealth",
    ok(operation::xhealth::typ::XHealth)
)]
async fn update(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::xhealth::Update>,
    super::extract::Path(name): super::extract::Path<String>,
    super::extract::Body(dto): super::extract::Body<operation::xhealth::typ::UpdateXHealthRequest>,
) -> Result<Json<operation::xhealth::typ::XHealth>> {
}

#[route(
    get,
    "/apps/{name:app name}/xhealth",
    ok(operation::xhealth::typ::XHealth)
)]
async fn find(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::xhealth::Get>,
    super::extract::Path(name): super::extract::Path<String>,
) -> Result<Json<operation::xhealth::typ::XHealth>> {
}

#[route(post, "/apps/{name:app name}/xhealth/run", accepted(operation::Done))]
async fn run(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::xhealth::Run>,
    super::extract::Path(name): super::extract::Path<String>,
) -> Result<(StatusCode, Json<operation::Done>)> {
}

#[route(delete, "/apps/{name:app name}/xhealth", no_content)]
async fn disable(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::xhealth::Disable>,
    super::extract::Path(name): super::extract::Path<String>,
) -> Result<StatusCode> {
}

#[route(get, "/apps/{name:app name}/xhealth/file", ok(String, "text/plain"))]
async fn download_file(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::xhealth::DownloadFile>,
    super::extract::Path(name): super::extract::Path<String>,
) -> Result<impl IntoResponse> {
}

pub(crate) fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        .routes(routes!(create, update, find, disable))
        .routes(routes!(run))
        .routes(routes!(download_file))
}
