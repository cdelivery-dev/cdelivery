use std::sync::Arc;

use axum::{extract::State, response::Response};
use uuid::Uuid;

use crate::boot::App;
use crate::operation;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateForm {
    sql: String,
    size: Option<String>,
}

pub(super) fn routes() -> axum::Router<Arc<App>> {
    axum::Router::new()
        .route("/dependencies/{name}/changes", axum::routing::post(create))
        .route(
            "/changes/{id}/close",
            axum::routing::post(action::<operation::change::Close>),
        )
        .route(
            "/changes/{id}/apply",
            axum::routing::post(action::<operation::change::Apply>),
        )
        .route(
            "/changes/{id}/complete",
            axum::routing::post(action::<operation::change::Complete>),
        )
}

async fn create(
    State(app): State<Arc<App>>,
    authorized: super::auth::Authorized<operation::change::Create>,
    axum::extract::Path(name): axum::extract::Path<String>,
    super::command::Payload(form): super::command::Payload<CreateForm>,
) -> super::response::Result<Response> {
    authorized
        .execute(
            &app,
            operation::change::typ::CreateRequest {
                dependency: name.clone(),
                sql: form.sql,
                size: form.size,
            },
        )
        .await?;
    Ok(super::router::redirect(&format!("/dependencies/{name}")))
}

async fn action<O>(
    State(app): State<Arc<App>>,
    authorized: super::auth::Authorized<O>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    super::command::Payload(_): super::command::Payload<operation::input::Empty>,
) -> super::response::Result<Response>
where
    O: operation::catalog::Catalogued<Input = operation::change::Id>,
{
    let change = operation::execute::<operation::change::Get>(
        &app,
        authorized.access(),
        operation::change::Id { id },
        jiff::Timestamp::now(),
    )
    .await?;
    authorized
        .execute(&app, operation::change::Id { id })
        .await?;
    let location = match change.target {
        operation::change::typ::Target::App { name, .. } => format!("/apps/{name}"),
        operation::change::typ::Target::PostgreSQL { dependency, .. } => {
            format!("/dependencies/{}", dependency.name)
        }
    };
    Ok(super::router::redirect(&location))
}
