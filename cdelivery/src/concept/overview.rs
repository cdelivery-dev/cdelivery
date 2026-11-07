use getset::CopyGetters;

#[derive(Debug)]
pub(crate) enum Metrics {
    Disabled,
    Unavailable(String),
    Ready {
        step_secs: u32,
        series: Vec<insight_wire::metric::OverviewSeries>,
        node_names: std::collections::BTreeMap<String, String>,
        access_step_secs: u32,
        access: Vec<pingress_wire::access::StatsRow>,
    },
}

#[derive(Debug, Clone, Copy, CopyGetters, macros_derive::Ctor)]
#[getset(get_copy = "pub(crate)")]
pub(crate) struct Instances {
    running: u64,
    desired: u64,
}

impl Instances {
    pub(crate) fn ready(&self) -> bool {
        self.desired > 0 && self.running >= self.desired
    }
}

#[derive(Debug, CopyGetters)]
#[getset(get_copy = "pub(crate)")]
pub(crate) struct Summary {
    nodes_ready: usize,
    nodes_total: usize,
    replicas_running: u64,
    replicas_desired: u64,
    pingress: Option<Instances>,
    insight_status: super::insight::Status,
    insight_server: Option<Instances>,
    insight_agents: Option<Instances>,
    insight_retention_days: Option<u16>,
}

pub(crate) fn summary(
    nodes: &[super::node::Node],
    units: &[(super::name::Name, String, u64, u64)],
    deployment: &super::insight::Deployment,
    pingress: Option<Instances>,
    insight_server: Option<Instances>,
    insight_agents: Option<Instances>,
) -> Summary {
    Summary {
        nodes_ready: nodes
            .iter()
            .filter(|node| matches!(node.state(), super::node::State::Ready))
            .count(),
        nodes_total: nodes.len(),
        replicas_running: units.iter().map(|(_, _, running, _)| running).sum(),
        replicas_desired: units.iter().map(|(_, _, _, desired)| desired).sum(),
        pingress,
        insight_status: deployment.status(),
        insight_server,
        insight_agents,
        insight_retention_days: deployment
            .server()
            .as_ref()
            .map(super::insight::Insight::retention_days),
    }
}
