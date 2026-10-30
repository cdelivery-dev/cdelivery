use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use askama::Template;
use axum::Extension;
use axum::extract::{OriginalUri, State};
use axum::response::Response;
use http::StatusCode;
use jiff::Timestamp;
use serde_json::json;

use crate::locale::tt;
use crate::{concept, config, operation};

#[derive(Template)]
#[template(path = "overview/index.html")]
struct Page {
    dashboard: super::dashboard::Dashboard,
    window: super::range::Window,
    overview: operation::overview::typ::OverviewSummary,
    metrics: MetricsView,
}

pub(super) enum MetricsView {
    Disabled,
    Unavailable(String),
    Ready(Vec<Panel>),
}

pub(super) struct Panel {
    key: &'static str,
    title: std::borrow::Cow<'static, str>,
    unit: &'static str,
    stacked: bool,
    chart: Option<SeriesChart>,
}

impl Panel {
    fn new(key: &'static str, chart: Option<SeriesChart>) -> Self {
        let (title, unit) = super::metrics::meta(key);
        Self {
            key,
            title: if key == "cpu_usage" {
                super::metrics::meta("cpu_utilization").0
            } else {
                title
            },
            unit,
            stacked: matches!(
                key,
                "cpu_usage" | "memory_working_set" | "disk_read_rate" | "disk_write_rate"
            ),
            chart,
        }
    }

    fn single(
        key: &'static str,
        stats: &operation::pingress::typ::AccessStats,
        value: fn(&operation::pingress::typ::AccessBucket) -> Option<f64>,
    ) -> Self {
        if !stats.buckets.iter().any(|bucket| value(bucket).is_some()) {
            return Self::new(key, None);
        }
        let data = super::metrics::rows(
            stats.step_secs,
            i64::from(stats.step_secs),
            &stats.buckets,
            |bucket| bucket.start.as_second(),
            |bucket| Some(([value(bucket)], super::metrics::Extras::default())),
        );
        Self::new(
            key,
            data.map(|data| SeriesChart {
                series_labels: json!([super::metrics::meta(key).0]),
                data,
                ..Default::default()
            }),
        )
    }
}

#[derive(Default)]
struct SeriesChart {
    series_labels: serde_json::Value,
    series_kinds: serde_json::Value,
    data: String,
}

pub(super) fn routes() -> axum::Router<Arc<crate::boot::App>> {
    axum::Router::new().route("/", axum::routing::get(get))
}

async fn get(
    State(app): State<Arc<crate::boot::App>>,
    Extension(access): Extension<concept::permission::Access>,
    OriginalUri(uri): OriginalUri,
) -> super::response::Result<Response> {
    let now = Timestamp::now();
    let window = super::range::Window::page(&uri, now)?;
    let query = operation::typ::metric::RangeQuery {
        start: Some(window.range.0),
        end: Some(window.range.1),
        max_buckets: Some(config::UI_BUCKETS),
    };
    let (dashboard, overview) = tokio::try_join!(
        super::dashboard::Dashboard::load(&app, &access, super::dashboard::Location::Overview),
        operation::execute::<operation::overview::Get>(&app, &access, query, now),
    )?;
    let metrics = match overview.metrics {
        operation::overview::typ::OverviewMetrics::Disabled => MetricsView::Disabled,
        operation::overview::typ::OverviewMetrics::Unavailable { error } => {
            MetricsView::Unavailable(error)
        }
        operation::overview::typ::OverviewMetrics::Ready { charts } => {
            MetricsView::Ready(panels(&charts))
        }
    };
    super::response::render(
        &Page {
            dashboard,
            window,
            overview: overview.summary,
            metrics,
        },
        StatusCode::OK,
    )
}

fn panels(charts: &operation::overview::typ::OverviewCharts) -> Vec<Panel> {
    let mut panels: Vec<_> = [
        "cpu_usage",
        "memory_working_set",
        "disk_read_rate",
        "disk_write_rate",
        "network_receive_rate",
        "network_transmit_rate",
        "psi_cpu_some",
        "psi_memory_some",
        "psi_memory_full",
        "psi_io_some",
        "psi_io_full",
    ]
    .into_iter()
    .map(|key| Panel::new(key, metric_chart(charts, key)))
    .collect();
    panels.extend(access_panels(&charts.access));
    panels.push(Panel::new(
        "utilization",
        metric_chart(charts, "utilization"),
    ));
    panels
}

