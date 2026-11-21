use std::sync::LazyLock;

use axum::body::Body;
use axum::extract::OriginalUri;
use axum::response::{IntoResponse, Response};
use http::header::{CACHE_CONTROL, CONTENT_TYPE, HeaderValue, X_CONTENT_TYPE_OPTIONS};
use sha2::{Digest, Sha256};

use crate::error::ErrorKind;

macro_rules! embedded_asset {
    ($path:ident, $bytes:ident, $file:literal) => {
        pub(super) const $path: &str = concat!("/assets/", $file);
        const $bytes: &[u8] = include_bytes!(concat!("../../assets/", $file));
    };
    ($path:ident, $bytes:ident, $stem:literal, $ext:literal) => {
        const $bytes: &[u8] = include_bytes!(concat!("../../assets/", $stem, ".", $ext));
        pub(super) static $path: LazyLock<String> = LazyLock::new(|| {
            let hash = digest_prefix($bytes);
            format!("/assets/{}.{hash}.{}", $stem, $ext)
        });
    };
}

embedded_asset!(LOGO_PATH, LOGO, "cdelivery", "svg");
embedded_asset!(FAVICON_LIGHT_PATH, FAVICON_LIGHT, "icon", "svg");
embedded_asset!(FAVICON_DARK_PATH, FAVICON_DARK, "favicon-dark", "svg");
embedded_asset!(STYLE_PATH, STYLE, "cdelivery", "css");
embedded_asset!(SHARED_PATH, SHARED, "cdelivery", "js");
embedded_asset!(ALPINE_PATH, ALPINE, "alpine@3.15.12.min.js");
embedded_asset!(DYGRAPH_PATH, DYGRAPH, "dygraph@2.2.2.min.js");
embedded_asset!(DYGRAPH_STYLE_PATH, DYGRAPH_STYLE, "dygraph@2.2.2.min.css");
embedded_asset!(
    DYGRAPH_SMOOTH_PATH,
    DYGRAPH_SMOOTH,
    "dygraph-smooth-plotter@2.2.2.min.js"
);
embedded_asset!(
    RELATIVE_TIME_PATH,
    RELATIVE_TIME,
    "relative-time-element@5.3.1.min.js"
);
embedded_asset!(SHA256_PATH, SHA256, "sha256@1.0.0.min.js");
embedded_asset!(LICENSES_PATH, LICENSES, "THIRD_PARTY_LICENSES.txt");

pub(super) fn routes<S>() -> axum::Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    axum::Router::new().route("/assets/{*path}", axum::routing::get(get))
}

async fn get(OriginalUri(uri): OriginalUri) -> super::response::Result<Response> {
    let immutable = "public, max-age=31536000, immutable";
    let (bytes, content_type, cache) = match uri.path() {
        "/assets/LICENSE.txt" => (
            include_bytes!("../../../LICENSE").as_slice(),
            "text/plain; charset=utf-8",
            "no-cache",
        ),
        LICENSES_PATH => (LICENSES, "text/plain; charset=utf-8", "no-cache"),
        path if path == LOGO_PATH.as_str() => (LOGO, "image/svg+xml", immutable),
        path if path == FAVICON_LIGHT_PATH.as_str() => (FAVICON_LIGHT, "image/svg+xml", immutable),
        path if path == FAVICON_DARK_PATH.as_str() => (FAVICON_DARK, "image/svg+xml", immutable),
        path if path == STYLE_PATH.as_str() => (STYLE, "text/css; charset=utf-8", immutable),
        path if path == SHARED_PATH.as_str() => {
            (SHARED, "text/javascript; charset=utf-8", immutable)
        }
        DYGRAPH_STYLE_PATH => (DYGRAPH_STYLE, "text/css; charset=utf-8", immutable),
        ALPINE_PATH => (ALPINE, "text/javascript; charset=utf-8", immutable),
        DYGRAPH_PATH => (DYGRAPH, "text/javascript; charset=utf-8", immutable),
        DYGRAPH_SMOOTH_PATH => (DYGRAPH_SMOOTH, "text/javascript; charset=utf-8", immutable),
        RELATIVE_TIME_PATH => (RELATIVE_TIME, "text/javascript; charset=utf-8", immutable),
        SHA256_PATH => (SHA256, "text/javascript; charset=utf-8", immutable),
        _ => return Err(ErrorKind::RouteNotFound.into()),
    };
    Ok(asset(bytes, content_type, cache))
}

fn digest_prefix(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .take(13)
        .map(|nibble| {
            char::from(if nibble < 10 {
                b'0' + nibble
            } else {
                b'a' + nibble - 10
            })
        })
        .collect()
}

fn asset(bytes: &'static [u8], content_type: &'static str, cache: &'static str) -> Response {
    let mut response = Body::from(bytes).into_response();
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static(cache));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    response
}
