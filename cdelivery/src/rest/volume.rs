use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use macros_derive::route;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::boot::App;
use crate::error::Result;
use crate::operation;

#[route(get, "/volume", ok(operation::volume::typ::Settings))]
async fn get(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::volume::Get>,
) -> Result<Json<operation::volume::typ::Settings>> {
}

#[route(put, "/volume", ok(operation::Done))]
async fn enable(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::volume::Enable>,
    super::extract::Body(input): super::extract::Body<operation::volume::typ::Changes>,
) -> Result<Json<operation::Done>> {
}

#[route(delete, "/volume", ok(operation::Done))]
async fn disable(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::volume::Disable>,
) -> Result<Json<operation::Done>> {
}

#[route(get, "/nodes/{id:node id}/volumes", ok(operation::volume::typ::Report))]
async fn list(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::volume::List>,
    super::extract::Path(id): super::extract::Path<String>,
    super::extract::Query(query): super::extract::Query<operation::volume::typ::Query>,
) -> Result<Json<operation::volume::typ::Report>> {
}

#[route(post, "/nodes/{id:node id}/volumes", created(operation::Done))]
async fn create(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::volume::Create>,
    super::extract::Path(id): super::extract::Path<String>,
    super::extract::Body(request): super::extract::Body<
        operation::volume::typ::CreateVolumeRequest,
    >,
) -> Result<(StatusCode, Json<operation::Done>)> {
}

#[route(
    patch,
    "/nodes/{id:node id}/volumes/{name:volume name}",
    ok(operation::Done)
)]
async fn update(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::volume::Update>,
    super::extract::Path((id, name)): super::extract::Path<(String, String)>,
    super::extract::Body(changes): super::extract::Body<
        operation::volume::typ::UpdateVolumeRequest,
    >,
) -> Result<Json<operation::Done>> {
}

#[route(
    delete,
    "/nodes/{id:node id}/volumes/{name:volume name}",
    ok(operation::Done)
)]
async fn delete(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::volume::Delete>,
    super::extract::Path((id, name)): super::extract::Path<(String, String)>,
) -> Result<Json<operation::Done>> {
}

pub(crate) fn routes() -> OpenApiRouter<Arc<App>> {
    OpenApiRouter::new()
        .routes(routes!(get, enable, disable))
        .routes(routes!(list, create))
        .routes(routes!(update, delete))
}
