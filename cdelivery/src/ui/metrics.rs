use std::borrow::Cow;
use std::collections::BTreeMap;

use insight_wire::metric::Stat;
use serde::Serialize;

use crate::locale::tt;

pub(super) mod access;
pub(super) mod container;
pub(super) mod page;

pub(super) enum MetricsState {
    Disabled,
    Empty,
    Charts(Vec<Group>),
}

pub(super) fn state(groups: Option<Vec<Group>>) -> MetricsState {
    match groups {
        None => MetricsState::Disabled,
        Some(groups) if groups.is_empty() => MetricsState::Empty,
        Some(groups) => MetricsState::Charts(groups),
    }
}

pub(super) struct Group {
    pub(super) key: &'static str,
    pub(super) title: Cow<'static, str>,
    pub(super) charts: Vec<Chart>,
}

macro_rules! groups {
    ($($key:literal => ($title:expr, $charts:expr $(,)?);)*) => {{
        [$($crate::ui::metrics::Group {
            key: $key,
            title: $title,
            charts: ($charts).into_iter().flatten().collect(),
        }),*]
            .into_iter()
            .filter(|group| !group.charts.is_empty())
            .collect::<Vec<_>>()
    }};
}
pub(super) use groups;

#[derive(Default)]
pub(super) struct Chart {
    pub(super) title: Cow<'static, str>,
    pub(super) unit: &'static str,
    pub(super) detail_unit: Option<&'static str>,
    pub(super) detail_label: Option<Cow<'static, str>>,
    pub(super) total_label: Option<Cow<'static, str>>,
    pub(super) stat_labels: Option<&'static str>,
    pub(super) data: String,
}

pub(super) fn metric_gap(step_secs: u32) -> i64 {
    i64::from(step_secs).max(i64::from(insight_wire::metric::SAMPLE_INTERVAL_MAX_SECS))
}

pub(super) fn filesystem_gap(step_secs: u32) -> i64 {
    i64::from(step_secs).max(2 * i64::from(insight_wire::metric::FILESYSTEM_SAMPLE_INTERVAL_SECS))
}

pub(super) struct ChartBuilder<'a, B> {
    pub(super) step_secs: u32,
    pub(super) points: &'a [B],
    pub(super) at: fn(&B) -> jiff::Timestamp,
    pub(super) series: fn(&B) -> &BTreeMap<String, Stat>,
    pub(super) detail_pairs: &'a [(&'static str, &'static str)],
    pub(super) replicas: Option<fn(&B) -> u32>,
}

impl<B> ChartBuilder<'_, B> {
    pub(super) fn chart(&self, name: &str) -> Option<Chart> {
        let detail_name = self
            .detail_pairs
            .iter()
            .find_map(|(main, detail)| (*main == name).then_some(*detail));
        chart(
            name,
            detail_name,
            self.step_secs,
            self.points,
            |point| (self.at)(point).as_second(),
            |point| {
                let series = (self.series)(point);
                let values = stat_values(series.get(name)?)?;
                let extras = Extras {
                    detail: detail_name
                        .and_then(|name| series.get(name))
                        .and_then(|stat| stat.avg),
                    total: None,
                    replicas: self.replicas.map(|replicas| replicas(point)),
                };
                Some((values, extras))
            },
        )
    }

    pub(super) fn charts(&self, names: &[&str]) -> Vec<Option<Chart>> {
        names.iter().map(|name| self.chart(name)).collect()
    }
}

#[derive(Serialize)]
struct Row<V> {
    at: i64,
    v: Option<V>,
    #[serde(flatten)]
    extras: Extras,
}

#[derive(Default, Serialize)]
pub(super) struct Extras {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) detail: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) total: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) replicas: Option<u32>,
}

pub(super) fn stat_values(stat: &Stat) -> Option<[f64; 3]> {
    Some([stat.min?, stat.avg?, stat.max?])
}

