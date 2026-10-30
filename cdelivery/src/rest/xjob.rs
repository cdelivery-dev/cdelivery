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

#[route(get, "/apps/{name:app name}/xjobs", ok(operation::xjob::typ::XJobs))]
async fn list(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::xjob::List>,
    super::extract::Path(name): super::extract::Path<String>,
) -> Result<Json<operation::xjob::typ::XJobs>> {
}

#[route(
    put,
    "/apps/{name:app name}/xjobs/{job:job name}",
    created(operation::xjob::typ::XJob)
)]
async fn create(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::xjob::Create>,
    super::extract::Path((name, job)): super::extract::Path<(String, String)>,
    super::extract::Body(dto): super::extract::Body<operation::xjob::typ::CreateXJobRequest>,
) -> Result<(StatusCode, Json<operation::xjob::typ::XJob>)> {
}

#[route(
    get,
    "/apps/{name:app name}/xjobs/{job:job name}",
    ok(operation::xjob::typ::XJob)
)]
async fn find(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::xjob::Get>,
    super::extract::Path((name, job)): super::extract::Path<(String, String)>,
) -> Result<Json<operation::xjob::typ::XJob>> {
}

#[route(
    patch,
    "/apps/{name:app name}/xjobs/{job:job name}",
    ok(operation::xjob::typ::XJob)
)]
async fn update(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::xjob::Update>,
    super::extract::Path((name, job)): super::extract::Path<(String, String)>,
    super::extract::Body(dto): super::extract::Body<operation::xjob::typ::UpdateXJobRequest>,
) -> Result<Json<operation::xjob::typ::XJob>> {
}

#[route(delete, "/apps/{name:app name}/xjobs/{job:job name}", no_content)]
async fn remove(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::xjob::Delete>,
    super::extract::Path((name, job)): super::extract::Path<(String, String)>,
) -> Result<StatusCode> {
}

#[route(
    post,
    "/apps/{name:app name}/xjobs/{job:job name}/run",
    accepted(operation::xjob::typ::XJob)
)]
async fn run(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::xjob::Run>,
    super::extract::Path((name, job)): super::extract::Path<(String, String)>,
) -> Result<(StatusCode, Json<operation::xjob::typ::XJob>)> {
}

pub(crate) fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        .routes(routes!(list))
        .routes(routes!(create, find, update, remove))
        .routes(routes!(run))
}
