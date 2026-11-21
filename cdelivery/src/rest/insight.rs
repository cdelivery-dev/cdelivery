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

#[route(patch, "/insight", ok(operation::insight::typ::Insight))]
async fn enable(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::insight::Enable>,
    super::extract::Body(dto): super::extract::Body<operation::insight::typ::EnableInsightRequest>,
) -> Result<Json<operation::insight::typ::Insight>> {
}

#[route(get, "/insight", ok(operation::insight::typ::Insight))]
async fn find(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::insight::Overview>,
) -> Result<Json<operation::insight::typ::Insight>> {
}

#[route(
    get,
    "/insight/{component:component}/metrics",
    ok(operation::typ::metric::ContainerSeries)
)]
async fn metrics(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::insight::Metrics>,
    super::extract::Path(component): super::extract::Path<operation::insight::typ::Component>,
    super::extract::Query(query): super::extract::Query<operation::typ::metric::Query>,
) -> Result<Json<operation::typ::metric::ContainerSeries>> {
}

#[route(delete, "/insight", no_content)]
async fn disable(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::insight::Disable>,
) -> Result<StatusCode> {
}

pub(crate) fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        .routes(routes!(enable, find, disable))
        .routes(routes!(metrics))
}
