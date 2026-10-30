use std::sync::Arc;

use axum::extract::State;
use axum::response::Response;
use http::StatusCode;

use crate::boot::App;
use crate::concept;
use crate::locale::tt;
use crate::operation;

#[derive(askama::Template)]
#[template(path = "account/apikeys.html")]
struct Page {
    account: super::Account,
    keys: Vec<operation::apikey::typ::ApiKey>,
    role: concept::user::Role,
}

#[derive(askama::Template)]
#[template(path = "account/apikey_created.html")]
struct CreatedPage {
    account: super::Account,
    issued: operation::apikey::typ::IssuedApiKey,
}

pub(super) fn routes() -> axum::Router<Arc<App>> {
    axum::Router::new()
        .route("/account/apikeys", axum::routing::get(get).post(create))
        .route("/account/apikeys/{id}/revoke", axum::routing::post(revoke))
}

async fn get(
    authorized: super::super::auth::Authorized<operation::apikey::List>,
    State(app): State<Arc<App>>,
) -> super::super::response::Result<Response> {
    let account = super::Account::new(authorized.access());
    let role = authorized.access().role();
    let keys = authorized
        .execute(&app, operation::input::Empty {})
        .await?
        .keys;
    super::super::response::render(
        &Page {
            account,
            keys,
            role,
        },
        StatusCode::OK,
    )
}

async fn create(
    authorized: super::super::auth::Authorized<operation::apikey::Create>,
    State(app): State<Arc<App>>,
    super::super::command::Payload(input): super::super::command::Payload<
        operation::apikey::typ::CreateApiKeyRequest,
    >,
) -> super::super::response::Result<Response> {
    let account = super::Account::new(authorized.access());
    let issued = authorized.execute(&app, input).await?;
    super::super::response::render(&CreatedPage { account, issued }, StatusCode::CREATED)
}

async fn revoke(
    authorized: super::super::auth::Authorized<operation::apikey::Revoke>,
    State(app): State<Arc<App>>,
    axum::extract::Path(id): axum::extract::Path<uuid::Uuid>,
    super::super::command::Payload(_): super::super::command::Payload<operation::input::Empty>,
) -> super::super::response::Result<Response> {
    authorized
        .execute(&app, operation::input::ApiKeyId { id })
        .await?;
    Ok(super::super::router::redirect("/account/apikeys"))
}
