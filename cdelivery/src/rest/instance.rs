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

#[route(get, "/instance", ok(operation::instance::typ::Instance))]
async fn find(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::instance::Get>,
) -> Result<Json<operation::instance::typ::Instance>> {
}

#[route(get, "/overview", ok(operation::overview::typ::Overview))]
async fn overview(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::overview::Get>,
    super::extract::Query(query): super::extract::Query<operation::typ::metric::RangeQuery>,
) -> Result<Json<operation::overview::typ::Overview>> {
}

#[route(get, "/versions", ok(operation::instance::typ::Versions))]
async fn versions(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::instance::VersionList>,
) -> Result<Json<operation::instance::typ::Versions>> {
}

#[route(get, "/instance/logs", ok(operation::typ::log::Records))]
async fn logs(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::instance::Logs>,
    super::extract::Query(query): super::extract::Query<operation::typ::log::Query>,
) -> Result<Json<operation::typ::log::Records>> {
}

#[route(patch, "/instance", accepted)]
async fn update(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::instance::Update>,
    super::extract::Body(dto): super::extract::Body<
        operation::instance::typ::UpdateInstanceRequest,
    >,
) -> Result<StatusCode> {
}

pub(crate) fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        .routes(routes!(find))
        .routes(routes!(overview))
        .routes(routes!(versions))
        .routes(routes!(logs))
        .routes(routes!(update))
}
