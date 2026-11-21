use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::Response;
use http::StatusCode;
use jiff::Timestamp;

use crate::boot::App;
use crate::locale::tt;
use crate::operation;

#[derive(askama::Template)]
#[template(path = "node/volumes.html")]
struct Page {
    dashboard: super::super::dashboard::Dashboard,
    node: operation::node::typ::Node,
    report: Result<Option<operation::volume::typ::Report>, String>,
    can_write: bool,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateForm {
    name: String,
    changes: operation::volume::typ::UpdateVolumeRequest,
}

pub(super) fn routes() -> axum::Router<Arc<App>> {
    axum::Router::new()
        .route("/nodes/{id}/volumes", axum::routing::get(get))
        .route("/nodes/{id}/volumes/create", axum::routing::post(create))
        .route("/nodes/{id}/volumes/update", axum::routing::post(update))
        .route("/nodes/{id}/volumes/delete", axum::routing::post(delete))
        .route(
            "/nodes/{id}/volumes/delete-preview",
            axum::routing::post(delete_preview),
        )
}

async fn delete_preview(
    authorized: super::super::auth::Authorized<operation::volume::DeletePreview>,
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    super::super::command::Payload(form): super::super::command::Payload<operation::input::Name>,
) -> super::super::response::Result<Response> {
    let destination = format!("/nodes/{id}/volumes");
    authorized
        .execute(
            &app,
            operation::volume::typ::PreviewTarget {
                id,
                name: form.name,
            },
        )
        .await?;
    Ok(super::super::router::redirect(&destination))
}

async fn get(
    authorized: super::super::auth::Authorized<operation::volume::List>,
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> super::super::response::Result<Response> {
    let (dashboard, node, settings) = tokio::try_join!(
        super::super::dashboard::Dashboard::load(
            &app,
            authorized.access(),
            super::super::dashboard::Location::Nodes,
        ),
        operation::execute::<operation::node::Get>(
            &app,
            authorized.access(),
            operation::input::Id { id: id.clone() },
            Timestamp::now(),
        ),
        super::super::read::<operation::volume::Get>(&app, authorized.access()),
    )?;
    let (status, report) = if settings.image.is_some() {
        match authorized
            .execute(
                &app,
                operation::volume::ListInput {
                    id: id.clone(),
                    query: operation::volume::typ::Query { sizes: true },
                },
            )
            .await
        {
            Ok(report) => (StatusCode::OK, Ok(Some(report))),
            Err(error) => {
                let status = error.status();
                if matches!(
                    status,
                    StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
                ) {
                    return Err(error.into());
                }
                if status.is_server_error() {
                    tracing::error!(error = ?error, "volume query failed for node {id}");
                }
                (status, Err(error.detail(crate::locale::current()).0))
            }
        }
    } else {
        (StatusCode::OK, Ok(None))
    };
    super::super::response::render(
        &Page {
            dashboard,
            node,
            report,
            can_write: authorized
                .access()
                .allows(<operation::volume::Create as operation::Operation>::REQUIREMENT),
        },
        status,
    )
}

async fn create(
    authorized: super::super::auth::Authorized<operation::volume::Create>,
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    super::super::command::Payload(request): super::super::command::Payload<
        operation::volume::typ::CreateVolumeRequest,
    >,
) -> super::super::response::Result<Response> {
    let destination = format!("/nodes/{id}/volumes");
    authorized
        .execute(&app, operation::volume::CreateInput { id, request })
        .await?;
    Ok(super::super::router::redirect(&destination))
}

async fn update(
    authorized: super::super::auth::Authorized<operation::volume::Update>,
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    super::super::command::Payload(form): super::super::command::Payload<UpdateForm>,
) -> super::super::response::Result<Response> {
    let destination = format!("/nodes/{id}/volumes");
    authorized
        .execute(
            &app,
            operation::volume::UpdateInput {
                id,
                name: form.name,
                changes: form.changes,
            },
        )
        .await?;
    Ok(super::super::router::redirect(&destination))
}

async fn delete(
    authorized: super::super::auth::Authorized<operation::volume::Delete>,
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    super::super::command::Payload(form): super::super::command::Payload<operation::input::Name>,
) -> super::super::response::Result<Response> {
    let destination = format!("/nodes/{id}/volumes");
    authorized
        .execute(
            &app,
            operation::volume::typ::Target {
                id,
                name: form.name,
            },
        )
        .await?;
    Ok(super::super::router::redirect(&destination))
}
