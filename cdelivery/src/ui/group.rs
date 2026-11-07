use std::sync::Arc;

use askama::Template;
use axum::extract::State;
use axum::response::Response;
use http::StatusCode;

use crate::boot::App;
use crate::concept;
use crate::locale::tt;
use crate::operation;

#[derive(Template)]
#[template(path = "group/list.html")]
struct ListPage {
    dashboard: super::dashboard::Dashboard,
}

#[derive(Template)]
#[template(path = "group/create.html")]
struct CreatePage {
    dashboard: super::dashboard::Dashboard,
}

#[derive(Template)]
#[template(path = "group/info.html")]
struct InfoPage {
    dashboard: super::dashboard::Dashboard,
    name: String,
    description: String,
    can_write: bool,
}

pub(super) fn routes() -> axum::Router<Arc<App>> {
    axum::Router::new()
        .route("/groups", axum::routing::get(list).post(create))
        .route("/groups/.create", axum::routing::get(create_form))
        .route("/groups/{name}", axum::routing::get(get).post(update))
        .route("/groups/{name}/delete", axum::routing::post(delete))
}

async fn list(
    State(app): State<Arc<App>>,
    authorized: super::auth::Authorized<operation::group::List>,
) -> super::response::Result<Response> {
    let dashboard = super::dashboard::Dashboard::load(
        &app,
        authorized.access(),
        super::dashboard::Location::Groups,
    )
    .await?;
    super::response::render(&ListPage { dashboard }, StatusCode::OK)
}

async fn create_form(
    State(app): State<Arc<App>>,
    authorized: super::auth::Authorized<operation::group::Create>,
) -> super::response::Result<Response> {
    let dashboard = super::dashboard::Dashboard::load(
        &app,
        authorized.access(),
        super::dashboard::Location::Groups,
    )
    .await?;
    super::response::render(&CreatePage { dashboard }, StatusCode::OK)
}

async fn create(
    State(app): State<Arc<App>>,
    authorized: super::auth::Authorized<operation::group::Create>,
    super::command::Payload(payload): super::command::Payload<
        operation::group::typ::CreateGroupRequest,
    >,
) -> super::response::Result<Response> {
    authorized.execute(&app, payload).await?;
    Ok(super::router::redirect("/groups"))
}

async fn get(
    authorized: super::auth::Authorized<operation::group::Get>,
    State(app): State<Arc<App>>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> super::response::Result<Response> {
    let can_write = authorized
        .access()
        .allows(concept::permission::Requirement::Admin);
    let group = authorized
        .execute(&app, operation::input::Name { name })
        .await?;
    let dashboard = super::dashboard::Dashboard::load(
        &app,
        authorized.access(),
        super::dashboard::Location::Group(group.name.clone()),
    )
    .await?;
    super::response::render(
        &InfoPage {
            dashboard,
            name: group.name,
            description: group.description.unwrap_or_default(),
            can_write,
        },
        StatusCode::OK,
    )
}

async fn update(
    authorized: super::auth::Authorized<operation::group::Update>,
    State(app): State<Arc<App>>,
    axum::extract::Path(name): axum::extract::Path<String>,
    super::command::Payload(changes): super::command::Payload<
        operation::group::typ::UpdateGroupRequest,
    >,
) -> super::response::Result<Response> {
    authorized
        .execute(&app, operation::group::UpdateInput { name, changes })
        .await?;
    Ok(super::router::redirect("/groups"))
}

async fn delete(
    authorized: super::auth::Authorized<operation::group::Delete>,
    State(app): State<Arc<App>>,
    axum::extract::Path(name): axum::extract::Path<String>,
    super::command::Payload(_): super::command::Payload<operation::input::Empty>,
) -> super::response::Result<Response> {
    authorized
        .execute(&app, operation::input::Name { name })
        .await?;
    Ok(super::router::redirect("/groups"))
}
