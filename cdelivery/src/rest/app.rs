use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use macros_derive::route;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::boot::App as AppState;
use crate::error::Result;
use crate::operation;

#[route(get, "/apps/{name:app name}", ok(operation::app::typ::view::AppSpec))]
async fn spec(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::app::Get>,
    super::extract::Path(name): super::extract::Path<String>,
) -> Result<Json<operation::app::typ::view::AppSpec>> {
}

#[route(
    get,
    "/apps/{name:app name}/status",
    ok(operation::app::typ::view::AppStatus)
)]
async fn status(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::app::Status>,
    super::extract::Path(name): super::extract::Path<String>,
) -> Result<Json<operation::app::typ::view::AppStatus>> {
}

#[route(
    get,
    "/apps/{name:app name}/metrics",
    ok(operation::typ::metric::ContainerSeries)
)]
async fn metrics(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::app::Metrics>,
    super::extract::Path(name): super::extract::Path<String>,
    super::extract::Query(query): super::extract::Query<operation::typ::metric::Query>,
) -> Result<Json<operation::typ::metric::ContainerSeries>> {
}

#[route(get, "/apps/{name:app name}/logs", ok(operation::typ::log::Records))]
async fn logs(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::app::Logs>,
    super::extract::Path(name): super::extract::Path<String>,
    super::extract::Query(query): super::extract::Query<operation::typ::log::AppQuery>,
) -> Result<Json<operation::typ::log::Records>> {
}

#[route(
    get,
    "/apps/{name:app name}/logs/sources",
    ok(operation::typ::log::Sources)
)]
async fn log_sources(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::app::LogSources>,
    super::extract::Path(name): super::extract::Path<String>,
) -> Result<Json<operation::typ::log::Sources>> {
}

#[route(
    get,
    "/apps/{name:app name}/access",
    ok(operation::pingress::typ::AccessRecords)
)]
async fn access(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::app::Access>,
    super::extract::Path(name): super::extract::Path<String>,
    super::extract::Query(query): super::extract::Query<operation::pingress::typ::AppAccessQuery>,
) -> Result<Json<operation::pingress::typ::AccessRecords>> {
}

#[route(post, "/apps", created(operation::app::typ::view::AppSpec))]
async fn create(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::app::Create>,
    super::extract::Body(dto): super::extract::Body<
        operation::app::typ::mutation::CreateAppRequest,
    >,
) -> Result<(StatusCode, Json<operation::app::typ::view::AppSpec>)> {
}

#[route(get, "/apps", ok(operation::app::typ::view::Apps))]
async fn list(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::app::List>,
) -> Result<Json<operation::app::typ::view::Apps>> {
}

#[route(
    patch,
    "/apps/{name:app name}",
    output = operation::app::Updated,
    ok(operation::app::typ::view::AppSpec),
    accepted(operation::guardian::typ::Guardian)
)]
async fn update(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::app::Update>,
    super::extract::Path(name): super::extract::Path<String>,
    super::extract::Body(dto): super::extract::Body<operation::app::typ::mutation::Changes>,
) -> Result<axum::response::Response> {
    Ok(
        match authorized
            .execute(&state, operation::app::UpdateInput { name, changes: dto })
            .await?
        {
            operation::app::Updated::Applied(spec) => (StatusCode::OK, Json(spec)).into_response(),
            operation::app::Updated::Guarded(guardian) => {
                (StatusCode::ACCEPTED, Json(guardian)).into_response()
            }
        },
    )
}

#[route(delete, "/apps/{name:app name}", no_content)]
async fn remove(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::app::Delete>,
    super::extract::Path(name): super::extract::Path<String>,
) -> Result<StatusCode> {
}

#[route(
    post,
    "/apps/{name:app name}/rollback",
    ok(operation::app::typ::view::AppSpec)
)]
async fn rollback(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::app::Rollback>,
    super::extract::Path(name): super::extract::Path<String>,
) -> Result<Json<operation::app::typ::view::AppSpec>> {
}

#[route(post, "/apps/{name:app name}/restart", no_content)]
async fn restart(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::app::Restart>,
    super::extract::Path(name): super::extract::Path<String>,
) -> Result<StatusCode> {
}

#[route(
    get,
    "/apps/{name:app name}/xconfig/{config:config name}/file",
    ok(super::BinaryArtifact, "application/octet-stream")
)]
async fn download_xconfig(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::app::DownloadXConfig>,
    super::extract::Path((name, config)): super::extract::Path<(String, String)>,
) -> Result<impl IntoResponse> {
}

pub(crate) fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        .routes(routes!(create, list))
        .routes(routes!(status))
        .routes(routes!(metrics))
        .routes(routes!(logs))
        .routes(routes!(log_sources))
        .routes(routes!(access))
        .routes(routes!(spec, update, remove))
        .routes(routes!(rollback))
        .routes(routes!(restart))
        .routes(routes!(download_xconfig))
}
