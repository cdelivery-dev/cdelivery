use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use macros_derive::route;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::boot::App as AppState;
use crate::error::Result;
use crate::operation;

#[route(get, "/events/conditions", ok(operation::event::typ::Conditions))]
async fn conditions(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::event::Conditions>,
) -> Result<Json<operation::event::typ::Conditions>> {
}

#[route(get, "/events", ok(operation::event::typ::EventRecords))]
async fn list(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::event::List>,
    super::extract::Query(q): super::extract::Query<operation::event::typ::EventQuery>,
) -> Result<Json<operation::event::typ::EventRecords>> {
}

pub(crate) fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        .routes(routes!(list))
        .routes(routes!(conditions))
}
