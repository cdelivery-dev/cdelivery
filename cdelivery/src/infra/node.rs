pub(crate) mod join;

use std::collections::HashMap;

use docker::types::{NodeAvailability, NodeReachability, NodeRole, NodeState, Placement};

use crate::concept;
use crate::error::{Result, SpecProblem};

pub(crate) const LABEL_PREFIX: &str = "dev.cdelivery.labels.";
const ID_CONSTRAINT_PREFIX: &str = "node.id==";
const NODE_LABELS: &str = "node.labels.";
const LABEL_VALUE: &str = "true";

pub(super) fn id_constraint(id: &str) -> String {
    format!("{ID_CONSTRAINT_PREFIX}{id}")
}

pub(super) fn label_constraint(label: &str) -> String {
    format!("{NODE_LABELS}{LABEL_PREFIX}{label}=={LABEL_VALUE}")
}

pub(super) fn selector_constraint(selector: &concept::node::Selector) -> String {
    match selector {
        concept::node::Selector::NodeId(id) => id_constraint(id),
        concept::node::Selector::Label(label) => label_constraint(label),
    }
}

pub(super) fn decode_selector(
    placement: Option<&Placement>,
    service: &str,
) -> Result<concept::node::Selector> {
    let bad = || SpecProblem::BadPlacement(service.to_owned());
    let constraints = placement
        .and_then(|placement| placement.constraints.as_deref())
        .ok_or_else(bad)?;
    let [constraint] = constraints else {
        return Err(bad().into());
    };
    decode_constraint(constraint).ok_or_else(|| bad().into())
}

fn decode_constraint(constraint: &str) -> Option<concept::node::Selector> {
    if let Some(id) = constraint.strip_prefix(ID_CONSTRAINT_PREFIX) {
        return concept::node::valid_id(id).then(|| concept::node::Selector::NodeId(id.to_owned()));
    }
    let label: concept::name::Name = constraint
        .strip_prefix(NODE_LABELS)?
        .strip_prefix(LABEL_PREFIX)?
        .strip_suffix(LABEL_VALUE)?
        .strip_suffix("==")?
        .parse()
        .ok()?;
    Some(concept::node::Selector::Label(label.to_string()))
}

fn decode(node: docker::types::Node) -> concept::node::Node {
    let (os, architecture) = node.description.platform.map_or((None, None), |platform| {
        (platform.os, platform.architecture)
    });
    let (reachability, leader) = node
        .manager_status
        .map(|manager| (reachability(&manager.reachability), manager.leader))
        .unzip();
    concept::node::Node::new(
        node.id,
        node.description.hostname,
        node.spec.role.into(),
        node.spec.availability.into(),
        state(&node.status.state),
        reachability,
        node.description.resources.nano_cpus.cores(),
        node.description.resources.memory_bytes,
        node.description.engine.engine_version,
        node.status.addr,
        labels(&node.spec.labels),
        node.created_at,
        os,
        architecture,
        leader,
        node.status.message,
    )
}

macros::mirror_enum! { NodeRole => concept::node::Role { Manager, Worker } }
macros::mirror_enum! { NodeAvailability => concept::node::Availability { Active, Pause, Drain } }

fn state(state: &NodeState) -> concept::node::State {
    match state {
        NodeState::Ready => concept::node::State::Ready,
        NodeState::Down => concept::node::State::Down,
        NodeState::Disconnected => concept::node::State::Disconnected,
        NodeState::Unknown(_) => concept::node::State::Unknown,
    }
}

fn reachability(reachability: &NodeReachability) -> concept::node::Reachability {
    match reachability {
        NodeReachability::Reachable => concept::node::Reachability::Reachable,
        NodeReachability::Unreachable => concept::node::Reachability::Unreachable,
        NodeReachability::Unknown(_) => concept::node::Reachability::Unknown,
    }
}

fn labels(all: &HashMap<String, String>) -> Vec<String> {
    let mut labels: Vec<String> = all
        .iter()
        .filter(|(_, v)| v.as_str() == LABEL_VALUE)
        .filter_map(|(k, _)| k.strip_prefix(LABEL_PREFIX))
        .map(str::to_owned)
        .collect();
    labels.sort();
    labels
}

pub(crate) async fn observe(
    docker: &super::docker::Docker,
    id: &str,
) -> Result<Option<concept::node::Node>> {
    if !concept::node::valid_id(id) {
        return Ok(None);
    }
    Ok(docker.node_inspect(id).await?.map(decode))
}

pub(crate) async fn observe_all(
    docker: &super::docker::Docker,
) -> Result<Vec<concept::node::Node>> {
    let mut nodes: Vec<_> = docker.node_list().await?.into_iter().map(decode).collect();
    nodes.sort_by(|a, b| (a.hostname(), a.id()).cmp(&(b.hostname(), b.id())));
    Ok(nodes)
}

pub(crate) async fn relabel(
    docker: &super::docker::Docker,
    id: &str,
    labels: &[concept::name::Name],
) -> Result<Option<concept::node::Node>> {
    if !concept::node::valid_id(id) {
        return Ok(None);
    }
    let Some(mut node) = docker.node_inspect(id).await? else {
        return Ok(None);
    };
    let version = node.version.index;
    node.spec.labels.retain(|k, _| !k.starts_with(LABEL_PREFIX));
    for label in labels {
        node.spec
            .labels
            .insert(format!("{LABEL_PREFIX}{label}"), LABEL_VALUE.to_owned());
    }
    docker.node_update(id, version, &node.spec).await?;
    Ok(Some(decode(node)))
}

pub(crate) async fn delete(docker: &super::docker::Docker, id: &str) -> Result<()> {
    if !concept::node::valid_id(id) {
        return Ok(());
    }
    docker.node_delete(id).await
}

pub(crate) async fn manager_addr(
    docker: &super::docker::Docker,
) -> Result<Option<std::net::SocketAddr>> {
    let Some(swarm) = docker.system_info().await?.swarm else {
        return Ok(None);
    };
    Ok(swarm
        .remote_managers
        .iter()
        .find(|m| m.node_id == swarm.node_id)
        .or_else(|| swarm.remote_managers.first())
        .map(|manager| manager.addr))
}

pub(crate) async fn join_info(
    docker: &super::docker::Docker,
    role: concept::node::Role,
) -> Result<Option<concept::node::JoinInfo>> {
    let Some(manager_addr) = manager_addr(docker).await? else {
        return Ok(None);
    };
    let tokens = docker.swarm_inspect().await?.join_tokens;
    let token = match role {
        concept::node::Role::Worker => tokens.worker,
        concept::node::Role::Manager => tokens.manager,
    };
    Ok(Some(concept::node::JoinInfo::new(
        manager_addr,
        cipher::Secret::new(token),
    )))
}
