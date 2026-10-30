pub(crate) mod extract;
pub(crate) mod internal;
mod join;
pub(crate) mod middleware;
pub(crate) mod same_origin;
pub(crate) mod session;

use std::sync::Arc;
use std::time::Instant;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::middleware::from_fn_with_state;
use axum::response::Response;
use http::header::HeaderName;
use http::{HeaderMap, HeaderValue};
use tokio_util::sync::CancellationToken;
use tower_http::catch_panic::CatchPanicLayer;
use utoipa::OpenApi;
use utoipa::openapi;
use utoipa_axum::router::OpenApiRouter;

use crate::boot::{App, Setup};
use crate::error::{Error, ErrorKind, Result};
use crate::{mcp, operation, rest, ui};

const API_PREFIX: &str = "/api";
const MCP_PATH: &str = "/mcp";
const X_FORWARDED_PROTO: HeaderName = HeaderName::from_static("x-forwarded-proto");
const SERVER_TIMING: HeaderName = HeaderName::from_static("server-timing");
const METHODS: [&str; 5] = ["get", "put", "post", "delete", "patch"];

#[derive(OpenApi)]
#[openapi(info(title = "CDelivery"))]
struct ApiDoc;

pub(crate) fn setup_router(state: Arc<Setup>) -> Router {
    middleware::apply_common(ui::router::setup().with_state(state))
}

pub(crate) fn app_router(state: Arc<App>, cancel: CancellationToken) -> Result<Router> {
    let protected = rest::protected_routes()
        .layer(DefaultBodyLimit::max(crate::config::JSON_BODY_LIMIT))
        .layer(from_fn_with_state(state.clone(), middleware::require_auth))
        .layer(from_fn_with_state(
            crate::config::PROTECTED_TRANSPORT_TIMEOUT,
            middleware::with_timeout::<Error>,
        ));
    let (router, api) = document(protected)?;
    let mcp = Router::<Arc<App>>::new()
        .route_service(MCP_PATH, mcp::service(&state, cancel))
        .layer(from_fn_with_state(state.clone(), middleware::require_auth))
        .layer(from_fn_with_state(
            crate::config::PROTECTED_TRANSPORT_TIMEOUT,
            middleware::with_timeout::<Error>,
        ))
        .layer(CatchPanicLayer::custom(middleware::panic_response::<Error>));
    let root = ui::router::app(&state, api)
        .merge(mcp)
        .merge(join::routes());
    Ok(build(router, root, state))
}

pub(crate) fn openapi() -> Result<openapi::OpenApi> {
    document(rest::protected_routes()).map(|(_, api)| api)
}

fn document<S>(endpoints: OpenApiRouter<S>) -> Result<(Router<S>, openapi::OpenApi)>
where
    S: Clone + Send + Sync + 'static,
{
    let (router, mut api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .merge(endpoints)
        .split_for_parts();
    api.paths.paths = std::mem::take(&mut api.paths.paths)
        .into_iter()
        .map(|(path, item)| (format!("{API_PREFIX}{path}"), item))
        .collect();
    api.tags = Some(
        rest::API_TAGS
            .iter()
            .copied()
            .map(openapi::Tag::new)
            .collect(),
    );
    let components = api.components.get_or_insert_with(openapi::Components::new);
    components.add_security_scheme(
        crate::config::SESSION_COOKIE,
        openapi::security::SecurityScheme::ApiKey(openapi::security::ApiKey::Cookie(
            openapi::security::ApiKeyValue::new(crate::config::SESSION_COOKIE),
        )),
    );
    components.add_security_scheme(
        "bearer",
        openapi::security::SecurityScheme::Http(openapi::security::Http::new(
            openapi::security::HttpAuthScheme::Bearer,
        )),
    );
    api.security = Some(vec![
        openapi::security::SecurityRequirement::new("bearer", Vec::<String>::new()),
        openapi::security::SecurityRequirement::new(
            crate::config::SESSION_COOKIE,
            Vec::<String>::new(),
        ),
    ]);
    operation::schema::normalize_openapi(&mut api)?;
    validate_contract(&api)?;
    Ok((router, api))
}

fn build<S>(router: Router<S>, root: Router<S>, state: S) -> Router
where
    S: Clone + Send + Sync + 'static,
{
    let api = middleware::edge::<_, Error>(router);
    let router = Router::new()
        .nest(API_PREFIX, api)
        .merge(root)
        .with_state(state);
    middleware::apply_common(router)
}

fn validate_contract(api: &openapi::OpenApi) -> Result<()> {
    for route in rest::ROUTES {
        let registered = api
            .paths
            .paths
            .get(&format!("{API_PREFIX}{}", route.path))
            .and_then(|item| operation(item, route.method))
            .is_some_and(|operation| operation.operation_id.as_deref() == Some(route.id));
        if !registered {
            tracing::error!(
                "catalog operation {} has no matching REST route {} {}",
                route.id,
                route.method,
                route.path
            );
            return Err(ErrorKind::InvalidOperationOutput.into());
        }
    }
    let documented = api
        .paths
        .paths
        .values()
        .flat_map(|item| METHODS.iter().filter_map(|method| operation(item, method)))
        .count();
    if documented != rest::ROUTES.len() {
        tracing::error!("OpenAPI operation count does not match the REST catalog");
        return Err(ErrorKind::InvalidOperationOutput.into());
    }
    Ok(())
}

fn api_relative_path(path: &str) -> Option<&str> {
    path.strip_prefix(API_PREFIX)
        .filter(|path| path.starts_with('/'))
}

pub(crate) fn server_timing(mut response: Response, started: Instant) -> Response {
    let millis = started.elapsed().as_secs_f64() * 1000.0;
    if let Ok(value) = HeaderValue::try_from(format!("app;dur={millis:.1}")) {
        response.headers_mut().insert(SERVER_TIMING, value);
    }
    response
}

pub(crate) fn is_https(headers: &HeaderMap) -> Result<bool> {
    let mut values = headers.get_all(&X_FORWARDED_PROTO).iter();
    let Some(value) = values.next() else {
        return Ok(false);
    };
    if values.next().is_some() {
        return Err(ErrorKind::Forbidden.into());
    }
    match value.to_str() {
        Ok("http") => Ok(false),
        Ok("https") => Ok(true),
        Ok(_) | Err(_) => Err(ErrorKind::Forbidden.into()),
    }
}

pub(crate) fn panic_detail(payload: &dyn std::any::Any) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic payload".to_owned())
}

fn operation<'a>(
    item: &'a openapi::path::PathItem,
    method: &str,
) -> Option<&'a openapi::path::Operation> {
    match method {
        "get" => item.get.as_ref(),
        "put" => item.put.as_ref(),
        "post" => item.post.as_ref(),
        "delete" => item.delete.as_ref(),
        "patch" => item.patch.as_ref(),
        _ => None,
    }
}
