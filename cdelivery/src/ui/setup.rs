use std::sync::Arc;

use axum::response::{IntoResponse, Response};
use axum_extra::extract::CookieJar;
use garde::Validate;
use http::{HeaderMap, StatusCode};

use crate::boot::Setup;
use crate::error::ErrorKind;
use crate::locale::tt;
use crate::{concept, operation, usecase};

type State = concept::setup::State;

#[derive(serde::Deserialize, Validate)]
#[serde(deny_unknown_fields)]
struct InitializeParams {
    #[garde(length(min = 1, max = 64))]
    first_name: String,
    #[garde(length(min = 1, max = 64))]
    last_name: String,
    #[garde(email, length(max = 254))]
    email: String,
    #[garde(custom(operation::validate::sha256_hex))]
    password_sha256: String,
}

#[derive(askama::Template)]
#[template(path = "setup.html")]
struct SetupPage {
    state: State,
}

pub(super) fn routes() -> axum::Router<Arc<Setup>> {
    axum::Router::new()
        .route("/setup", axum::routing::get(get).post(initialize))
        .route("/setup/resume", axum::routing::post(resume))
}

async fn get(
    axum::extract::State(setup): axum::extract::State<Arc<Setup>>,
) -> super::response::Result<Response> {
    let state = usecase::setup::state(&setup.docker).await?;
    super::response::render(&SetupPage { state }, StatusCode::OK)
}

async fn initialize(
    axum::extract::State(setup): axum::extract::State<Arc<Setup>>,
    jar: CookieJar,
    headers: HeaderMap,
    super::command::Payload(params): super::command::Payload<InitializeParams>,
) -> super::response::Result<Response> {
    let report = operation::validate::report(&params);
    if !report.is_empty() {
        return Err(ErrorKind::Validation(report).into());
    }
    let initialized = usecase::setup::initialize(
        &setup.docker,
        &setup.lock,
        params.first_name,
        params.last_name,
        &params.email,
        &params.password_sha256,
    )
    .await?;
    let response = if super::command::json() {
        let body = axum::Json(serde_json::json!({ "status": initialized.as_ref() }));
        (StatusCode::ACCEPTED, body).into_response()
    } else {
        super::command::complete("/setup")
    };
    if let concept::setup::Initialization::Initialized(session) = initialized {
        let cookie = crate::http::session::cookie(&headers, session.into_inner())?;
        return Ok((jar.add(cookie), response).into_response());
    }
    Ok(response)
}

async fn resume(
    axum::extract::State(setup): axum::extract::State<Arc<Setup>>,
    super::command::Payload(_payload): super::command::Payload<operation::input::Empty>,
) -> super::response::Result<Response> {
    usecase::setup::resume(&setup.docker, &setup.lock).await?;
    Ok(super::command::complete("/setup"))
}
