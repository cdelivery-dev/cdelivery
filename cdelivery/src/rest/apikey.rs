use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use macros_derive::route;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::boot::App;
use crate::error::Result;
use crate::operation;

#[route(post, "/apikeys", created(operation::apikey::typ::IssuedApiKey))]
async fn create(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::apikey::Create>,
    super::extract::Body(dto): super::extract::Body<operation::apikey::typ::CreateApiKeyRequest>,
) -> Result<(StatusCode, Json<operation::apikey::typ::IssuedApiKey>)> {
}

#[route(get, "/apikeys", ok(operation::apikey::typ::ApiKeys))]
async fn list(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::apikey::List>,
) -> Result<Json<operation::apikey::typ::ApiKeys>> {
}

#[route(delete, "/apikeys/{id:API key id}", no_content)]
async fn revoke(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::apikey::Revoke>,
    super::extract::Path(id): super::extract::Path<Uuid>,
) -> Result<StatusCode> {
}

pub(crate) fn routes() -> OpenApiRouter<Arc<App>> {
    OpenApiRouter::new()
        .routes(routes!(create, list))
        .routes(routes!(revoke))
}
