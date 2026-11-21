use std::sync::Arc;

use axum::{Json, extract::State};
use http::StatusCode;
use macros_derive::route;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::boot::App;
use crate::error::Result;
use crate::operation;

#[route(get, "/changes", ok(operation::change::typ::Changes))]
async fn list(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::change::List>,
) -> Result<Json<operation::change::typ::Changes>> {
}

#[route(post, "/changes", created(operation::change::typ::Change))]
async fn create(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::change::Create>,
    super::extract::Body(body): super::extract::Body<operation::change::typ::CreateRequest>,
) -> Result<(StatusCode, Json<operation::change::typ::Change>)> {
}

#[route(get, "/changes/{id:Change id}", ok(operation::change::typ::Change))]
async fn get(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::change::Get>,
    super::extract::Path(id): super::extract::Path<Uuid>,
) -> Result<Json<operation::change::typ::Change>> {
}

#[route(post, "/changes/{id:Change id}/close", ok(operation::Done))]
async fn close(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::change::Close>,
    super::extract::Path(id): super::extract::Path<Uuid>,
) -> Result<Json<operation::Done>> {
}

#[route(post, "/changes/{id:Change id}/apply", ok(operation::Done))]
async fn apply(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::change::Apply>,
    super::extract::Path(id): super::extract::Path<Uuid>,
) -> Result<Json<operation::Done>> {
}

#[route(post, "/changes/{id:Change id}/complete", ok(operation::Done))]
async fn complete(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::change::Complete>,
    super::extract::Path(id): super::extract::Path<Uuid>,
) -> Result<Json<operation::Done>> {
}

pub(crate) fn routes() -> OpenApiRouter<Arc<App>> {
    OpenApiRouter::new()
        .routes(routes!(list, create))
        .routes(routes!(get))
        .routes(routes!(close))
        .routes(routes!(apply))
        .routes(routes!(complete))
}
