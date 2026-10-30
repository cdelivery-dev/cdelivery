use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::CookieJar;
use garde::Validate;
use http::{HeaderMap, StatusCode};
use jiff::Timestamp;

use crate::boot::App;
use crate::error::ErrorKind;
use crate::locale::tt;
use crate::operation;

pub(super) type Authorized<O> = crate::http::extract::Authorized<O, super::response::Error>;

#[derive(serde::Deserialize, Validate)]
#[serde(deny_unknown_fields)]
struct LoginPayload {
    #[garde(email, length(max = 254))]
    email: String,
    #[garde(custom(operation::validate::sha256_hex))]
    password_sha256: String,
    #[garde(inner(custom(operation::validate::totp_code)))]
    totp_code: Option<String>,
}

#[derive(askama::Template)]
#[template(path = "login.html")]
struct LoginPage<'a> {
    email: &'a str,
    fields: &'a garde::Report,
}

pub(super) fn routes() -> axum::Router<Arc<App>> {
    axum::Router::new().route("/login", axum::routing::get(get).post(post))
}

pub(super) async fn protect(
    State(app): State<Arc<App>>,
    jar: CookieJar,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(cookie) = jar.get(crate::config::SESSION_COOKIE) else {
        return login_redirect(jar);
    };
    match crate::usecase::access::session(
        &app.docker,
        &app.key,
        cookie.value(),
        crate::concept::actor::Origin::Rest,
        Timestamp::now(),
    )
    .await
    {
        Ok((access, preferences, renewal)) => {
            let locale = preferences
                .language()
                .unwrap_or_else(crate::locale::current);
            if crate::locale::LOCALE
                .try_with(|current| current.set(locale))
                .is_err()
            {
                return super::response::Error::from(ErrorKind::InvalidOperationOutput)
                    .into_response();
            }
            let renewal = match renewal
                .map(|token| crate::http::session::cookie(request.headers(), token))
                .transpose()
            {
                Ok(renewal) => renewal,
                Err(error) => return super::response::Error::from(error).into_response(),
            };
            let theme = preferences.theme();
            let response = async move {
                if request.method() == http::Method::POST
                    && let Err(error) = crate::http::same_origin::validate(request.headers())
                {
                    return super::response::Error::from(error).into_response();
                }
                request.extensions_mut().insert(access);
                let started = std::time::Instant::now();
                crate::http::server_timing(next.run(request).await, started)
            };
            let response = crate::theme::THEME.scope(theme, response).await;
            crate::http::session::apply_renewal(jar, renewal, response)
        }
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::AuthRequired | ErrorKind::TokenInvalid
            ) =>
        {
            login_redirect(jar)
        }
        Err(error) => super::response::Error::from(error).into_response(),
    }
}

pub(super) fn login_redirect(jar: CookieJar) -> Response {
    let response = crate::http::session::clear(jar, Redirect::to("/login").into_response());
    super::response::decorate(response)
}

async fn get() -> super::response::Result<Response> {
    page(StatusCode::OK, "", &garde::Report::new())
}

async fn post(
    State(app): State<Arc<App>>,
    jar: CookieJar,
    headers: HeaderMap,
    super::command::Payload(payload): super::command::Payload<LoginPayload>,
) -> super::response::Result<Response> {
    let report = operation::validate::report(&payload);
    if !report.is_empty() {
        return page(StatusCode::UNPROCESSABLE_ENTITY, &payload.email, &report);
    }
    let session = crate::usecase::auth::login(
        &app.docker,
        &app.key,
        &payload.email,
        &payload.password_sha256,
        payload.totp_code.as_deref(),
        Timestamp::now(),
    )
    .await?;
    let cookie = crate::http::session::cookie(&headers, session)?;
    let response = (jar.add(cookie), Redirect::to("/")).into_response();
    Ok(super::response::decorate(response))
}

fn page(
    status: StatusCode,
    email: &str,
    fields: &garde::Report,
) -> super::response::Result<Response> {
    super::response::render(&LoginPage { email, fields }, status)
}
