use axum::response::{IntoResponse, Response};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use http::HeaderMap;
use http::header::SET_COOKIE;

use crate::error::{ErrorKind, Result};

pub(crate) fn cookie(headers: &HeaderMap, token: String) -> Result<Cookie<'static>> {
    let max_age = crate::config::SESSION_TTL
        .unsigned_abs()
        .try_into()
        .map_err(|_| ErrorKind::InvalidOperationOutput)?;
    Ok(Cookie::build((crate::config::SESSION_COOKIE, token))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(super::is_https(headers)?)
        .max_age(max_age)
        .build())
}

pub(crate) fn apply_renewal(
    jar: CookieJar,
    renewal: Option<Cookie<'static>>,
    response: Response,
) -> Response {
    let Some(cookie) = renewal else {
        return response;
    };
    if response.headers().contains_key(SET_COOKIE) {
        response
    } else {
        (jar.add(cookie), response).into_response()
    }
}

pub(crate) fn clear(jar: CookieJar, response: Response) -> Response {
    let cookie = Cookie::build(crate::config::SESSION_COOKIE).path("/");
    (jar.remove(cookie), response).into_response()
}
