use std::net::Ipv4Addr;
use std::sync::Arc;

use axum::Json;
use axum::extract::{DefaultBodyLimit, FromRequest as _, Request, State};
use axum::middleware::{Next, from_fn, from_fn_with_state};
use axum::response::Response;
use jiff::Timestamp;
use serde::Deserialize;
use tower_http::catch_panic::CatchPanicLayer;

use crate::boot::App;
use crate::error::{Error, ErrorKind, Result};
use crate::{rest, usecase};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Redeem {
    advertise_addr: Ipv4Addr,
}

pub(super) fn routes() -> axum::Router<Arc<App>> {
    axum::Router::new()
        .route("/join/redeem", axum::routing::post(redeem))
        .layer(DefaultBodyLimit::max(crate::config::PUBLIC_BODY_LIMIT))
        .layer(from_fn_with_state(
            crate::config::PROTECTED_TRANSPORT_TIMEOUT,
            super::middleware::with_timeout::<Error>,
        ))
        .layer(CatchPanicLayer::custom(
            super::middleware::panic_response::<Error>,
        ))
        .layer(from_fn(no_store))
}

async fn no_store(request: Request, next: Next) -> Response {
    super::middleware::no_store(next.run(request).await)
}

async fn redeem(State(app): State<Arc<App>>, request: Request) -> Result<Json<serde_json::Value>> {
    let token =
        super::middleware::bearer_token(request.headers()).ok_or(ErrorKind::TokenInvalid)?;
    let info =
        usecase::node::redeem_join_invitation(&app.docker, &app.key, token, Timestamp::now())
            .await?;
    let rest::extract::Body(input) =
        rest::extract::Body::<Redeem>::from_request(request, &app).await?;
    let address = input.advertise_addr;
    if address.octets()[0] == 0
        || address.octets()[0] >= 224
        || address.is_loopback()
        || address.is_link_local()
    {
        return Err(ErrorKind::InvalidBody(
            "advertise_addr must be a non-loopback unicast IPv4 address".to_owned(),
        )
        .into());
    }
    Ok(Json(serde_json::json!({
        "ListenAddr": "0.0.0.0:2377",
        "AdvertiseAddr": address,
        "RemoteAddrs": [info.manager_addr()],
        "JoinToken": info.token().reveal(),
        "Availability": "active",
    })))
}
