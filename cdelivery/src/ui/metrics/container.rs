use insight_wire::metric::{ContainerBucket, ContainerInstance};

use crate::locale::tt;
use crate::operation;

pub(in crate::ui) struct Instance {
    pub(in crate::ui) id: String,
    pub(in crate::ui) label: String,
}

const DETAIL_PAIRS: &[(&str, &str)] = &[
    ("cpu_utilization", "cpu_usage"),
    ("memory_utilization", "memory_working_set"),
];

const CPU: &[&str] = &[
    "cpu_utilization",
    "cpu_usage",
    "cpu_limit",
    "cpu_throttled",
    "psi_cpu_some",
];

const MEMORY: &[&str] = &[
    "memory_utilization",
    "memory_working_set",
    "memory_current",
    "memory_anon",
    "memory_limit",
    "memory_swap",
    "oom_kill_rate",
    "psi_memory_some",
    "psi_memory_full",
];

const DISK: &[&str] = &[
    "disk_read_rate",
    "disk_write_rate",
    "disk_read_iops",
    "disk_write_iops",
    "psi_io_some",
    "psi_io_full",
];

const NETWORK: &[&str] = &[
    "network_receive_rate",
    "network_transmit_rate",
    "tcp_retrans_rate",
];

const PROCESSES: &[&str] = &["pids", "pids_limit"];

pub(in crate::ui) fn charts(
    full: bool,
    step_secs: u32,
    points: &[ContainerBucket],
    access: Vec<super::Group>,
) -> Vec<super::Group> {
    let builder = super::ChartBuilder {
        step_secs,
        points,
        at: |p| p.timestamp,
        series: |p| &p.series,
        detail_pairs: DETAIL_PAIRS,
        replicas: Some(|p| p.replicas),
    };
    let charts = |names, summary| {
        if full {
            builder.charts(names)
        } else {
            vec![builder.chart(summary)]
        }
    };
    let mut groups = super::groups! {
        "cpu" => (tt!("ui.metrics.group.cpu"), charts(CPU, "cpu_usage"));
        "memory" => (tt!("ui.metrics.group.memory"), charts(MEMORY, "memory_working_set"));
    };
    groups.extend(access);
    if full {
        groups.extend(super::groups! {
            "disk" => (tt!("ui.metrics.group.disk"), builder.charts(DISK));
            "network" => (tt!("ui.metrics.group.network"), builder.charts(NETWORK));
            "replicas" => (tt!("ui.metrics.group.replicas"), [replicas(step_secs, points)]);
            "processes" => (tt!("ui.metrics.group.processes"), builder.charts(PROCESSES));
        });
    }
    groups
}

pub(in crate::ui) fn instances(
    items: &[ContainerInstance],
    selected: &str,
    nodes: &[operation::node::typ::Node],
) -> Vec<Instance> {
    let mut instances = Vec::new();
    for instance in items {
        if instances
            .iter()
            .any(|option: &Instance| option.id == instance.container_id)
        {
            continue;
        }
        let node_label = nodes
            .iter()
            .find(|node| node.id == instance.node_id)
            .filter(|node| !node.addr.is_empty())
            .map_or(truncated(&instance.node_id), |node| node.addr.as_str());
        instances.push(Instance {
            id: instance.container_id.clone(),
            label: format!("{} ({node_label})", truncated(&instance.container_id)),
        });
    }
    if !selected.is_empty() && !instances.iter().any(|option| option.id == selected) {
        instances.push(Instance {
            id: selected.to_owned(),
            label: truncated(selected).to_owned(),
        });
    }
    instances
}

fn replicas(step_secs: u32, points: &[ContainerBucket]) -> Option<super::Chart> {
    super::chart(
        "replicas",
        None,
        step_secs,
        points,
        |p| p.timestamp.as_second(),
        |p| Some(([f64::from(p.replicas); 3], super::Extras::default())),
    )
}

fn truncated(id: &str) -> &str {
    id.get(..12).unwrap_or(id)
}