fn access_panels(stats: &operation::pingress::typ::AccessStats) -> Vec<Panel> {
    vec![
        Panel::new(
            "pingress",
            super::metrics::access::overview_qps(stats).map(|(labels, data)| SeriesChart {
                series_labels: json!(labels),
                data,
                ..Default::default()
            }),
        ),
        Panel::single("pingress_errors", stats, |bucket| {
            Some(super::metrics::access::class_ratio(
                bucket,
                pingress_wire::access::StatusClass::ServerError,
            ))
        }),
        Panel::single("pingress_latency", stats, |bucket| {
            bucket.aggregate.duration_us.p95.map(|value| value / 1e6)
        }),
    ]
}

fn labels(
    lines: &[&insight_wire::metric::OverviewSeries],
    nodes: &BTreeMap<String, String>,
) -> (Vec<String>, Vec<&'static str>) {
    let node_name = |id: &String| nodes.get(id).cloned().unwrap_or_else(|| id.clone());
    let names: Vec<_> = lines
        .iter()
        .map(|line| match &line.target {
            insight_wire::metric::OverviewTarget::Service { service_id, name } => {
                (name.clone(), "Service", Some(service_id))
            }
            insight_wire::metric::OverviewTarget::Rest => {
                (tt!("ui.overview.other").into_owned(), "Rest", None)
            }
            insight_wire::metric::OverviewTarget::Host => {
                (tt!("ui.overview.host").into_owned(), "Host", None)
            }
            insight_wire::metric::OverviewTarget::Node { node_id } => {
                (node_name(node_id), "Node", Some(node_id))
            }
            insight_wire::metric::OverviewTarget::Filesystem {
                node_id,
                mount_point,
            } => (
                format!("{}: {mount_point}", node_name(node_id)),
                "Filesystem",
                Some(node_id),
            ),
            insight_wire::metric::OverviewTarget::Cluster => (
                super::metrics::meta(&line.metric).0.into_owned(),
                "Cluster",
                None,
            ),
        })
        .collect();
    names
        .iter()
        .map(|(name, kind, id)| {
            let label = match id
                .filter(|_| names.iter().filter(|(other, _, _)| other == name).count() > 1)
            {
                Some(id) => format!("{name} ({})", id.get(..12).unwrap_or(id)),
                None => name.clone(),
            };
            (label, *kind)
        })
        .unzip()
}

fn metric_chart(
    charts: &operation::overview::typ::OverviewCharts,
    key: &str,
) -> Option<SeriesChart> {
    let lines: Vec<_> = charts
        .series
        .iter()
        .filter(|line| line.metric == key)
        .collect();
    if lines
        .iter()
        .all(|line| line.points.iter().all(|point| point.value.is_none()))
    {
        return None;
    }
    let (labels, kinds) = labels(&lines, &charts.node_names);
    let filesystem = key == "utilization";
    let gap = if filesystem {
        super::metrics::filesystem_gap(charts.step_secs)
    } else {
        super::metrics::metric_gap(charts.step_secs)
    };
    let maps: Vec<BTreeMap<i64, Option<&insight_wire::metric::OverviewPoint>>> = lines
        .iter()
        .map(|line| {
            let mut points = BTreeMap::new();
            let mut previous: Option<i64> = None;
            for point in &line.points {
                let at = point.timestamp.as_second();
                if let Some(previous) = previous
                    && at.saturating_sub(previous) > gap
                {
                    points.insert(previous.saturating_add(gap), None);
                }
                points.insert(at, (!filesystem || point.value.is_some()).then_some(point));
                previous = Some(at);
            }
            points
        })
        .collect();
    let capacity_key = match key {
        "cpu_usage" => "cpu_total",
        "memory_working_set" => "memory_total",
        _ => "",
    };
    let capacity: BTreeMap<_, _> = charts
        .series
        .iter()
        .filter(|line| line.metric == capacity_key)
        .flat_map(|line| &line.points)
        .map(|point| (point.timestamp.as_second(), point.value))
        .collect();
    let ats: BTreeSet<_> = maps.iter().flat_map(|map| map.keys().copied()).collect();
    let rows: Vec<_> = ats
        .into_iter()
        .map(|at| {
            let mut values = Vec::with_capacity(maps.len());
            let mut extras = filesystem.then(|| Vec::with_capacity(maps.len()));
            for map in &maps {
                let point = map.get(&at);
                values.push(match point {
                    None => json!(null),
                    Some(None) => json!(false),
                    Some(Some(point)) => json!(point.value),
                });
                if let Some(extras) = &mut extras {
                    extras.push(
                        point
                            .copied()
                            .flatten()
                            .map(|point| [point.used, point.total]),
                    );
                }
            }
            json!({"at":at,"v":values,"e":extras,"total":capacity.get(&at).copied().flatten()})
        })
        .collect();
    Some(SeriesChart {
        series_labels: json!(labels),
        series_kinds: json!(kinds),
        data: json!(rows).to_string(),
    })
}
