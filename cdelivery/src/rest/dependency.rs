use std::sync::Arc;

use axum::{Json, extract::State};
use http::StatusCode;
use macros_derive::route;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::boot::App;
use crate::error::Result;
use crate::operation;

#[route(
    get,
    "/dependencies/.planetscale-sizes",
    ok(operation::dependency::typ::PlanetScaleSizes)
)]
async fn planetscale_sizes(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::dependency::PlanetScaleSizes>,
    super::extract::Query(query): super::extract::Query<
        operation::dependency::typ::PlanetScaleSizesQuery,
    >,
) -> Result<Json<operation::dependency::typ::PlanetScaleSizes>> {
}

#[route(get, "/dependencies", ok(operation::dependency::typ::Dependencies))]
async fn list(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::dependency::List>,
    super::extract::Query(query): super::extract::Query<operation::dependency::typ::Query>,
) -> Result<Json<operation::dependency::typ::Dependencies>> {
}

#[route(post, "/dependencies", created(operation::Done))]
async fn create(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::dependency::Create>,
    super::extract::Body(body): super::extract::Body<operation::dependency::typ::CreateRequest>,
) -> Result<(StatusCode, Json<operation::Done>)> {
}

#[route(
    get,
    "/dependencies/{name:Dependency name}",
    ok(operation::dependency::typ::Dependency)
)]
async fn get(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::dependency::Get>,
    super::extract::Path(name): super::extract::Path<String>,
) -> Result<Json<operation::dependency::typ::Dependency>> {
}

#[route(delete, "/dependencies/{name:Dependency name}", no_content)]
async fn delete(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::dependency::Delete>,
    super::extract::Path(name): super::extract::Path<String>,
) -> Result<StatusCode> {
}

pub(crate) fn routes() -> OpenApiRouter<Arc<App>> {
    OpenApiRouter::new()
        .routes(routes!(planetscale_sizes))
        .routes(routes!(list, create))
        .routes(routes!(get, delete))
}
