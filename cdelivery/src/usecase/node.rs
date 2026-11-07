use std::collections::HashMap;

use insight_wire::event::Kind;
use jiff::{SignedDuration, Timestamp};

use crate::concept;
use crate::config;
use crate::error::{ErrorKind, Result};
use crate::infra;

pub(crate) async fn list(docker: &infra::docker::Docker) -> Result<Vec<concept::node::Node>> {
    infra::node::observe_all(docker).await
}

pub(crate) async fn get(docker: &infra::docker::Docker, id: &str) -> Result<concept::node::Node> {
    super::require(infra::node::observe(docker, id), ErrorKind::NodeNotFound).await
}

pub(crate) async fn relabel(
    docker: &infra::docker::Docker,
    id: &str,
    labels: &[concept::name::Name],
) -> Result<concept::node::Node> {
    let node = infra::node::relabel(docker, id, labels)
        .await?
        .ok_or(ErrorKind::NodeNotFound)?;
    infra::event::emit(Kind::NodeRelabeled, id, node.hostname());
    Ok(node)
}

pub(crate) async fn delete(docker: &infra::docker::Docker, id: &str) -> Result<()> {
    infra::node::delete(docker, id).await?;
    infra::event::emit(Kind::NodeRemoved, id, "");
    Ok(())
}

pub(crate) async fn create_join_invitation(
    docker: &infra::docker::Docker,
    access: &concept::permission::Access,
    role: concept::node::Role,
    now: Timestamp,
) -> Result<(String, Timestamp, std::net::SocketAddr)> {
    let manager_addr =
        super::require(infra::node::manager_addr(docker), ErrorKind::NoSwarmManager).await?;
    let expires_at = config::expiry(now, config::NODE_JOIN_TTL);
    let invitation = concept::node::join::Invitation::new(role, expires_at, access.actor().clone());
    Ok((
        infra::node::join::issue(&invitation, now).await?,
        expires_at,
        manager_addr,
    ))
}

pub(crate) async fn redeem_join_invitation(
    docker: &infra::docker::Docker,
    key: &cipher::MasterKey,
    token: &str,
    now: Timestamp,
) -> Result<concept::node::JoinInfo> {
    let invitation = infra::node::join::verify(token, now).await?;
    let role = match invitation.issuer().subject() {
        concept::actor::Subject::User { id, .. } => {
            let user =
                super::found(infra::user::load(docker, key, id), ErrorKind::TokenInvalid).await?;
            invitation.user_role(&user, now)?
        }
        concept::actor::Subject::Duty => {
            let settings = infra::setting::load(docker, key).await?.into_inner();
            let agent = settings.duty_agent().ok_or(ErrorKind::TokenInvalid)?;
            agent.role()
        }
    };
    let access = concept::permission::Access::new(invitation.issuer().clone(), role);
    if !access.allows(concept::permission::Requirement::Admin) {
        return Err(ErrorKind::TokenInvalid.into());
    }
    super::require(
        infra::node::join_info(docker, invitation.role()),
        ErrorKind::NoSwarmManager,
    )
    .await
}

pub(crate) async fn silent_agents(
    docker: &infra::docker::Docker,
    api: &infra::insight::query::Api,
    now: Timestamp,
) -> Result<Vec<(String, Option<Timestamp>)>> {
    let Some((_, interval)) = infra::insight::agent::observe(docker).await? else {
        return Ok(Vec::new());
    };
    let nodes = infra::node::observe_all(docker).await?;
    let since = config::expiry(
        now,
        SignedDuration::from_secs(-i64::from(config::AGENT_REPORT_WINDOW_SECS)),
    );
    let last: HashMap<_, _> = api
        .node_latest((since, now))
        .await?
        .into_iter()
        .map(|report| (report.node_id, report.observed_at))
        .collect();
    let freshness = config::agent_freshness(interval);
    let grace = SignedDuration::from_secs(i64::from(config::AGENT_REPORT_WINDOW_SECS));
    Ok(nodes
        .iter()
        .filter(|node| !matches!(node.availability(), concept::node::Availability::Drain))
        .filter_map(|node| {
            let observed = last.get(node.id().as_str()).copied();
            match observed {
                Some(at) if now.duration_since(at) <= freshness => None,
                None if now.duration_since(node.created_at()) <= grace => None,
                observed => Some((node.id().clone(), observed)),
            }
        })
        .collect())
}

pub(crate) async fn metrics(
    docker: &infra::docker::Docker,
    api: &infra::insight::query::Api,
    id: &str,
    range: (jiff::Timestamp, jiff::Timestamp),
    step_secs: u32,
) -> Result<(
    Vec<insight_wire::metric::HostBucket>,
    Vec<insight_wire::metric::FilesystemBucket>,
)> {
    super::insight::require_server(docker).await?;
    tokio::try_join!(
        api.host_series(range, id, step_secs),
        api.filesystem_series(
            range,
            id,
            config::step_floor(
                step_secs,
                insight_wire::metric::FILESYSTEM_SAMPLE_INTERVAL_SECS,
            ),
        ),
    )
}