pub(super) fn meta(name: &str) -> (Cow<'static, str>, &'static str) {
    match name {
        "utilization" => (tt!("ui.overview.filesystem.title"), "percent"),
        "pingress" => (tt!("ui.overview.pingress.title"), "events"),
        "pingress_errors" => (tt!("ui.metrics.chart.errorRatio"), "percent"),
        "pingress_latency" => (tt!("ui.overview.chart.p95"), "duration"),
        "replicas" => (tt!("ui.metrics.chart.replicas"), "count"),
        "pids" => (tt!("ui.metrics.chart.pids"), "count"),
        "pids_limit" => (tt!("ui.metrics.chart.pidsLimit"), "count"),
        "cpu_utilization" => (tt!("ui.node.metrics.cpu.utilization"), "percent"),
        "cpu_usage" | "cpu_used" => (tt!("ui.metrics.chart.cpuUsed"), "cores"),
        "cpu_limit" => (tt!("ui.metrics.chart.cpuLimit"), "cores"),
        "cpu_throttled" => (tt!("ui.metrics.chart.cpuThrottled"), "percent"),
        "cpu_user" => (tt!("ui.node.metrics.cpu.user"), "percent"),
        "cpu_system" => (tt!("ui.node.metrics.cpu.system"), "percent"),
        "cpu_iowait" => (tt!("ui.node.metrics.cpu.iowait"), "percent"),
        "cpu_steal" => (tt!("ui.node.metrics.cpu.steal"), "percent"),
        "cpu_total" => (tt!("ui.node.metrics.cpu.total"), "cores"),
        "memory_utilization" => (tt!("ui.node.metrics.memory.utilization"), "percent"),
        "memory_working_set" => (tt!("ui.metrics.chart.memWorkingSet"), "bytes"),
        "memory_current" => (tt!("ui.metrics.chart.memCurrent"), "bytes"),
        "memory_anon" => (tt!("ui.metrics.chart.memAnon"), "bytes"),
        "memory_limit" => (tt!("ui.metrics.chart.memLimit"), "bytes"),
        "memory_swap" | "swap_used" => (tt!("ui.metrics.chart.memSwap"), "bytes"),
        "memory_used" => (tt!("ui.node.metrics.memory.used"), "bytes"),
        "memory_available" => (tt!("ui.node.metrics.memory.available"), "bytes"),
        "memory_total" => (tt!("ui.node.metrics.memory.total"), "bytes"),
        "swap_total" => (tt!("ui.node.metrics.memory.swapTotal"), "bytes"),
        "oom_kill_rate" => (tt!("ui.metrics.chart.oomKillRate"), "events"),
        "disk_read_rate" => (tt!("ui.metrics.chart.diskReadRate"), "bytes-rate"),
        "disk_write_rate" => (tt!("ui.metrics.chart.diskWriteRate"), "bytes-rate"),
        "disk_read_iops" => (tt!("ui.metrics.chart.diskReadIops"), "events"),
        "disk_write_iops" => (tt!("ui.metrics.chart.diskWriteIops"), "events"),
        "psi_cpu_some" => (tt!("ui.metrics.chart.psiCpuSome"), "percent"),
        "psi_memory_some" => (tt!("ui.metrics.chart.psiMemorySome"), "percent"),
        "psi_memory_full" => (tt!("ui.metrics.chart.psiMemoryFull"), "percent"),
        "psi_io_some" => (tt!("ui.metrics.chart.psiIoSome"), "percent"),
        "psi_io_full" => (tt!("ui.metrics.chart.psiIoFull"), "percent"),
        "network_receive_rate" => (tt!("ui.metrics.chart.networkReceiveRate"), "bytes-rate"),
        "network_transmit_rate" => (tt!("ui.metrics.chart.networkTransmitRate"), "bytes-rate"),
        "tcp_retrans_rate" => (tt!("ui.metrics.chart.tcpRetransRate"), "events"),
        "node_uptime" => (tt!("ui.node.metrics.system.uptime"), "duration"),
        _ => (Cow::Owned(name.to_owned()), "count"),
    }
}

pub(super) fn chart<B>(
    name: &str,
    detail_name: Option<&str>,
    step_secs: u32,
    points: &[B],
    at: impl Fn(&B) -> i64,
    values_at: impl Fn(&B) -> Option<([f64; 3], Extras)>,
) -> Option<Chart> {
    let data = rows(step_secs, metric_gap(step_secs), points, at, values_at)?;
    let (title, unit) = meta(name);
    Some(Chart {
        title,
        unit,
        detail_unit: detail_name.map(|name| meta(name).1),
        data,
        ..Default::default()
    })
}

pub(super) fn rows<B: Copy, V: Serialize>(
    step_secs: u32,
    gap_secs: i64,
    points: impl IntoIterator<Item = B>,
    at: impl Fn(B) -> i64,
    values_at: impl Fn(B) -> Option<(V, Extras)>,
) -> Option<String> {
    let step = i64::from(step_secs);
    let mut rows = Vec::new();
    let mut previous: Option<i64> = None;
    for point in points {
        let Some((values, extras)) = values_at(point) else {
            continue;
        };
        let at = at(point);
        if let Some(previous) = previous
            && at.saturating_sub(previous) > gap_secs
        {
            rows.push(Row {
                at: previous.saturating_add(step),
                v: None,
                extras: Extras::default(),
            });
        }
        rows.push(Row {
            at,
            v: Some(values),
            extras,
        });
        previous = Some(at);
    }
    (!rows.is_empty()).then(|| serde_json::json!(rows).to_string())
}
