use jiff::Timestamp;
use serde::Serialize;
use utoipa::ToSchema;

use crate::concept;

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct Overview {
    pub(crate) summary: OverviewSummary,
    pub(crate) metrics: OverviewMetrics,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct OverviewSummary {
    pub(crate) nodes_ready: usize,
    pub(crate) nodes_total: usize,
    pub(crate) replicas_running: u64,
    pub(crate) replicas_desired: u64,
    pub(crate) pingress: Option<ServiceInstances>,
    pub(crate) insight_status: super::super::insight::typ::InsightStatus,
    pub(crate) insight_server: Option<ServiceInstances>,
    pub(crate) insight_agents: Option<ServiceInstances>,
    pub(crate) insight_retention_days: Option<u16>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct ServiceInstances {
    pub(crate) running: u64,
    pub(crate) desired: u64,
    pub(crate) ready: bool,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "status")]
pub(crate) enum OverviewMetrics {
    Disabled,
    Unavailable { error: String },
    Ready { charts: OverviewCharts },
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct OverviewCharts {
    pub(crate) step_secs: u32,
    pub(crate) series: Vec<insight_wire::metric::OverviewSeries>,
    pub(crate) node_names: std::collections::BTreeMap<String, String>,
    pub(crate) access: super::super::pingress::typ::AccessStats,
}

super::super::typ::mirror_struct! {
    OverviewSummary from summary: &concept::overview::Summary {
        nodes_ready: summary.nodes_ready(),
        nodes_total: summary.nodes_total(),
        replicas_running: summary.replicas_running(),
        replicas_desired: summary.replicas_desired(),
        pingress: summary.pingress().map(Into::into),
        insight_status: summary.insight_status().into(),
        insight_server: summary.insight_server().map(Into::into),
        insight_agents: summary.insight_agents().map(Into::into),
        insight_retention_days: summary.insight_retention_days(),
    }
}

super::super::typ::mirror_struct! {
    ServiceInstances from instances: concept::overview::Instances {
        running: instances.running(),
        desired: instances.desired(),
        ready: instances.ready(),
    }
}

impl Overview {
    pub(crate) fn assemble(
        summary: &concept::overview::Summary,
        metrics: concept::overview::Metrics,
        range: (Timestamp, Timestamp),
    ) -> Self {
        let metrics = match metrics {
            concept::overview::Metrics::Disabled => OverviewMetrics::Disabled,
            concept::overview::Metrics::Unavailable(error) => {
                OverviewMetrics::Unavailable { error }
            }
            concept::overview::Metrics::Ready {
                step_secs,
                series,
                node_names,
                access_step_secs,
                access,
            } => OverviewMetrics::Ready {
                charts: OverviewCharts {
                    step_secs,
                    series,
                    node_names,
                    access: super::super::pingress::typ::AccessStats::assemble(
                        range,
                        access_step_secs,
                        access,
                        None,
                    ),
                },
            },
        };
        Self {
            summary: summary.into(),
            metrics,
        }
    }
}
