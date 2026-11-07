use std::sync::Arc;

use axum::{Json, extract::State};
use http::StatusCode;
use macros_derive::route;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::boot::App;
use crate::error::Result;
use crate::operation;

#[route(post, "/secrets", created(operation::Done))]
async fn create(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::secret::Create>,
    super::extract::Body(dto): super::extract::Body<operation::secret::typ::CreateRequest>,
) -> Result<(StatusCode, Json<operation::Done>)> {
}

#[route(get, "/secrets", ok(operation::secret::typ::Secrets))]
async fn list(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::secret::List>,
    super::extract::Query(query): super::extract::Query<operation::secret::typ::Query>,
) -> Result<Json<operation::secret::typ::Secrets>> {
}

#[route(delete, "/secrets/{name:Secret name}", no_content)]
async fn delete(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::secret::Delete>,
    super::extract::Path(name): super::extract::Path<String>,
) -> Result<StatusCode> {
}

pub(crate) fn routes() -> OpenApiRouter<Arc<App>> {
    OpenApiRouter::new()
        .routes(routes!(create, list))
        .routes(routes!(delete))
}
