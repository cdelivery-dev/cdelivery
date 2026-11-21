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

#[route(get, "/nodes", ok(operation::node::typ::Nodes))]
async fn list(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::node::List>,
) -> Result<Json<operation::node::typ::Nodes>> {
}

#[route(
    post,
    "/nodes/join-invitations",
    ok(operation::node::typ::JoinInvitation)
)]
async fn create_join_invitation(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::node::CreateJoinInvitation>,
    super::extract::Body(dto): super::extract::Body<
        operation::node::typ::CreateJoinInvitationRequest,
    >,
) -> Result<Json<operation::node::typ::JoinInvitation>> {
}

#[route(get, "/nodes/silent-agents", ok(operation::node::typ::SilentAgents))]
async fn silent_agents(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::node::SilentAgents>,
) -> Result<Json<operation::node::typ::SilentAgents>> {
}

#[route(get, "/nodes/{id:node id}", ok(operation::node::typ::Node))]
async fn find(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::node::Get>,
    super::extract::Path(id): super::extract::Path<String>,
) -> Result<Json<operation::node::typ::Node>> {
}

#[route(
    get,
    "/nodes/{id:node id}/metrics",
    ok(operation::node::typ::NodeSeries)
)]
async fn metrics(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::node::Metrics>,
    super::extract::Path(id): super::extract::Path<String>,
    super::extract::Query(query): super::extract::Query<operation::typ::metric::RangeQuery>,
) -> Result<Json<operation::node::typ::NodeSeries>> {
}

#[route(patch, "/nodes/{id:node id}", ok(operation::node::typ::Node))]
async fn update(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::node::Relabel>,
    super::extract::Path(id): super::extract::Path<String>,
    super::extract::Body(dto): super::extract::Body<operation::node::typ::UpdateNodeRequest>,
) -> Result<Json<operation::node::typ::Node>> {
}

#[route(delete, "/nodes/{id:node id}", no_content)]
async fn remove(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::node::Delete>,
    super::extract::Path(id): super::extract::Path<String>,
) -> Result<StatusCode> {
}

pub(crate) fn routes() -> OpenApiRouter<Arc<App>> {
    OpenApiRouter::new()
        .routes(routes!(list))
        .routes(routes!(create_join_invitation))
        .routes(routes!(silent_agents))
        .routes(routes!(metrics))
        .routes(routes!(find, update, remove))
}
