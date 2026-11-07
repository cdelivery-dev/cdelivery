use http::HeaderMap;
use http::header::{HOST, HeaderName, HeaderValue, ORIGIN};

use crate::error::{ErrorKind, Result};

pub(crate) fn validate(headers: &HeaderMap) -> Result<()> {
    let actual = one(headers, &ORIGIN)
        .and_then(|value| value.to_str().ok())
        .and_then(strict_origin)
        .ok_or(ErrorKind::Forbidden)?;
    let host = one(headers, &HOST)
        .and_then(|value| value.to_str().ok())
        .filter(|host| !host.contains('@'))
        .ok_or(ErrorKind::Forbidden)?;
    let authority = host
        .parse::<http::uri::Authority>()
        .map_err(|_| ErrorKind::Forbidden)?;
    let scheme = if super::is_https(headers)? {
        "https"
    } else {
        "http"
    };
    let expected = url::Url::parse(&format!("{scheme}://{authority}"))
        .map_err(|_| ErrorKind::Forbidden)?
        .origin();
    if actual != expected {
        return Err(ErrorKind::Forbidden.into());
    }
    Ok(())
}

fn one<'a>(headers: &'a HeaderMap, name: &HeaderName) -> Option<&'a HeaderValue> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?;
    values.next().is_none().then_some(value)
}

fn strict_origin(raw: &str) -> Option<url::Origin> {
    let url = url::Url::parse(raw).ok()?;
    let origin = url.origin();
    (matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some_and(|host| !host.is_empty())
        && url.username().is_empty()
        && url.password().is_none()
        && raw == origin.ascii_serialization())
    .then_some(origin)
}
