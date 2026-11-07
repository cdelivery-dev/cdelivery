use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::boot::App;
use crate::error::Result;
use crate::usecase;

pub(crate) fn router(state: Arc<App>) -> axum::Router {
    axum::Router::new()
        .route(pingress_wire::CONFIG_PATH, axum::routing::get(config))
        .route(
            &format!("{}{{token}}", pingress_wire::CHALLENGE_PATH_PREFIX),
            axum::routing::get(challenge),
        )
        .with_state(state)
}

async fn challenge(State(state): State<Arc<App>>, Path(token): Path<String>) -> Response {
    match state
        .challenges
        .read_sync(&token, |_, key_authorization| key_authorization.clone())
    {
        Some(key_authorization) => (StatusCode::OK, key_authorization).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct Params {
    etag: Option<String>,
    nonce: Option<String>,
}

async fn config(State(state): State<Arc<App>>, Query(params): Query<Params>) -> Response {
    let Some(nonce) = params.nonce.filter(|nonce| !nonce.is_empty()) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    match respond(&state, params.etag.as_deref(), nonce).await {
        Ok(Some(body)) => (StatusCode::OK, body).into_response(),
        Ok(None) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => {
            tracing::error!(error = ?error, "pingress config request failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn respond(state: &App, etag: Option<&str>, nonce: String) -> Result<Option<String>> {
    let snapshot = usecase::pingress::snapshot(&state.docker, &state.key).await?;
    if etag == Some(snapshot.etag.as_str()) {
        return Ok(None);
    }
    let envelope = pingress_wire::Envelope { nonce, snapshot };
    Ok(Some(
        state
            .key
            .cipher_for(cipher::Domain::Pingress)
            .seal(&envelope)?,
    ))
}
