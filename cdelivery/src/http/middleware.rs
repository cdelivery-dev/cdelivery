use std::sync::Arc;

use axum::Router;
use axum::extract::{MatchedPath, Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::CookieJar;
use http::header::{AUTHORIZATION, CACHE_CONTROL, COOKIE, SET_COOKIE, WWW_AUTHENTICATE};
use http::{HeaderMap, HeaderValue};
use jiff::Timestamp;
use tower::ServiceBuilder;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::compression::CompressionLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::sensitive_headers::{
    SetSensitiveRequestHeadersLayer, SetSensitiveResponseHeadersLayer,
};
use tower_http::trace::TraceLayer;

use crate::boot::App;
use crate::error::{Error, ErrorKind};
use crate::{concept, operation, rest, usecase};

pub(super) fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let mut values = headers.get_all(AUTHORIZATION).iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        return None;
    }
    let (scheme, token) = value.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer")
        && !token.is_empty()
        && !token.bytes().any(|byte| byte.is_ascii_whitespace()))
    .then_some(token)
}

pub(crate) async fn require_auth(
    State(app): State<Arc<App>>,
    mut req: Request,
    next: Next,
) -> Response {
    let Some(path) = req
        .extensions()
        .get::<MatchedPath>()
        .map(MatchedPath::as_str)
    else {
        return no_store(Error::new(ErrorKind::InvalidOperationOutput).into_response());
    };
    let (origin, spec) = if path == super::MCP_PATH {
        (concept::actor::Origin::Mcp, None)
    } else {
        let Some(path) = super::api_relative_path(path) else {
            return no_store(Error::new(ErrorKind::InvalidOperationOutput).into_response());
        };
        (
            concept::actor::Origin::Rest,
            rest::for_request(req.method(), path),
        )
    };
    let had_authorization = req.headers().contains_key(AUTHORIZATION);
    let now = Timestamp::now();
    let (cookies, result) = if had_authorization {
        let result = match bearer_token(req.headers()) {
            Some(token) => {
                usecase::access::api_key(&app.docker, &app.key, token, origin, now).await
            }
            None => Err(ErrorKind::ApiKeyInvalid.into()),
        }
        .map(|access| (access, None));
        (None, result)
    } else {
        let cookies = CookieJar::from_headers(req.headers());
        let result = match cookies.get(crate::config::SESSION_COOKIE) {
            Some(cookie) => {
                usecase::access::session(&app.docker, &app.key, cookie.value(), origin, now)
                    .await
                    .map(|(access, _, renewal)| (access, renewal))
            }
            None => Err(ErrorKind::AuthRequired.into()),
        };
        (Some(cookies), result)
    };
    let response = match result {
        Ok((access, renewal)) => {
            let renewal = match renewal
                .map(|token| super::session::cookie(req.headers(), token))
                .transpose()
            {
                Ok(renewal) => renewal,
                Err(error) => return no_store(error.into_response()),
            };
            let cookie_write = !had_authorization
                && if matches!(origin, concept::actor::Origin::Mcp) {
                    req.method() == http::Method::POST
                } else {
                    spec.is_none_or(|spec| matches!(spec.effect, operation::Effect::Write { .. }))
                };
            let response = if spec.is_some_and(|spec| !access.allows(spec.requirement)) {
                Error::new(ErrorKind::Forbidden).into_response()
            } else if cookie_write && let Err(error) = super::same_origin::validate(req.headers()) {
                error.into_response()
            } else {
                req.extensions_mut().insert(access);
                let started = std::time::Instant::now();
                super::server_timing(next.run(req).await, started)
            };
            match cookies {
                Some(jar) => super::session::apply_renewal(jar, renewal, response),
                None => response,
            }
        }
        Err(e) => {
            let status = e.status();
            let response = e.into_response();
            if status == http::StatusCode::UNAUTHORIZED {
                challenge(response, had_authorization)
            } else {
                response
            }
        }
    };
    no_store(response)
}

fn challenge(mut resp: Response, had_authorization: bool) -> Response {
    let value = if had_authorization {
        "Bearer error=\"invalid_token\""
    } else {
        "Bearer"
    };
    resp.headers_mut()
        .insert(WWW_AUTHENTICATE, HeaderValue::from_static(value));
    resp
}

pub(super) fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn panic_response<E: From<ErrorKind> + IntoResponse>(
    payload: Box<dyn std::any::Any + Send>,
) -> Response {
    E::from(ErrorKind::Panic(super::panic_detail(&*payload))).into_response()
}

pub(crate) async fn with_timeout<E: From<ErrorKind> + IntoResponse>(
    State(budget): State<jiff::SignedDuration>,
    req: Request,
    next: Next,
) -> Response {
    match tokio::time::timeout(budget.unsigned_abs(), next.run(req)).await {
        Ok(response) => response,
        Err(source) => E::from(ErrorKind::OperationTimeout {
            operation: "transport",
            source,
        })
        .into_response(),
    }
}

pub(crate) fn edge<S, E>(router: Router<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    E: From<ErrorKind> + IntoResponse + Send + 'static,
{
    router
        .fallback(|| async { E::from(ErrorKind::RouteNotFound).into_response() })
        .method_not_allowed_fallback(|| async {
            E::from(ErrorKind::MethodNotAllowed).into_response()
        })
        .layer(CatchPanicLayer::custom(panic_response::<E>))
}

pub(crate) fn apply_common(router: Router) -> Router {
    router.layer(
        ServiceBuilder::new()
            .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
            .layer(PropagateRequestIdLayer::x_request_id())
            .layer(SetSensitiveRequestHeadersLayer::new([
                AUTHORIZATION,
                COOKIE,
            ]))
            .layer(TraceLayer::new_for_http())
            .layer(SetSensitiveResponseHeadersLayer::new([SET_COOKIE]))
            .layer(CompressionLayer::new()),
    )
}
