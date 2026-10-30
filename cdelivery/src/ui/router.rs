use std::cell::Cell;
use std::sync::Arc;

use ::http::Method;
use ::http::header::{ACCEPT, ACCEPT_LANGUAGE, CACHE_CONTROL, HeaderValue, VARY};
use axum::extract::{DefaultBodyLimit, Request};
use axum::middleware::{Next, from_fn, from_fn_with_state, map_response};
use axum::response::{IntoResponse, Response};
use utoipa_scalar::{Scalar, Servable};

use crate::boot::{App, Setup};
use crate::http;
use crate::locale;

type Error = super::response::Error;

pub(crate) fn app(state: &Arc<App>, api: utoipa::openapi::OpenApi) -> axum::Router<Arc<App>> {
    let forms =
        super::auth::routes().layer(DefaultBodyLimit::max(crate::config::PUBLIC_BODY_LIMIT));
    let pages = axum::Router::new()
        .merge(super::group::routes())
        .merge(super::account::routes())
        .merge(super::settings::routes())
        .merge(super::node::routes())
        .merge(super::overview::routes())
        .merge(super::target::routes())
        .merge(super::volume::routes())
        .layer(DefaultBodyLimit::max(crate::config::PUBLIC_BODY_LIMIT));
    let apps = super::app::routes().layer(DefaultBodyLimit::max(crate::config::JSON_BODY_LIMIT));
    let insight =
        super::insight::routes().layer(DefaultBodyLimit::max(crate::config::JSON_BODY_LIMIT));
    let pingress =
        super::pingress::routes().layer(DefaultBodyLimit::max(super::pingress::BODY_LIMIT));
    let scalar: axum::Router<Arc<App>> = Scalar::with_url("/scalar", api)
        .custom_html(
            include_str!("../../templates/scalar.html")
                .replace("$favicon_light", super::asset::FAVICON_LIGHT_PATH.as_str())
                .replace("$favicon_dark", super::asset::FAVICON_DARK_PATH.as_str()),
        )
        .into();
    let scalar = scalar.layer(map_response(|response: Response| async {
        super::response::decorate_scalar(response)
    }));
    let protected = axum::Router::new()
        .merge(pages)
        .merge(apps)
        .merge(insight)
        .merge(pingress)
        .merge(scalar)
        .layer(from_fn_with_state(state.clone(), super::auth::protect))
        .layer(from_fn_with_state(
            crate::config::PROTECTED_TRANSPORT_TIMEOUT,
            http::middleware::with_timeout::<Error>,
        ));
    let public = axum::Router::new()
        .merge(super::asset::routes())
        .merge(forms)
        .route(
            "/setup",
            axum::routing::get(|| async { redirect("/login") })
                .post(|| async { Error::from(crate::error::ErrorKind::AlreadyInitialized) }),
        )
        .layer(from_fn(require_same_origin))
        .layer(from_fn_with_state(
            crate::config::PUBLIC_TRANSPORT_TIMEOUT,
            http::middleware::with_timeout::<Error>,
        ));
    http::middleware::edge::<_, Error>(public.merge(protected)).layer(from_fn(with_locale))
}

pub(crate) fn setup() -> axum::Router<Arc<Setup>> {
    let forms =
        super::setup::routes().layer(DefaultBodyLimit::max(crate::config::PUBLIC_BODY_LIMIT));
    http::middleware::edge::<_, Error>(
        axum::Router::new()
            .merge(super::asset::routes())
            .merge(forms)
            .route("/", axum::routing::get(|| async { redirect("/setup") }))
            .layer(from_fn(require_same_origin))
            .layer(from_fn_with_state(
                crate::config::PUBLIC_TRANSPORT_TIMEOUT,
                http::middleware::with_timeout::<Error>,
            )),
    )
    .layer(from_fn(with_locale))
}

async fn with_locale(req: Request, next: Next) -> Response {
    let setup = req.method() == Method::POST && req.uri().path() == "/setup";
    let json = setup
        && req
            .headers()
            .get(ACCEPT)
            .is_some_and(|value| value == "application/json");
    let locale = if json {
        locale::Locale::En
    } else {
        locale::negotiate(
            req.headers()
                .get(ACCEPT_LANGUAGE)
                .and_then(|value| value.to_str().ok()),
        )
    };
    let mut response = super::setup::JSON
        .scope(json, locale::LOCALE.scope(Cell::new(locale), next.run(req)))
        .await;
    if setup {
        let headers = response.headers_mut();
        headers.append(VARY, HeaderValue::from_static("Accept"));
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    response
}

pub(super) fn redirect(location: &str) -> Response {
    super::response::decorate(axum::response::Redirect::to(location).into_response())
}

async fn require_same_origin(req: Request, next: Next) -> Response {
    if req.method() == Method::POST
        && let Err(error) = http::same_origin::validate(req.headers())
    {
        return Error::from(error).into_response();
    }
    next.run(req).await
}
