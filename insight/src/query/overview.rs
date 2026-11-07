use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum_extra::extract::Query;
use datafusion::arrow::array::{Array, Float64Array, RecordBatch, TimestampNanosecondArray};
use datafusion::arrow::datatypes::DataType;
use datafusion::arrow::util::display::array_value_to_string;
use datafusion::error::Result as DfResult;
use datafusion::functions_aggregate::expr_fn::{avg, sum};
use datafusion::functions_aggregate::first_last::first_value_udaf;
use datafusion::logical_expr::ExprFunctionExt;
use datafusion::prelude::*;
use insight_schema::{NODE_ID, TIMESTAMP, metric as m};
use insight_wire::metric::{
    OverviewPoint as Point, OverviewSeries as Series, OverviewTarget as Target,
};
use serde::Deserialize;

use crate::error::{ErrorKind, Result};
use crate::storage::store::Store;

const SERVICES: [&str; 4] = [
    "cpu_usage",
    "memory_working_set",
    "disk_read_rate",
    "disk_write_rate",
];
const NODES: [&str; 11] = [
    "psi_cpu_some",
    "psi_memory_some",
    "psi_memory_full",
    "psi_io_some",
    "psi_io_full",
    "cpu_executing",
    "memory_used",
    "network_receive_rate",
    "network_transmit_rate",
    "cpu_total",
    "memory_total",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Step {
    step_secs: u32,
}

enum Source {
    Services,
    Nodes,
    Mounts,
}

#[derive(Default)]
struct Accumulator {
    current: Option<Series>,
    score: f64,
    weight: f64,
    top: Vec<(f64, Series)>,
    totals: BTreeMap<jiff::Timestamp, Option<f64>>,
}

fn identity(target: &Target) -> (&str, &str) {
    match target {
        Target::Service { service_id, .. } => (service_id, ""),
        Target::Node { node_id } => (node_id, ""),
        Target::Filesystem {
            node_id,
            mount_point,
        } => (node_id, mount_point),
        _ => ("", ""),
    }
}

impl Accumulator {
    fn finish_current(&mut self) {
        let Some(series) = self.current.take() else {
            return;
        };
        let score = std::mem::take(&mut self.score);
        let weight = std::mem::take(&mut self.weight);
        if series.points.iter().all(|point| point.value.is_none()) {
            return;
        }
        let limit = if matches!(series.target, Target::Filesystem { .. }) {
            10
        } else {
            5
        };
        self.top
            .push((if weight > 0.0 { score / weight } else { score }, series));
        self.top.sort_by(|a, b| {
            b.0.total_cmp(&a.0)
                .then_with(|| identity(&a.1.target).cmp(&identity(&b.1.target)))
        });
        if self.top.len() > limit
            && let Some((_, series)) = self.top.pop()
            && matches!(series.target, Target::Service { .. })
        {
            for point in series.points {
                add(&mut self.totals, &point);
            }
        }
    }

    fn push(&mut self, metric: &str, target: &Target, point: Point, weight: Option<f64>) {
        if matches!(target, Target::Cluster) {
            add(&mut self.totals, &point);
            return;
        }
        if self
            .current
            .as_ref()
            .is_some_and(|series| identity(&series.target) != identity(target))
        {
            self.finish_current();
        }
        if let Some(value) = point.value {
            self.score = if matches!(target, Target::Filesystem { .. }) {
                self.score.max(value)
            } else {
                self.score + value * weight.unwrap_or(1.0)
            };
            self.weight += weight.unwrap_or(0.0);
        }
        let current = self.current.get_or_insert_with(|| Series {
            metric: metric.to_owned(),
            target: target.clone(),
            points: Vec::new(),
        });
        current.target = target.clone();
        current.points.push(point);
    }
}

fn add(totals: &mut BTreeMap<jiff::Timestamp, Option<f64>>, point: &Point) {
    let total = totals.entry(point.timestamp).or_default();
    if let Some(value) = point.value {
        *total.get_or_insert(0.0) += value;
    }
}

fn column<'a, T: Array + 'static>(batch: &'a RecordBatch, name: &str) -> Result<&'a T> {
    batch
        .column_by_name(name)
        .and_then(|array| array.as_any().downcast_ref())
        .ok_or_else(|| {
            ErrorKind::Invariant(format!("overview column {name} has unexpected type")).into()
        })
}

fn number(array: &Float64Array, row: usize) -> Option<f64> {
    (!array.is_null(row)).then(|| array.value(row))
}

fn point(timestamp: jiff::Timestamp, value: Option<f64>) -> Point {
    Point {
        timestamp,
        value,
        used: None,
        total: None,
    }
}

impl Source {
    fn is_pressure(&self, metric: &str) -> bool {
        matches!(self, Self::Nodes) && metric.starts_with("psi_")
    }

    fn metrics(&self) -> &[&str] {
        match self {
            Self::Services => &SERVICES,
            Self::Nodes => &NODES,
            Self::Mounts => &["utilization"],
        }
    }

    fn consume(&self, groups: &mut Vec<Accumulator>, batch: &RecordBatch) -> Result<()> {
        let metrics = self.metrics();
        groups.resize_with(metrics.len(), Accumulator::default);
        let timestamps = column::<TimestampNanosecondArray>(batch, TIMESTAMP)?;
        let values = metrics
            .iter()
            .map(|name| column::<Float64Array>(batch, name))
            .collect::<Result<Vec<_>>>()?;
        let weights = metrics
            .iter()
            .map(|name| {
                self.is_pressure(name)
                    .then(|| column::<Float64Array>(batch, &format!("{name}_weight")))
                    .transpose()
            })
            .collect::<Result<Vec<_>>>()?;
        let filesystem = if matches!(self, Self::Mounts) {
            Some((
                column::<Float64Array>(batch, "used")?,
                column::<Float64Array>(batch, "total")?,
            ))
        } else {
            None
        };
        for row in 0..batch.num_rows() {
            let timestamp = jiff::Timestamp::from_nanosecond(i128::from(timestamps.value(row)))?;
            let text = |name: &str| -> Result<String> {
                let array = batch.column_by_name(name).ok_or_else(|| {
                    ErrorKind::Invariant(format!("overview column {name} is missing"))
                })?;
                Ok(array_value_to_string(array.as_ref(), row)?)
            };
            let target = match self {
                Self::Services => Target::Service {
                    service_id: text(m::SERVICE_ID)?,
                    name: text(m::SERVICE_NAME)?,
                },
                Self::Nodes => Target::Node {
                    node_id: text(NODE_ID)?,
                },
                Self::Mounts => Target::Filesystem {
                    node_id: text(NODE_ID)?,
                    mount_point: text(m::MOUNT_POINT)?,
                },
            };
            for (((accumulator, metric), value), weight) in
                groups.iter_mut().zip(metrics).zip(&values).zip(&weights)
            {
                let mut point = point(timestamp, number(value, row));
                if let Some((used, total)) = filesystem {
                    point.used = number(used, row);
                    point.total = number(total, row);
                }
                accumulator.push(
                    metric,
                    if matches!(self, Self::Nodes) && !self.is_pressure(metric) {
                        &Target::Cluster
                    } else {
                        &target
                    },
                    point,
                    weight.and_then(|array| number(array, row)),
                );
            }
        }
        Ok(())
    }

    fn finish(&self, groups: Vec<Accumulator>) -> Vec<Series> {
        let mut series = Vec::new();
        for (metric, mut accumulator) in self.metrics().iter().zip(groups) {
            accumulator.finish_current();
            series.extend(accumulator.top.into_iter().map(|(_, line)| line));
            if !accumulator.totals.is_empty() {
                series.push(Series {
                    metric: (*metric).to_owned(),
                    target: if matches!(self, Self::Nodes) {
                        Target::Cluster
                    } else {
                        Target::Rest
                    },
                    points: accumulator
                        .totals
                        .into_iter()
                        .map(|(at, value)| point(at, value))
                        .collect(),
                });
            }
        }
        series
    }

    fn plan(&self, df: DataFrame, range: (i64, i64), step: u32) -> DfResult<DataFrame> {
        let elapsed = when(
            col(m::ELAPSED_MS).between(lit(1i64), lit(crate::config::METRIC_ELAPSED_MAX_MS)),
            cast(col(m::ELAPSED_MS), DataType::Float64),
        )
        .end()?;
        let width = lit(f64::from(step) * 1000.0);
        let (df, keys, aggregates) = match self {
            Self::Services => {
                let df =
                    super::metric::container_values(df.filter(col(m::MANAGED).eq(lit(true)))?)?;
                let mut columns = vec![
                    TIMESTAMP,
                    NODE_ID,
                    insight_schema::CONTAINER_ID,
                    m::BOOT_ID,
                    m::CONTAINER_STARTED_AT,
                    m::NODE_UPTIME_MS,
                    m::SERVICE_ID,
                    m::SERVICE_NAME,
                    m::ELAPSED_MS,
                ];
                columns.extend(SERVICES);
                let df = super::metric::deduplicate(df.select_columns(&columns)?)?;
                let mut aggregates = SERVICES
                    .iter()
                    .map(|name| (sum(col(*name) * elapsed.clone()) / width.clone()).alias(*name))
                    .collect::<Vec<_>>();
                aggregates.push(
                    first_value_udaf()
                        .call(vec![col(m::SERVICE_NAME)])
                        .order_by(vec![col(TIMESTAMP).sort(false, false)])
                        .build()?
                        .alias(m::SERVICE_NAME),
                );
                (df, vec![m::SERVICE_ID], aggregates)
            }
            Self::Nodes => {
                let df = super::metric::host_values(df)?;
                let mut aggregates = Vec::new();
                for name in NODES {
                    let total = sum(col(name) * elapsed.clone());
                    if self.is_pressure(name) {
                        let weight = sum(when(col(name).is_not_null(), elapsed.clone()).end()?);
                        aggregates.push((total / weight.clone()).alias(name));
                        aggregates.push(weight.alias(format!("{name}_weight")));
                    } else {
                        aggregates.push((total / width.clone()).alias(name));
                    }
                }
                (df, vec![NODE_ID], aggregates)
            }
            Self::Mounts => {
                let mount = col(m::MOUNT_POINT);
                let df = super::metric::filesystem_values(df)?.filter(
                    mount
                        .clone()
                        .not_eq(lit("/boot"))
                        .and(mount.clone().not_eq(lit("/efi")))
                        .and(mount.clone().not_like(lit("/boot/%")))
                        .and(mount.not_like(lit("/efi/%"))),
                )?;
                (
                    df,
                    vec![NODE_ID, m::MOUNT_POINT],
                    vec![
                        avg(col("utilization")).alias("utilization"),
                        avg(col("used_bytes")).alias("used"),
                        avg(col("total_bytes")).alias("total"),
                    ],
                )
            }
        };
        let df = df.filter(col(TIMESTAMP).gt_eq(lit_timestamp_nano(range.0)))?;
        let groups = keys
            .iter()
            .map(|key| col(*key))
            .chain([super::metric::bucket(step, 0)])
            .collect::<Vec<_>>();
        let sort = keys
            .iter()
            .copied()
            .chain([TIMESTAMP])
            .map(|key| col(key).sort(true, false))
            .collect();
        df.aggregate(groups, aggregates)?.sort(sort)
    }
}

pub(crate) async fn get(
    State(store): State<Arc<Store>>,
    Path(range): Path<(i64, i64)>,
    Query(query): Query<Step>,
) -> Result<Json<Vec<Series>>> {
    super::metric::validate_step(range, query.step_secs)?;
    if range.0 >= range.1 {
        return Ok(Json(Vec::new()));
    }
    let mut series = Vec::new();
    for (source, index) in [
        (Source::Services, insight_wire::CONTAINER_METRIC_INDEX_ID),
        (Source::Nodes, insight_wire::HOST_METRIC_INDEX_ID),
        (Source::Mounts, insight_wire::FILESYSTEM_METRIC_INDEX_ID),
    ] {
        let step = if matches!(source, Source::Mounts) {
            query.step_secs.max(u32::from(
                insight_wire::metric::FILESYSTEM_SAMPLE_INTERVAL_SECS,
            ))
        } else {
            query.step_secs
        };
        let scan = if matches!(source, Source::Nodes) {
            super::metric::window(range, step).0
        } else {
            range
        };
        let groups = store
            .fold(
                index,
                scan,
                |df| source.plan(df, range, step),
                |groups, batch| source.consume(groups, &batch),
            )
            .await?;
        series.extend(source.finish(groups));
    }
    for (host, app) in [
        ("cpu_executing", "cpu_usage"),
        ("memory_used", "memory_working_set"),
    ] {
        let mut managed = BTreeMap::<_, f64>::new();
        for line in series.iter().filter(|line| line.metric == app) {
            for point in &line.points {
                if let Some(value) = point.value {
                    *managed.entry(point.timestamp).or_default() += value;
                }
            }
        }
        if let Some(line) = series.iter_mut().find(|line| line.metric == host) {
            app.clone_into(&mut line.metric);
            line.target = Target::Host;
            for point in &mut line.points {
                point.value = point
                    .value
                    .map(|value| value - managed.get(&point.timestamp).copied().unwrap_or(0.0))
                    .filter(|value| *value >= 0.0);
            }
        }
    }
    Ok(Json(series))
}
