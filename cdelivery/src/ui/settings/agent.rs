use std::sync::Arc;

use axum::extract::State;
use axum::response::Response;
use http::StatusCode;

use crate::boot::App;
use crate::locale::tt;
use crate::operation;

#[derive(askama::Template)]
#[template(path = "settings/agent/model.html")]
struct ModelPage {
    account: super::super::account::Account,
    agent: Option<operation::settings::typ::AgentSettings>,
}

#[derive(askama::Template)]
#[template(path = "settings/agent/duty.html")]
struct DutyPage {
    account: super::super::account::Account,
    agent: Option<operation::settings::typ::AgentSettings>,
}

pub(super) fn routes() -> axum::Router<Arc<App>> {
    axum::Router::new()
        .route(
            "/settings/agent/model",
            axum::routing::get(model).post(super::set::<operation::settings::SetAgent>),
        )
        .route(
            "/settings/agent/model/clear",
            axum::routing::post(super::clear::<operation::settings::ClearAgent>),
        )
        .route(
            "/settings/agent/duty",
            axum::routing::get(duty).post(super::set::<operation::settings::SetAgent>),
        )
}

async fn model(
    authorized: super::super::auth::Authorized<operation::settings::SetAgent>,
    State(app): State<Arc<App>>,
) -> super::super::response::Result<Response> {
    let settings =
        super::super::read::<operation::settings::GetAgent>(&app, authorized.access()).await?;
    super::super::response::render(
        &ModelPage {
            account: super::super::account::Account::new(authorized.access()),
            agent: settings.agent,
        },
        StatusCode::OK,
    )
}

async fn duty(
    authorized: super::super::auth::Authorized<operation::settings::SetAgent>,
    State(app): State<Arc<App>>,
) -> super::super::response::Result<Response> {
    let settings =
        super::super::read::<operation::settings::GetAgent>(&app, authorized.access()).await?;
    super::super::response::render(
        &DutyPage {
            account: super::super::account::Account::new(authorized.access()),
            agent: settings.agent,
        },
        StatusCode::OK,
    )
}
