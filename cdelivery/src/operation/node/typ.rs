use garde::Validate;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::concept;

pub(crate) const LABELS_MAX: usize = 32;

macros::mirror_enum! {
    #[macros_derive::validated_schema]
    #[derive(Debug, Serialize, Deserialize, ToSchema, Validate)]
    NodeSelector both concept::node::Selector {
        /// Node ID from `node_list`.
        NodeId(
            #[garde(custom(super::super::validate::valid_node_id))]
            id: String
        ),
        /// Nodes with this scheduling label, set by `node_relabel`.
        Label(
            #[garde(custom(super::super::validate::valid_name))]
            label: String
        ),
    }
}

macros::mirror_enum! {
    #[derive(Debug, Serialize, Deserialize, ToSchema)]
    NodeRole both concept::node::Role { Manager, Worker }
}

macros::mirror_enum! {
    #[derive(Debug, PartialEq, Serialize, ToSchema)]
    NodeAvailability from concept::node::Availability { Active, Pause, Drain }
}

macros::mirror_enum! {
    #[derive(Debug, Serialize, ToSchema)]
    NodeState from concept::node::State { Ready, Down, Disconnected, Unknown }
}

macros::mirror_enum! {
    #[derive(Debug, Serialize, ToSchema)]
    NodeReachability from concept::node::Reachability { Reachable, Unreachable, Unknown }
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct Node {
    pub(crate) id: String,
    pub(crate) hostname: String,
    pub(crate) role: NodeRole,
    pub(crate) availability: NodeAvailability,
    pub(crate) state: NodeState,
    pub(crate) reachability: Option<NodeReachability>,
    pub(crate) cpus: f64,
    pub(crate) memory_bytes: u64,
    pub(crate) engine_version: String,
    pub(crate) addr: String,
    pub(crate) labels: Vec<String>,
    pub(crate) created_at: Timestamp,
    pub(crate) os: Option<String>,
    pub(crate) architecture: Option<String>,
    pub(crate) leader: Option<bool>,
    pub(crate) status_message: String,
}

super::super::typ::mirror_struct! {
    Node from node: concept::node::Node {
        id: node.id().clone(),
        hostname: node.hostname().clone(),
        role: node.role().into(),
        availability: node.availability().into(),
        state: node.state().into(),
        reachability: node.reachability().map(Into::into),
        cpus: node.cpus(),
        memory_bytes: node.memory_bytes(),
        engine_version: node.engine_version().clone(),
        addr: node.addr().clone(),
        labels: node.labels().clone(),
        created_at: node.created_at(),
        os: node.os().clone(),
        architecture: node.architecture().clone(),
        leader: node.leader(),
        status_message: node.status_message().clone(),
    }
}

#[derive(Serialize, ToSchema)]
pub(crate) struct Nodes {
    pub(crate) nodes: Vec<Node>,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct SilentAgent {
    pub(crate) node_id: String,
    pub(crate) last_report_at: Option<Timestamp>,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct SilentAgents {
    pub(crate) silent_agents: Vec<SilentAgent>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct NodeSeries {
    pub(crate) step_secs: u32,
    pub(crate) host: Vec<insight_wire::metric::HostBucket>,
    pub(crate) filesystems: Vec<insight_wire::metric::FilesystemBucket>,
}

#[macros_derive::validated_schema]
#[derive(Deserialize, ToSchema, Validate)]
pub(crate) struct CreateJoinInvitationRequest {
    #[garde(skip)]
    pub(crate) role: NodeRole,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct JoinInvitation {
    pub(crate) invitation: String,
    pub(crate) expires_at: Timestamp,
    pub(crate) join_url: String,
}

const _: () = assert!(LABELS_MAX == 32);

#[macros_derive::validated_schema]
#[derive(Deserialize, ToSchema, Validate)]
pub(crate) struct UpdateNodeRequest {
    #[garde(length(max = 32), inner(custom(super::super::validate::valid_name)))]
    pub(crate) labels: Vec<String>,
}
