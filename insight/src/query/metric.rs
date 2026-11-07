use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::Response;
use axum_extra::extract::Query;
use datafusion::arrow::datatypes::DataType;
use datafusion::common::ScalarValue;
use datafusion::error::Result as DfResult;
use datafusion::functions::expr_fn::{date_bin, least, named_struct, round};
use datafusion::functions_aggregate::expr_fn::{avg, count, max, min};
use datafusion::functions_window::expr_fn::lag;
use datafusion::logical_expr::ExprFunctionExt;
use datafusion::prelude::*;
use serde::Deserialize;

use crate::error::{ErrorKind, Result};
use crate::storage::store::Store;
use insight_schema::metric::{
    AVAILABLE_BYTES, BOOT_ID, CONTAINER_STARTED_AT, CPU_CORES, CPU_IDLE_TICKS, CPU_IOWAIT_TICKS,
    CPU_PERIOD_USEC, CPU_QUOTA_USEC, CPU_STEAL_TICKS, CPU_SYSTEM_TICKS, CPU_THROTTLED_USEC_DELTA,
    CPU_USAGE_USEC_DELTA, CPU_USER_TICKS, DISK_READ_BYTES_DELTA, DISK_READ_OPS_DELTA,
    DISK_WRITE_BYTES_DELTA, DISK_WRITE_OPS_DELTA, ELAPSED_MS, FREE_BYTES, INODES_FREE,
    INODES_TOTAL, MEMORY_ANON_BYTES, MEMORY_AVAILABLE_BYTES, MEMORY_CURRENT_BYTES,
    MEMORY_FREE_BYTES, MEMORY_INACTIVE_FILE_BYTES, MEMORY_LIMIT_BYTES, MEMORY_OOM_KILLS_DELTA,
    MEMORY_SWAP_CURRENT_BYTES, MEMORY_TOTAL_BYTES, MOUNT_POINT, NETWORK_INTERFACE_SET,
    NETWORK_RECEIVE_BYTES_DELTA, NETWORK_RECEIVE_BYTES_TOTAL, NETWORK_TRANSMIT_BYTES_DELTA,
    NETWORK_TRANSMIT_BYTES_TOTAL, NODE_UPTIME_MS, PIDS_CURRENT, PIDS_MAX, PSI_CPU_SOME_USEC_DELTA,
    PSI_CPU_SOME_USEC_TOTAL, PSI_IO_FULL_USEC_DELTA, PSI_IO_FULL_USEC_TOTAL,
    PSI_IO_SOME_USEC_DELTA, PSI_IO_SOME_USEC_TOTAL, PSI_MEMORY_FULL_USEC_DELTA,
    PSI_MEMORY_FULL_USEC_TOTAL, PSI_MEMORY_SOME_USEC_DELTA, PSI_MEMORY_SOME_USEC_TOTAL, SERVICE_ID,
    SWAP_FREE_BYTES, SWAP_TOTAL_BYTES, TCP_RETRANS_SEGS_DELTA, TOTAL_BYTES,
};
use insight_schema::{CONTAINER_ID, NODE_ID, TIMESTAMP};

