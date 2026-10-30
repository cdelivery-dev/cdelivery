use std::collections::BTreeMap;

use askama::Template;
use axum::response::{Html, IntoResponse, Response};
use http::StatusCode;
use http::header::{CACHE_CONTROL, HeaderName, HeaderValue, VARY, X_CONTENT_TYPE_OPTIONS};

use crate::error::ErrorKind;
use crate::locale;
use crate::locale::tt;

const CONTENT_SECURITY_POLICY: HeaderName = HeaderName::from_static("content-security-policy");
const CSP: &str = "default-src 'self' 'unsafe-inline' 'unsafe-eval'; img-src 'self' https: data:; frame-ancestors 'none'";
const SCALAR_CSP: &str = "default-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net/npm/@scalar/api-reference@1.65.1/dist/browser/standalone.js; img-src 'self' https:; frame-ancestors 'none'";

pub(super) type Result<T> = std::result::Result<T, Error>;

pub(super) struct Error(crate::error::Error);

#[derive(askama::Template)]
#[template(path = "error.html")]
struct ErrorPage<'a> {
    status: u16,
    code: &'static str,
    detail: &'a str,
    fields: &'a BTreeMap<String, String>,
}

impl From<crate::error::Error> for Error {
    fn from(error: crate::error::Error) -> Self {
        Self(error)
    }
}

impl From<ErrorKind> for Error {
    #[track_caller]
    fn from(kind: ErrorKind) -> Self {
        Self(kind.into())
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        if super::setup::json() {
            return self.0.into_response();
        }
        error(&self.0)
    }
}

pub(super) fn render(template: &impl Template, status: StatusCode) -> Result<Response> {
    let html = match template.render() {
        Ok(html) => html,
        Err(error) => return Err(crate::error::Error::from(error).into()),
    };
    Ok(decorate((status, Html(html)).into_response()))
}

pub(super) fn decorate(response: Response) -> Response {
    decorate_with_csp(response, CSP)
}

pub(super) fn decorate_scalar(response: Response) -> Response {
    decorate_with_csp(response, SCALAR_CSP)
}

fn decorate_with_csp(mut response: Response, csp: &'static str) -> Response {
    let headers = response.headers_mut();
    for (name, value) in [
        (CACHE_CONTROL, "no-store"),
        (VARY, "Accept-Language"),
        (X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (CONTENT_SECURITY_POLICY, csp),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    response
}

fn error(error: &crate::error::Error) -> Response {
    let status = error.status();
    if status.is_server_error() {
        tracing::error!(error = ?error, "UI request failed");
    }
    let locale = locale::current();
    let (detail, fields) = error.detail(locale);
    let fields = fields.unwrap_or_default();
    let page = ErrorPage {
        status: status.as_u16(),
        code: <&'static str>::from(error.kind()),
        detail: &detail,
        fields: &fields,
    };
    match page.render() {
        Ok(html) => decorate((status, Html(html)).into_response()),
        Err(render_error) => {
            let render_error = crate::error::Error::from(render_error);
            tracing::error!(error = ?render_error, "HTML error page rendering failed");
            minimal_500()
        }
    }
}

fn minimal_500() -> Response {
    let language = locale::current().tag();
    let title = tt!("ui.error.title");
    decorate(
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Html(format!("<!doctype html><html lang=\"{language}\"><head><meta charset=\"utf-8\"><title>{title} | CDelivery</title></head><body><h1>500</h1><p>{title}</p></body></html>")),
        )
            .into_response(),
    )
}
