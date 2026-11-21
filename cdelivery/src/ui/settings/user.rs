use std::sync::Arc;

use askama::Template;
use axum::extract::{Path, State};
use axum::response::Response;
use http::StatusCode;
use uuid::Uuid;

use crate::boot::App;
use crate::locale::tt;
use crate::operation;

#[derive(Template)]
#[template(path = "user/list.html")]
struct ListPage {
    account: super::super::account::Account,
    users: Vec<operation::user::typ::ManagedUser>,
    current_user_id: String,
}

#[derive(Template)]
#[template(path = "user/create.html")]
struct CreatePage {
    account: super::super::account::Account,
}

#[derive(Template)]
#[template(path = "user/edit.html")]
struct EditPage {
    account: super::super::account::Account,
    user: operation::user::typ::ManagedUser,
    current_user_id: String,
}

pub(super) fn routes() -> axum::Router<Arc<App>> {
    axum::Router::new()
        .route("/settings/users", axum::routing::get(list).post(create))
        .route("/settings/users/.create", axum::routing::get(create_form))
        .route(
            "/settings/users/{id}",
            axum::routing::get(edit).post(update),
        )
        .route("/settings/users/{id}/delete", axum::routing::post(delete))
}

async fn list(
    authorized: super::super::auth::Authorized<operation::user::List>,
    State(app): State<Arc<App>>,
) -> super::super::response::Result<Response> {
    let current_user_id = authorized.access().user_id()?;
    let users = authorized
        .execute(&app, operation::input::Empty {})
        .await?
        .users;
    super::super::response::render(
        &ListPage {
            account: super::super::account::Account::new(authorized.access()),
            users,
            current_user_id: current_user_id.to_string(),
        },
        StatusCode::OK,
    )
}

async fn create_form(
    authorized: super::super::auth::Authorized<operation::user::Create>,
) -> super::super::response::Result<Response> {
    super::super::response::render(
        &CreatePage {
            account: super::super::account::Account::new(authorized.access()),
        },
        StatusCode::OK,
    )
}

async fn create(
    authorized: super::super::auth::Authorized<operation::user::Create>,
    State(app): State<Arc<App>>,
    super::super::command::Payload(input): super::super::command::Payload<
        operation::user::typ::CreateUserRequest,
    >,
) -> super::super::response::Result<Response> {
    authorized.execute(&app, input).await?;
    Ok(super::super::router::redirect("/settings/users"))
}

async fn edit(
    authorized: super::super::auth::Authorized<operation::user::Get>,
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
) -> super::super::response::Result<Response> {
    let current_user_id = authorized.access().user_id()?;
    let user = authorized
        .execute(&app, operation::input::UserId { id })
        .await?;
    super::super::response::render(
        &EditPage {
            account: super::super::account::Account::new(authorized.access()),
            user,
            current_user_id: current_user_id.to_string(),
        },
        StatusCode::OK,
    )
}

async fn update(
    authorized: super::super::auth::Authorized<operation::user::Update>,
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
    super::super::command::Payload(changes): super::super::command::Payload<
        operation::user::typ::UpdateUserRequest,
    >,
) -> super::super::response::Result<Response> {
    authorized
        .execute(&app, operation::user::UpdateUserInput { id, changes })
        .await?;
    Ok(super::super::router::redirect(&format!(
        "/settings/users/{id}"
    )))
}

async fn delete(
    authorized: super::super::auth::Authorized<operation::user::Delete>,
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
    super::super::command::Payload(_): super::super::command::Payload<operation::input::Empty>,
) -> super::super::response::Result<Response> {
    authorized
        .execute(&app, operation::input::UserId { id })
        .await?;
    Ok(super::super::router::redirect("/settings/users"))
}
