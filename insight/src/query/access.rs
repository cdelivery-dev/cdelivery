use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::Response;
use axum_extra::extract::Query;
use datafusion::error::Result as DfResult;
use datafusion::functions_aggregate::count::count_all;
use datafusion::functions_aggregate::expr_fn::{approx_percentile_cont, count, max, sum};
use datafusion::logical_expr::ExprFunctionExt;
use datafusion::prelude::*;
use pingress_wire::access::{Filter, INDEX_ID, Outcome, StatusClass};
use serde::Deserialize;
use strum::VariantArray;

use super::Page;
use crate::error::Result;
use crate::storage::store::Store;
use insight_schema::{NODE_ID, access};

fn filter_expr(filter: &Filter) -> Option<Expr> {
    let fields = [
        (NODE_ID, filter.node_id.as_deref().map(lit)),
        (access::DOMAIN, filter.domain.as_deref().map(lit)),
        (
            access::UNMATCHED_HOST,
            filter.unmatched_host.as_deref().map(lit),
        ),
        (access::CLIENT_IP, filter.client_ip.as_deref().map(lit)),
        (access::METHOD, filter.method.map(|m| lit(m.as_str()))),
        (access::PATH, filter.path.as_deref().map(lit)),
        (access::STATUS, filter.status.map(lit)),
        (access::OUTCOME, filter.outcome.map(|o| lit(o.as_str()))),
        (access::APP_NAME, filter.app_name.as_deref().map(lit)),
        (access::TARGET_PORT, filter.target_port.map(lit)),
        (access::SERVICE_ID, filter.service_id.as_deref().map(lit)),
        (access::ERROR, filter.error.as_deref().map(lit)),
    ];
    super::eq_all(fields)
        .chain(filter.status_class.map(class_expr))
        .chain(
            filter
                .search_path
                .as_deref()
                .map(|needle| contains(lower(col(access::PATH)), lit(needle.to_lowercase()))),
        )
        .chain(
            filter
                .search_client_ip
                .as_deref()
                .map(|needle| contains(col(access::CLIENT_IP), lit(needle.to_ascii_lowercase()))),
        )
        .reduce(Expr::and)
}

fn class_expr(class: StatusClass) -> Expr {
    match class.bounds() {
        None => col(access::STATUS).is_null(),
        Some((low, None)) => col(access::STATUS).gt_eq(lit(low)),
        Some((low, Some(high))) => col(access::STATUS)
            .gt_eq(lit(low))
            .and(col(access::STATUS).lt(lit(high))),
    }
}

#[derive(Deserialize)]
pub(crate) struct Step {
    step_secs: u32,
}

pub(crate) async fn list(
    State(store): State<Arc<Store>>,
    Path(range): Path<(i64, i64)>,
    Query(page): Query<Page>,
    Query(filter): Query<Filter>,
) -> Result<Response> {
    super::search(&store, INDEX_ID, range, filter_expr(&filter), page).await
}

pub(crate) async fn stats(
    State(store): State<Arc<Store>>,
    Path(range): Path<(i64, i64)>,
    Query(step): Query<Step>,
    Query(filter): Query<Filter>,
) -> Result<Response> {
    stat_response(&store, range, step.step_secs, &filter, stats_plan).await
}

pub(crate) async fn status_stats(
    State(store): State<Arc<Store>>,
    Path(range): Path<(i64, i64)>,
    Query(step): Query<Step>,
    Query(filter): Query<Filter>,
) -> Result<Response> {
    stat_response(&store, range, step.step_secs, &filter, status_plan).await
}

async fn stat_response(
    store: &Store,
    range: (i64, i64),
    step_secs: u32,
    filter: &Filter,
    plan: fn(DataFrame, u32, i64) -> DfResult<DataFrame>,
) -> Result<Response> {
    super::metric::validate_step(range, step_secs)?;
    let batches = store
        .aggregate(INDEX_ID, range, filter_expr(filter), |df| {
            plan(df, step_secs, range.0)
        })
        .await?;
    crate::http::batches_to_json_response(&batches)
}

fn percentile(column: &str, fraction: f64, alias: &str) -> Expr {
    approx_percentile_cont(col(column).sort(true, false), lit(fraction), None).alias(alias)
}

fn count_where(predicate: Expr, alias: &str) -> DfResult<Expr> {
    Ok(count_all()
        .unalias()
        .filter(predicate)
        .build()?
        .alias(alias))
}

fn stats_plan(df: DataFrame, step_secs: u32, start_nanos: i64) -> DfResult<DataFrame> {
    let mut aggregates = vec![
        count_all().alias("count"),
        sum(col(access::REQUEST_BODY_BYTES)).alias("request_bytes"),
        sum(col(access::RESPONSE_BODY_BYTES)).alias("response_bytes"),
        percentile(access::DURATION_US, 0.5, "duration_p50"),
        percentile(access::DURATION_US, 0.95, "duration_p95"),
        percentile(access::DURATION_US, 0.99, "duration_p99"),
        max(col(access::DURATION_US)).alias("duration_max"),
        count(col(access::UPSTREAM_TTFB_US)).alias("ttfb_count"),
        percentile(access::UPSTREAM_TTFB_US, 0.5, "ttfb_p50"),
        percentile(access::UPSTREAM_TTFB_US, 0.95, "ttfb_p95"),
        percentile(access::UPSTREAM_TTFB_US, 0.99, "ttfb_p99"),
        max(col(access::UPSTREAM_TTFB_US)).alias("ttfb_max"),
    ];
    for class in StatusClass::VARIANTS {
        aggregates.push(count_where(class_expr(*class), class.as_str())?);
    }
    for outcome in Outcome::VARIANTS {
        let predicate = col(access::OUTCOME).eq(lit(outcome.as_str()));
        aggregates.push(count_where(predicate, outcome.as_str())?);
    }
    df.aggregate(
        vec![super::metric::bucket(step_secs, start_nanos)],
        aggregates,
    )
}

fn status_plan(df: DataFrame, step_secs: u32, start_nanos: i64) -> DfResult<DataFrame> {
    df.aggregate(
        vec![
            super::metric::bucket(step_secs, start_nanos),
            col(access::STATUS),
        ],
        vec![count_all().alias("count")],
    )
}
