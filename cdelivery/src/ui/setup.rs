use std::sync::Arc;

use axum::response::{IntoResponse, Response};
use garde::Validate;
use http::StatusCode;

use crate::boot::Setup;
use crate::error::ErrorKind;
use crate::locale::tt;
use crate::{concept, operation, usecase};

type State = concept::setup::State;

tokio::task_local! {
    pub(super) static JSON: bool;
}

pub(super) fn json() -> bool {
    JSON.try_with(|json| *json).unwrap_or(false)
}

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
struct SetupPage<'a> {
    state: State,
    first_name: &'a str,
    last_name: &'a str,
    email: &'a str,
    fields: &'a garde::Report,
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
    super::response::render(
        &SetupPage {
            state,
            first_name: "",
            last_name: "",
            email: "",
            fields: &garde::Report::new(),
        },
        StatusCode::OK,
    )
}

async fn initialize(
    axum::extract::State(setup): axum::extract::State<Arc<Setup>>,
    super::command::Payload(params): super::command::Payload<InitializeParams>,
) -> super::response::Result<Response> {
    let report = operation::validate::report(&params);
    if !report.is_empty() {
        if json() {
            return Err(ErrorKind::Validation(report).into());
        }
        let state = usecase::setup::state(&setup.docker).await?;
        return super::response::render(
            &SetupPage {
                state,
                first_name: &params.first_name,
                last_name: &params.last_name,
                email: &params.email,
                fields: &report,
            },
            StatusCode::UNPROCESSABLE_ENTITY,
        );
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
    if json() {
        let body = axum::Json(serde_json::json!({ "status": initialized.as_ref() }));
        return Ok((StatusCode::ACCEPTED, body).into_response());
    }
    Ok(super::router::redirect("/setup"))
}

async fn resume(
    axum::extract::State(setup): axum::extract::State<Arc<Setup>>,
    super::command::Payload(_payload): super::command::Payload<operation::input::Empty>,
) -> super::response::Result<Response> {
    usecase::setup::resume(&setup.docker, &setup.lock).await?;
    Ok(super::router::redirect("/setup"))
}
