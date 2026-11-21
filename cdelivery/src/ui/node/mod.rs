mod metrics;
mod volume;

use std::collections::HashSet;
use std::sync::Arc;

use askama::Template;
use axum::extract::State;
use axum::response::Response;
use http::StatusCode;
use jiff::Timestamp;

use crate::boot::App;
use crate::locale::tt;
use crate::{concept, operation};

#[derive(Template)]
#[template(path = "node/list.html")]
struct ListPage {
    dashboard: super::dashboard::Dashboard,
    nodes: Vec<operation::node::typ::Node>,
    can_relabel: bool,
    join: Option<(
        operation::node::typ::JoinInvitation,
        operation::node::typ::JoinInvitation,
    )>,
    silent_agents: HashSet<String>,
    current_version: Option<String>,
    target_version: Option<String>,
    latest_unavailable: bool,
    zfs_enabled: bool,
}

impl ListPage {
    #[expect(clippy::float_cmp)]
    fn single_core(node: &operation::node::typ::Node) -> bool {
        node.cpus == 1.0
    }

    fn has_warning(&self, node: &operation::node::typ::Node) -> bool {
        !matches!(
            node.availability,
            operation::node::typ::NodeAvailability::Active
        ) || matches!(node.role, operation::node::typ::NodeRole::Manager)
            && !matches!(
                node.reachability,
                Some(operation::node::typ::NodeReachability::Reachable)
            )
            || self.silent_agents.contains(&node.id)
    }
}

#[derive(Template)]
#[template(path = "node/detail.html")]
struct DetailPage {
    dashboard: super::dashboard::Dashboard,
    node: operation::node::typ::Node,
    silent_agent: Option<operation::node::typ::SilentAgent>,
    can_relabel: bool,
    can_delete: bool,
    labels_max: usize,
    name_max: usize,
    reserved_name: &'static str,
}

pub(super) fn routes() -> axum::Router<Arc<App>> {
    axum::Router::new()
        .route("/nodes", axum::routing::get(list))
        .route("/nodes/{id}", axum::routing::get(get).post(relabel))
        .route("/nodes/{id}/delete", axum::routing::post(delete))
        .merge(metrics::routes())
        .merge(volume::routes())
}

async fn list(
    authorized: super::auth::Authorized<operation::node::List>,
    State(app): State<Arc<App>>,
) -> super::response::Result<Response> {
    let can_relabel = authorized
        .access()
        .allows(<operation::node::Relabel as operation::Operation>::REQUIREMENT);
    let can_join = authorized
        .access()
        .allows(<operation::node::CreateJoinInvitation as operation::Operation>::REQUIREMENT);
    let can_manage = authorized
        .access()
        .allows(<operation::volume::Enable as operation::Operation>::REQUIREMENT);
    let (output, dashboard) = tokio::try_join!(
        authorized.execute(&app, operation::input::Empty {}),
        super::dashboard::Dashboard::load(
            &app,
            authorized.access(),
            super::dashboard::Location::Nodes,
        ),
    )?;
    let (current, [latest]) = tokio::join!(
        super::read::<operation::volume::Get>(&app, authorized.access()),
        super::latest_versions(
            &app,
            authorized.access(),
            can_manage,
            [concept::release::Component::CdeliveryAgent],
        ),
    );
    let current = current?;
    let zfs_enabled = current.zfs_enabled.unwrap_or(true);
    let current_version = current.image.as_ref().map(|image| image.tag().clone());
    let latest_unavailable = latest.is_none();
    let target_version = super::target_version(current_version.as_deref(), latest)?;
    let join = if can_join {
        let now = Timestamp::now();
        let issue = |role| {
            operation::execute::<operation::node::CreateJoinInvitation>(
                &app,
                authorized.access(),
                operation::node::typ::CreateJoinInvitationRequest { role },
                now,
            )
        };
        Some(tokio::try_join!(
            issue(operation::node::typ::NodeRole::Worker),
            issue(operation::node::typ::NodeRole::Manager),
        )?)
    } else {
        None
    };
    let silent_agents = silent_agents(&app, authorized.access())
        .await
        .into_iter()
        .map(|agent| agent.node_id)
        .collect();
    super::response::render(
        &ListPage {
            dashboard,
            nodes: output.nodes,
            can_relabel,
            join,
            silent_agents,
            current_version,
            target_version,
            latest_unavailable,
            zfs_enabled,
        },
        StatusCode::OK,
    )
}

async fn silent_agents(
    app: &App,
    access: &concept::permission::Access,
) -> Vec<operation::node::typ::SilentAgent> {
    super::read::<operation::node::SilentAgents>(app, access)
        .await
        .map_or_else(
            |error| {
                tracing::warn!(error = ?error, "silent agent check failed for node page");
                Vec::new()
            },
            |output| output.silent_agents,
        )
}

async fn get(
    authorized: super::auth::Authorized<operation::node::Get>,
    State(app): State<Arc<App>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> super::response::Result<Response> {
    let can_relabel = authorized
        .access()
        .allows(<operation::node::Relabel as operation::Operation>::REQUIREMENT);
    let can_delete = authorized
        .access()
        .allows(<operation::node::Delete as operation::Operation>::REQUIREMENT);
    let node = authorized
        .execute(&app, operation::input::Id { id })
        .await?;
    let dashboard = super::dashboard::Dashboard::load(
        &app,
        authorized.access(),
        super::dashboard::Location::Nodes,
    )
    .await?;
    let silent_agent = silent_agents(&app, authorized.access())
        .await
        .into_iter()
        .find(|agent| agent.node_id == node.id);
    super::response::render(
        &DetailPage {
            dashboard,
            node,
            silent_agent,
            can_relabel,
            can_delete,
            labels_max: operation::node::typ::LABELS_MAX,
            name_max: concept::name::MAX,
            reserved_name: crate::config::SERVICE_NAME,
        },
        StatusCode::OK,
    )
}

async fn relabel(
    authorized: super::auth::Authorized<operation::node::Relabel>,
    State(app): State<Arc<App>>,
    axum::extract::Path(id): axum::extract::Path<String>,
    super::command::Payload(changes): super::command::Payload<
        operation::node::typ::UpdateNodeRequest,
    >,
) -> super::response::Result<Response> {
    authorized
        .execute(
            &app,
            operation::node::RelabelInput {
                id: id.clone(),
                changes,
            },
        )
        .await?;
    Ok(super::router::redirect(&format!("/nodes/{id}")))
}

async fn delete(
    authorized: super::auth::Authorized<operation::node::Delete>,
    State(app): State<Arc<App>>,
    axum::extract::Path(id): axum::extract::Path<String>,
    super::command::Payload(_): super::command::Payload<operation::input::Empty>,
) -> super::response::Result<Response> {
    authorized
        .execute(&app, operation::input::Id { id })
        .await?;
    Ok(super::router::redirect("/nodes"))
}
