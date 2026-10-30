use std::sync::Arc;

use askama::Template;
use axum::extract::State;
use axum::response::Response;
use http::StatusCode;

use crate::locale::tt;
use crate::{concept, operation};

#[derive(Template)]
#[template(path = "insight/index.html")]
struct Page {
    dashboard: super::dashboard::Dashboard,
    insight: operation::insight::typ::Insight,
    form: Option<FormData>,
}

struct FormData {
    nodes: operation::node::typ::Nodes,
    server_target_version: Option<String>,
    agent_target_version: Option<String>,
}

pub(super) fn routes() -> axum::Router<Arc<crate::boot::App>> {
    axum::Router::new()
        .route("/insight", axum::routing::get(get))
        .route("/insight/configure", axum::routing::post(configure))
        .route("/insight/disable", axum::routing::post(disable))
}

pub(super) async fn optional_query<T>(
    query: impl Future<Output = crate::error::Result<T>>,
) -> crate::error::Result<Option<T>> {
    match query.await {
        Ok(output) => Ok(Some(output)),
        Err(error) if matches!(error.kind(), crate::error::ErrorKind::InsightNotEnabled) => {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

async fn get(
    authorized: super::auth::Authorized<operation::insight::Overview>,
    State(app): State<Arc<crate::boot::App>>,
) -> super::response::Result<Response> {
    let can_edit = authorized
        .access()
        .allows(concept::permission::Requirement::Admin);
    let (dashboard, insight, [server_latest, agent_latest], nodes) = tokio::join!(
        super::dashboard::Dashboard::load(
            &app,
            authorized.access(),
            super::dashboard::Location::Insight,
        ),
        authorized.execute(&app, operation::input::Empty {}),
        super::latest_versions(
            &app,
            authorized.access(),
            can_edit,
            [
                concept::release::Component::Insight,
                concept::release::Component::InsightAgent,
            ],
        ),
        super::load_nodes(&app, authorized.access(), can_edit),
    );
    let insight = insight?;
    let form = if can_edit {
        Some(FormData {
            nodes: nodes?,
            server_target_version: super::target_version(
                insight
                    .server
                    .as_ref()
                    .map(|server| server.version.as_str()),
                server_latest,
            )?,
            agent_target_version: super::target_version(
                insight.agent.as_ref().map(|agent| agent.version.as_str()),
                agent_latest,
            )?,
        })
    } else {
        None
    };
    super::response::render(
        &Page {
            dashboard: dashboard?,
            insight,
            form,
        },
        StatusCode::OK,
    )
}

async fn configure(
    authorized: super::auth::Authorized<operation::insight::Enable>,
    State(app): State<Arc<crate::boot::App>>,
    super::command::Payload(payload): super::command::Payload<
        operation::insight::typ::EnableInsightRequest,
    >,
) -> super::response::Result<Response> {
    authorized.execute(&app, payload).await?;
    Ok(super::router::redirect("/insight"))
}

async fn disable(
    authorized: super::auth::Authorized<operation::insight::Disable>,
    State(app): State<Arc<crate::boot::App>>,
    super::command::Payload(_): super::command::Payload<operation::input::Empty>,
) -> super::response::Result<Response> {
    authorized.execute(&app, operation::input::Empty {}).await?;
    Ok(super::router::redirect("/insight"))
}
