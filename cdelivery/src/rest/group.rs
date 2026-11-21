use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use macros_derive::route;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::boot::App;
use crate::error::Result;
use crate::operation;

#[route(post, "/groups", created(operation::group::typ::Group))]
async fn create(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::group::Create>,
    super::extract::Body(dto): super::extract::Body<operation::group::typ::CreateGroupRequest>,
) -> Result<(StatusCode, Json<operation::group::typ::Group>)> {
}

#[route(get, "/groups", ok(operation::group::typ::Groups))]
async fn list(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::group::List>,
) -> Result<Json<operation::group::typ::Groups>> {
}

#[route(get, "/groups/{name:group name}", ok(operation::group::typ::Group))]
async fn find(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::group::Get>,
    super::extract::Path(name): super::extract::Path<String>,
) -> Result<Json<operation::group::typ::Group>> {
}

#[route(patch, "/groups/{name:group name}", ok(operation::group::typ::Group))]
async fn update(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::group::Update>,
    super::extract::Path(name): super::extract::Path<String>,
    super::extract::Body(dto): super::extract::Body<operation::group::typ::UpdateGroupRequest>,
) -> Result<Json<operation::group::typ::Group>> {
}

#[route(delete, "/groups/{name:group name}", no_content)]
async fn remove(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::group::Delete>,
    super::extract::Path(name): super::extract::Path<String>,
) -> Result<StatusCode> {
}

pub(crate) fn routes() -> OpenApiRouter<Arc<App>> {
    OpenApiRouter::new()
        .routes(routes!(create, list))
        .routes(routes!(find, update, remove))
}
