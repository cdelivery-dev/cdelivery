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

#[route(put, "/pingress", created(operation::pingress::typ::Pingress))]
async fn enable(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::pingress::Enable>,
    super::extract::Body(dto): super::extract::Body<
        operation::pingress::typ::CreatePingressRequest,
    >,
) -> Result<(StatusCode, Json<operation::pingress::typ::Pingress>)> {
}

#[route(get, "/pingress", ok(operation::pingress::typ::Pingress))]
async fn find(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::pingress::Get>,
) -> Result<Json<operation::pingress::typ::Pingress>> {
}

#[route(get, "/pingress/metrics", ok(operation::typ::metric::ContainerSeries))]
async fn metrics(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::pingress::Metrics>,
    super::extract::Query(query): super::extract::Query<operation::typ::metric::Query>,
) -> Result<Json<operation::typ::metric::ContainerSeries>> {
}

#[route(get, "/pingress/access", ok(operation::pingress::typ::AccessRecords))]
async fn access(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::pingress::Access>,
    super::extract::Query(query): super::extract::Query<operation::pingress::typ::AccessQuery>,
) -> Result<Json<operation::pingress::typ::AccessRecords>> {
}

#[route(
    get,
    "/pingress/access/stats",
    ok(operation::pingress::typ::AccessStats)
)]
async fn access_stats(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::pingress::AccessStats>,
    super::extract::Query(query): super::extract::Query<operation::pingress::typ::AccessStatsQuery>,
) -> Result<Json<operation::pingress::typ::AccessStats>> {
}

#[route(get, "/pingress/logs", ok(operation::typ::log::Records))]
async fn logs(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::pingress::Logs>,
    super::extract::Query(query): super::extract::Query<operation::typ::log::Query>,
) -> Result<Json<operation::typ::log::Records>> {
}

#[route(patch, "/pingress", ok(operation::pingress::typ::Pingress))]
async fn update(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::pingress::Update>,
    super::extract::Body(dto): super::extract::Body<
        operation::pingress::typ::UpdatePingressRequest,
    >,
) -> Result<Json<operation::pingress::typ::Pingress>> {
}

#[route(delete, "/pingress", no_content)]
async fn disable(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::pingress::Disable>,
) -> Result<StatusCode> {
}

#[route(
    put,
    "/pingress/account",
    output = operation::pingress::typ::AcmeAccount,
    ok(operation::pingress::typ::AcmeAccount),
    created(operation::pingress::typ::AcmeAccount)
)]
async fn enable_account(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::pingress::EnableAccount>,
    super::extract::Body(dto): super::extract::Body<operation::pingress::typ::EnableAcmeRequest>,
) -> Result<(StatusCode, Json<operation::pingress::typ::AcmeAccount>)> {
    let account = authorized.execute(&state, dto).await?;
    let status = if account.created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(account)))
}

#[route(delete, "/pingress/account", no_content)]
async fn disable_account(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::pingress::DisableAccount>,
) -> Result<StatusCode> {
}

#[route(get, "/pingress/certs", ok(operation::pingress::typ::Certs))]
async fn list_certs(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::pingress::ListCerts>,
) -> Result<Json<operation::pingress::typ::Certs>> {
}

#[route(put, "/pingress/certs/{domain:certificate domain}", created)]
async fn upload_cert(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::pingress::UploadCert>,
    super::extract::Path(domain): super::extract::Path<String>,
    super::extract::Body(dto): super::extract::Body<operation::pingress::typ::UploadCertRequest>,
) -> Result<StatusCode> {
}

#[route(delete, "/pingress/certs/{domain:certificate domain}", no_content)]
async fn remove_cert(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::pingress::RemoveCert>,
    super::extract::Path(domain): super::extract::Path<String>,
) -> Result<StatusCode> {
}

pub(crate) fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        .routes(routes!(enable, find, update, disable))
        .routes(routes!(metrics))
        .routes(routes!(access))
        .routes(routes!(access_stats))
        .routes(routes!(logs))
        .routes(routes!(enable_account, disable_account))
        .routes(routes!(list_certs))
        .routes(routes!(upload_cert, remove_cert))
}