const STEP_SECS_MIN: u32 = insight_wire::metric::SAMPLE_INTERVAL_MIN_SECS as u32;
const STEP_SECS_MAX: u32 = insight_wire::metric::STEP_SECS_MAX;
const MAX_BUCKETS: i64 = 750;
const NANOS_PER_SECOND: i64 = 1_000_000_000;
const OBSERVED_AT: &str = "observed_at";
const REPLICAS: &str = "replicas";
const STATS: [&str; 3] = ["min", "avg", "max"];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Node {
    node_id: String,
    step_secs: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Container {
    service_id: String,
    #[expect(clippy::struct_field_names)]
    container_id: Option<String>,
    step_secs: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Instances {
    service_id: String,
}

async fn series(
    store: &Store,
    index_id: &str,
    range: (i64, i64),
    step_secs: u32,
    filter: Option<Expr>,
    plan: fn(DataFrame, u32, i64) -> DfResult<DataFrame>,
) -> Result<Response> {
    validate_step(range, step_secs)?;
    if range.0 >= range.1 {
        return crate::http::batches_to_json_response(&[]);
    }
    let (scan, cutoff) = window(range, step_secs);
    let batches = store
        .aggregate(index_id, scan, filter, |df| plan(df, step_secs, cutoff))
        .await?;
    crate::http::batches_to_json_response(&batches)
}

pub(crate) async fn host(
    State(store): State<Arc<Store>>,
    Path(range): Path<(i64, i64)>,
    Query(query): Query<Node>,
) -> Result<Response> {
    series(
        &store,
        insight_wire::HOST_METRIC_INDEX_ID,
        range,
        query.step_secs,
        Some(col(NODE_ID).eq(lit(query.node_id.as_str()))),
        host_plan,
    )
    .await
}

pub(crate) async fn container(
    State(store): State<Arc<Store>>,
    Path(range): Path<(i64, i64)>,
    Query(query): Query<Container>,
) -> Result<Response> {
    let mut filter = col(SERVICE_ID).eq(lit(query.service_id));
    if let Some(container_id) = query.container_id {
        filter = filter.and(col(CONTAINER_ID).eq(lit(container_id)));
    }
    series(
        &store,
        insight_wire::CONTAINER_METRIC_INDEX_ID,
        range,
        query.step_secs,
        Some(filter),
        container_plan,
    )
    .await
}

pub(crate) async fn instances(
    State(store): State<Arc<Store>>,
    Path(range): Path<(i64, i64)>,
    Query(query): Query<Instances>,
) -> Result<Response> {
    let batches = store
        .aggregate(
            insight_wire::CONTAINER_METRIC_INDEX_ID,
            range,
            Some(col(SERVICE_ID).eq(lit(query.service_id))),
            instances_plan,
        )
        .await?;
    crate::http::batches_to_json_response(&batches)
}

pub(crate) async fn filesystem(
    State(store): State<Arc<Store>>,
    Path(range): Path<(i64, i64)>,
    Query(query): Query<Node>,
) -> Result<Response> {
    series(
        &store,
        insight_wire::FILESYSTEM_METRIC_INDEX_ID,
        range,
        query.step_secs,
        Some(col(NODE_ID).eq(lit(query.node_id.as_str()))),
        filesystem_plan,
    )
    .await
}

pub(crate) async fn node_latest(
    State(store): State<Arc<Store>>,
    Path(range): Path<(i64, i64)>,
) -> Result<Response> {
    let batches = store
        .aggregate(insight_wire::HOST_METRIC_INDEX_ID, range, None, |df| {
            df.aggregate(
                vec![col(NODE_ID)],
                vec![max(col(TIMESTAMP)).alias(OBSERVED_AT)],
            )?
            .sort(vec![col(NODE_ID).sort(true, false)])
        })
        .await?;
    crate::http::batches_to_json_response(&batches)
}

pub(crate) fn validate_step(range: (i64, i64), step_secs: u32) -> Result<()> {
    if !(STEP_SECS_MIN..=STEP_SECS_MAX).contains(&step_secs) {
        return Err(ErrorKind::BadRequest(format!(
            "step_secs({step_secs}) must be within {STEP_SECS_MIN}..={STEP_SECS_MAX}"
        ))
        .into());
    }
    let (start_nanos, end_nanos) = range;
    let step = step_nanos(step_secs);
    let buckets = end_nanos
        .saturating_sub(1)
        .div_euclid(step)
        .saturating_sub(start_nanos.div_euclid(step))
        .saturating_add(1);
    if start_nanos < end_nanos && buckets > MAX_BUCKETS {
        return Err(ErrorKind::BadRequest(format!(
            "time range spans {buckets} buckets at step_secs({step_secs}), limit is {MAX_BUCKETS}"
        ))
        .into());
    }
    Ok(())
}

fn step_nanos(step_secs: u32) -> i64 {
    i64::from(step_secs).max(1).saturating_mul(NANOS_PER_SECOND)
}

pub(super) fn window(range: (i64, i64), step_secs: u32) -> ((i64, i64), i64) {
    let cutoff = range
        .0
        .saturating_sub(range.0.rem_euclid(step_nanos(step_secs)));
    let scan = (
        range.0.saturating_sub(crate::config::METRIC_LOOKBACK_NANOS),
        range.1,
    );
    (scan, cutoff)
}

fn host_plan(df: DataFrame, step_secs: u32, cutoff_nanos: i64) -> DfResult<DataFrame> {
    finish(
        host_window(df)?,
        step_secs,
        &host_families()?,
        None,
        cutoff_nanos,
    )
}

pub(super) fn deduplicate(df: DataFrame) -> DfResult<DataFrame> {
    df.sort(vec![col(TIMESTAMP).sort(false, true)])?.distinct()
}

pub(super) fn host_values(df: DataFrame) -> DfResult<DataFrame> {
    values(host_window(df)?, &host_families()?)?
        .with_column(ELAPSED_MS, when(elapsed_ok(), elapsed()).end()?)?
        .with_column(
            "cpu_executing",
            when(
                col(CPU_CORES).eq(col(format!("prev_{CPU_CORES}"))),
                (col("cpu_user") + col("cpu_system")) * to_f64(col(CPU_CORES)),
            )
            .end()?,
        )
}

fn host_window(df: DataFrame) -> DfResult<DataFrame> {
    deduplicate(df)?.window(vec![
        prev(NODE_UPTIME_MS)?,
        prev(CPU_CORES)?,
        prev(CPU_USER_TICKS)?,
        prev(CPU_SYSTEM_TICKS)?,
        prev(CPU_IDLE_TICKS)?,
        prev(CPU_IOWAIT_TICKS)?,
        prev(CPU_STEAL_TICKS)?,
        prev(PSI_CPU_SOME_USEC_TOTAL)?,
        prev(PSI_MEMORY_SOME_USEC_TOTAL)?,
        prev(PSI_MEMORY_FULL_USEC_TOTAL)?,
        prev(PSI_IO_SOME_USEC_TOTAL)?,
        prev(PSI_IO_FULL_USEC_TOTAL)?,
        prev(NETWORK_INTERFACE_SET)?,
        prev(NETWORK_RECEIVE_BYTES_TOTAL)?,
        prev(NETWORK_TRANSMIT_BYTES_TOTAL)?,
    ])
}

fn host_families() -> DfResult<[(&'static str, Expr, i64); 21]> {
    let ticks_total = delta(CPU_USER_TICKS)
        + delta(CPU_SYSTEM_TICKS)
        + delta(CPU_IDLE_TICKS)
        + delta(CPU_IOWAIT_TICKS)
        + delta(CPU_STEAL_TICKS);
    let ticks_ok = elapsed_ok()
        .and(delta(CPU_USER_TICKS).gt_eq(lit(0i64)))
        .and(delta(CPU_SYSTEM_TICKS).gt_eq(lit(0i64)))
        .and(delta(CPU_IDLE_TICKS).gt_eq(lit(0i64)))
        .and(delta(CPU_IOWAIT_TICKS).gt_eq(lit(0i64)))
        .and(delta(CPU_STEAL_TICKS).gt_eq(lit(0i64)))
        .and(ticks_total.clone().gt(lit(0i64)));
    let tick_ratio = |numerator: Expr| {
        when(
            ticks_ok.clone(),
            to_f64(numerator) / to_f64(ticks_total.clone()),
        )
        .end()
    };
    let cpu_utilization =
        tick_ratio(delta(CPU_USER_TICKS) + delta(CPU_SYSTEM_TICKS) + delta(CPU_STEAL_TICKS))?;
    let unfree = sub_guarded(MEMORY_TOTAL_BYTES, MEMORY_FREE_BYTES)?;
    let memory_used = when(
        to_f64(col(MEMORY_INACTIVE_FILE_BYTES)).lt_eq(unfree.clone()),
        unfree - to_f64(col(MEMORY_INACTIVE_FILE_BYTES)),
    )
    .end()?;
    let memory_utilization = when(
        col(MEMORY_TOTAL_BYTES).gt(lit(0u64)),
        memory_used.clone() / to_f64(col(MEMORY_TOTAL_BYTES)),
    )
    .end()?;
    let swap_used = sub_guarded(SWAP_TOTAL_BYTES, SWAP_FREE_BYTES)?;
    Ok([
        ("cpu_utilization", cpu_utilization.clone(), 4),
        ("cpu_user", tick_ratio(delta(CPU_USER_TICKS))?, 4),
        ("cpu_system", tick_ratio(delta(CPU_SYSTEM_TICKS))?, 4),
        ("cpu_iowait", tick_ratio(delta(CPU_IOWAIT_TICKS))?, 4),
        ("cpu_steal", tick_ratio(delta(CPU_STEAL_TICKS))?, 4),
        ("cpu_used", cpu_utilization * to_f64(col(CPU_CORES)), 4),
        ("cpu_total", to_f64(col(CPU_CORES)), 0),
        ("memory_utilization", memory_utilization, 4),
        ("memory_used", memory_used, 0),
        ("memory_available", to_f64(col(MEMORY_AVAILABLE_BYTES)), 0),
        ("memory_total", to_f64(col(MEMORY_TOTAL_BYTES)), 0),
        ("swap_used", swap_used, 0),
        ("swap_total", to_f64(col(SWAP_TOTAL_BYTES)), 0),
        ("psi_cpu_some", usec_rate(PSI_CPU_SOME_USEC_TOTAL)?, 4),
        ("psi_memory_some", usec_rate(PSI_MEMORY_SOME_USEC_TOTAL)?, 4),
        ("psi_memory_full", usec_rate(PSI_MEMORY_FULL_USEC_TOTAL)?, 4),
        ("psi_io_some", usec_rate(PSI_IO_SOME_USEC_TOTAL)?, 4),
        ("psi_io_full", usec_rate(PSI_IO_FULL_USEC_TOTAL)?, 4),
        (
            "network_receive_rate",
            network_rate(NETWORK_RECEIVE_BYTES_TOTAL)?,
            0,
        ),
        (
            "network_transmit_rate",
            network_rate(NETWORK_TRANSMIT_BYTES_TOTAL)?,
            0,
        ),
        ("node_uptime", to_f64(col(NODE_UPTIME_MS)) / lit(1000.0), 0),
    ])
}

fn filesystem_plan(df: DataFrame, step_secs: u32, cutoff_nanos: i64) -> DfResult<DataFrame> {
    finish(
        deduplicate(df)?,
        step_secs,
        &filesystem_families()?,
        Some(MOUNT_POINT),
        cutoff_nanos,
    )
}

pub(super) fn filesystem_values(df: DataFrame) -> DfResult<DataFrame> {
    values(deduplicate(df)?, &filesystem_families()?)
}

fn filesystem_families() -> DfResult<[(&'static str, Expr, i64); 5]> {
    let used = to_f64(col(TOTAL_BYTES)) - to_f64(col(FREE_BYTES));
    let denominator = used.clone() + to_f64(col(AVAILABLE_BYTES));
    let sample_ok = col(TOTAL_BYTES)
        .gt(lit(0u64))
        .and(col(FREE_BYTES).lt_eq(col(TOTAL_BYTES)))
        .and(col(AVAILABLE_BYTES).lt_eq(col(FREE_BYTES)));
    let utilization_ok = sample_ok.clone().and(denominator.clone().gt(lit(0.0)));
    let utilization = when(utilization_ok, used.clone() / denominator).end()?;
    let used_bytes = when(sample_ok.clone(), used).end()?;
    let available_bytes = when(sample_ok.clone(), to_f64(col(AVAILABLE_BYTES))).end()?;
    let total_bytes = when(sample_ok, to_f64(col(TOTAL_BYTES))).end()?;
    let [
        utilization_name,
        used_name,
        available_name,
        total_name,
        inodes_name,
    ] = insight_wire::metric::FilesystemBucket::FAMILIES;
    Ok([
        (utilization_name, utilization, 4),
        (used_name, used_bytes, 0),
        (available_name, available_bytes, 0),
        (total_name, total_bytes, 0),
        (inodes_name, capacity(INODES_TOTAL, INODES_FREE)?, 4),
    ])
}

fn values(df: DataFrame, families: &[(&str, Expr, i64)]) -> DfResult<DataFrame> {
    let computed = families
        .iter()
        .map(|(name, expr, _)| expr.clone().alias(*name));
    let projection = df
        .schema()
        .fields()
        .iter()
        .filter(|field| !families.iter().any(|(name, _, _)| *name == field.name()))
        .map(|field| col(field.name()))
        .chain(computed)
        .collect::<Vec<_>>();
    df.select(projection)
}

fn container_plan(df: DataFrame, step_secs: u32, cutoff_nanos: i64) -> DfResult<DataFrame> {
    finish_container(
        deduplicate(df)?,
        step_secs,
        &container_families()?,
        cutoff_nanos,
    )
}

pub(super) fn container_values(df: DataFrame) -> DfResult<DataFrame> {
    values(df, &container_families()?)
}

fn container_families() -> DfResult<Vec<(&'static str, Expr, i64)>> {
    let cpu_usage = interval_rate(CPU_USAGE_USEC_DELTA, 0.001)?;
    let cpu_limit = when(
        col(CPU_QUOTA_USEC)
            .gt(lit(0u64))
            .and(col(CPU_PERIOD_USEC).gt(lit(0u64))),
        to_f64(col(CPU_QUOTA_USEC)) / to_f64(col(CPU_PERIOD_USEC)),
    )
    .end()?;
    let cpu_utilization = cpu_usage.clone() / cpu_limit.clone();
    let throttled = interval_rate(CPU_THROTTLED_USEC_DELTA, 0.001)?;
    let cpu_throttled = when(
        throttled.clone().is_not_null(),
        least(vec![throttled, lit(1.0)]),
    )
    .end()?;
    let memory_working_set = sub_guarded(MEMORY_CURRENT_BYTES, MEMORY_INACTIVE_FILE_BYTES)?;
    let memory_utilization = when(
        col(MEMORY_LIMIT_BYTES).gt(lit(0u64)),
        memory_working_set.clone() / to_f64(col(MEMORY_LIMIT_BYTES)),
    )
    .end()?;
    let mut families = vec![
        ("cpu_utilization", cpu_utilization, 4),
        ("cpu_usage", cpu_usage, 4),
        ("cpu_limit", cpu_limit, 4),
        ("cpu_throttled", cpu_throttled, 4),
        ("memory_utilization", memory_utilization, 4),
        ("memory_working_set", memory_working_set, 0),
        ("memory_current", to_f64(col(MEMORY_CURRENT_BYTES)), 0),
        ("memory_anon", to_f64(col(MEMORY_ANON_BYTES)), 0),
        ("memory_limit", to_f64(col(MEMORY_LIMIT_BYTES)), 0),
        ("memory_swap", to_f64(col(MEMORY_SWAP_CURRENT_BYTES)), 0),
        ("pids", to_f64(col(PIDS_CURRENT)), 0),
        ("pids_limit", to_f64(col(PIDS_MAX)), 0),
    ];
    for (name, counter) in [
        ("psi_cpu_some", PSI_CPU_SOME_USEC_DELTA),
        ("psi_memory_some", PSI_MEMORY_SOME_USEC_DELTA),
        ("psi_memory_full", PSI_MEMORY_FULL_USEC_DELTA),
        ("psi_io_some", PSI_IO_SOME_USEC_DELTA),
        ("psi_io_full", PSI_IO_FULL_USEC_DELTA),
    ] {
        families.push((name, interval_rate(counter, 0.001)?, 4));
    }
    for (name, counter, decimals) in [
        ("oom_kill_rate", MEMORY_OOM_KILLS_DELTA, 4),
        ("disk_read_rate", DISK_READ_BYTES_DELTA, 0),
        ("disk_write_rate", DISK_WRITE_BYTES_DELTA, 0),
        ("disk_read_iops", DISK_READ_OPS_DELTA, 0),
        ("disk_write_iops", DISK_WRITE_OPS_DELTA, 0),
        ("network_receive_rate", NETWORK_RECEIVE_BYTES_DELTA, 0),
        ("network_transmit_rate", NETWORK_TRANSMIT_BYTES_DELTA, 0),
        ("tcp_retrans_rate", TCP_RETRANS_SEGS_DELTA, 4),
    ] {
        families.push((name, interval_rate(counter, 1000.0)?, decimals));
    }
    Ok(families)
}

fn finish(
    windowed: DataFrame,
    step_secs: u32,
    families: &[(&str, Expr, i64)],
    label: Option<&str>,
    cutoff_nanos: i64,
) -> DfResult<DataFrame> {
    let aggregates: Vec<Expr> = families
        .iter()
        .flat_map(|(name, expr, decimals)| stats(name, |_| expr.clone(), Some(*decimals)))
        .collect();
    let mut group_by = vec![bucket(step_secs, 0)];
    let mut projection = vec![col(TIMESTAMP)];
    let mut sort = vec![col(TIMESTAMP).sort(true, false)];
    if let Some(label) = label {
        group_by.push(col(label));
        projection.push(col(label));
        sort.push(col(label).sort(true, false));
    }
    projection.extend(families.iter().map(|(name, _, _)| nested(name)));
    windowed
        .aggregate(group_by, aggregates)?
        .select(projection)?
        .filter(col(TIMESTAMP).gt_eq(lit_timestamp_nano(cutoff_nanos)))?
        .sort(sort)
}

fn finish_container(
    windowed: DataFrame,
    step_secs: u32,
    families: &[(&str, Expr, i64)],
    cutoff_nanos: i64,
) -> DfResult<DataFrame> {
    let instance = windowed.aggregate(
        vec![
            bucket(step_secs, 0),
            col(BOOT_ID),
            col(CONTAINER_ID),
            col(CONTAINER_STARTED_AT),
        ],
        families
            .iter()
            .flat_map(|(name, expr, _)| stats(name, |_| expr.clone(), None))
            .collect(),
    )?;
    let mut aggregates = vec![count(col(CONTAINER_ID)).alias(REPLICAS)];
    aggregates.extend(families.iter().flat_map(|(name, _, decimals)| {
        stats(
            name,
            |suffix| col(format!("{name}_{suffix}")),
            Some(*decimals),
        )
    }));
    let mut projection = vec![col(TIMESTAMP), col(REPLICAS)];
    projection.extend(families.iter().map(|(name, _, _)| nested(name)));
    instance
        .aggregate(vec![col(TIMESTAMP)], aggregates)?
        .select(projection)?
        .filter(col(TIMESTAMP).gt_eq(lit_timestamp_nano(cutoff_nanos)))?
        .sort(vec![col(TIMESTAMP).sort(true, false)])
}

fn nested(name: &str) -> Expr {
    let fields = STATS
        .iter()
        .flat_map(|suffix| [lit(*suffix), col(format!("{name}_{suffix}"))])
        .collect();
    named_struct(fields).alias(name)
}

fn instances_plan(df: DataFrame) -> DfResult<DataFrame> {
    df.aggregate(
        vec![col(CONTAINER_ID), col(CONTAINER_STARTED_AT), col(NODE_ID)],
        vec![],
    )
}

pub(super) fn bucket(step_secs: u32, origin_nanos: i64) -> Expr {
    date_bin(
        lit(ScalarValue::new_interval_mdn(0, 0, step_nanos(step_secs))),
        col(TIMESTAMP),
        lit_timestamp_nano(origin_nanos),
    )
    .alias(TIMESTAMP)
}

fn prev(column: &str) -> DfResult<Expr> {
    lag(col(column), Some(1), None)
        .partition_by(vec![col(NODE_ID), col(BOOT_ID)])
        .order_by(vec![col(TIMESTAMP).sort(true, true)])
        .build()
        .map(|expr| expr.alias(format!("prev_{column}")))
}

fn delta(column: &str) -> Expr {
    try_cast(col(column), DataType::Int64)
        - try_cast(col(format!("prev_{column}")), DataType::Int64)
}

fn elapsed() -> Expr {
    delta(NODE_UPTIME_MS)
}

fn elapsed_ok() -> Expr {
    elapsed().between(lit(1i64), lit(crate::config::METRIC_ELAPSED_MAX_MS))
}

fn to_f64(expr: Expr) -> Expr {
    cast(expr, DataType::Float64)
}

fn counter_ok(counter: &str) -> Expr {
    elapsed_ok().and(delta(counter).gt_eq(lit(0i64)))
}

fn usec_per_elapsed(counter: &str) -> Expr {
    to_f64(delta(counter)) / (to_f64(elapsed()) * lit(1000.0))
}

fn network_rate(counter: &str) -> DfResult<Expr> {
    when(
        counter_ok(counter)
            .and(col(NETWORK_INTERFACE_SET).eq(col(format!("prev_{NETWORK_INTERFACE_SET}")))),
        to_f64(delta(counter)) * lit(1000.0) / to_f64(elapsed()),
    )
    .end()
}

fn usec_rate(counter: &str) -> DfResult<Expr> {
    when(counter_ok(counter), usec_per_elapsed(counter)).end()
}

fn interval_rate(delta: &str, multiplier: f64) -> DfResult<Expr> {
    when(
        col(ELAPSED_MS).between(lit(1i64), lit(crate::config::METRIC_ELAPSED_MAX_MS)),
        to_f64(col(delta)) * lit(multiplier) / to_f64(col(ELAPSED_MS)),
    )
    .end()
}

fn sub_guarded(whole: &str, part: &str) -> DfResult<Expr> {
    when(
        col(part).lt_eq(col(whole)),
        to_f64(try_cast(col(whole), DataType::Int64))
            - to_f64(try_cast(col(part), DataType::Int64)),
    )
    .end()
}

fn capacity(total: &str, available: &str) -> DfResult<Expr> {
    let guard = col(total)
        .gt(lit(0u64))
        .and(col(available).lt_eq(col(total)));
    let used = try_cast(col(total), DataType::Int64) - try_cast(col(available), DataType::Int64);
    when(guard, to_f64(used) / to_f64(col(total))).end()
}

fn stats(name: &str, value: impl Fn(&str) -> Expr, decimals: Option<i64>) -> Vec<Expr> {
    STATS
        .iter()
        .zip([min as fn(Expr) -> Expr, avg, max])
        .map(|(suffix, aggregate)| {
            let aggregated = aggregate(value(suffix));
            let aggregated = match decimals {
                Some(decimals) => round(vec![aggregated, lit(decimals)]),
                None => aggregated,
            };
            aggregated.alias(format!("{name}_{suffix}"))
        })
        .collect()
}
