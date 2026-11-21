use std::collections::BTreeMap;
use std::sync::Arc;

use askama::Template;
use axum::extract::{OriginalUri, State};
use axum::response::Response;
use http::StatusCode;
use insight_wire::metric::{FilesystemBucket, HostBucket, ZfsDatasetBucket, ZfsPoolBucket};
use jiff::Timestamp;

use crate::locale::tt;
use crate::{config, operation};

#[derive(Template)]
#[template(path = "node/metrics.html")]
struct Page {
    dashboard: super::super::dashboard::Dashboard,
    id: String,
    hostname: String,
    window: super::super::range::Window,
    state: super::super::metrics::MetricsState,
}

const DETAIL_PAIRS: &[(&str, &str)] = &[
    ("cpu_utilization", "cpu_used"),
    ("memory_utilization", "memory_used"),
];

const CPU: &[&str] = &[
    "cpu_utilization",
    "cpu_user",
    "cpu_system",
    "cpu_iowait",
    "cpu_steal",
    "cpu_used",
    "cpu_total",
    "psi_cpu_some",
];

const MEMORY: &[&str] = &[
    "memory_utilization",
    "memory_used",
    "memory_available",
    "memory_total",
    "swap_used",
    "swap_total",
    "psi_memory_some",
    "psi_memory_full",
];

const DISK: &[&str] = &["psi_io_some", "psi_io_full"];

const NETWORK: &[&str] = &["network_receive_rate", "network_transmit_rate"];

const SYSTEM: &[&str] = &["node_uptime"];

#[expect(clippy::too_many_lines)]
fn charts(
    step_secs: u32,
    host: &[HostBucket],
    filesystems: &[FilesystemBucket],
    pools: &[ZfsPoolBucket],
    datasets: &[ZfsDatasetBucket],
) -> Vec<super::super::metrics::Group> {
    let builder = super::super::metrics::ChartBuilder {
        step_secs,
        points: host,
        at: |p| p.timestamp,
        series: |p| &p.series,
        detail_pairs: DETAIL_PAIRS,
        replicas: None,
    };
    let mut storage = filesystem_charts(step_secs, filesystems);
    storage.extend(builder.charts(DISK));
    let mut groups = super::super::metrics::groups! {
        "cpu" => (tt!("ui.metrics.group.cpu"), builder.charts(CPU));
        "memory" => (tt!("ui.metrics.group.memory"), builder.charts(MEMORY));
        "storage" => (tt!("ui.node.metrics.group.storage"), storage);
        "network" => (tt!("ui.metrics.group.network"), builder.charts(NETWORK));
        "system" => (tt!("ui.node.metrics.group.system"), builder.charts(SYSTEM));
    };
    let mut by_pool: BTreeMap<&str, Vec<&ZfsPoolBucket>> = BTreeMap::new();
    for bucket in pools {
        by_pool.entry(&bucket.pool).or_default().push(bucket);
    }
    for (pool, buckets) in by_pool {
        let charts = [
            (
                tt!("ui.metrics.zfs.utilization"),
                "percent",
                (|b| &b.utilization) as fn(&ZfsPoolBucket) -> &insight_wire::metric::Stat,
            ),
            (tt!("ui.metrics.zfs.size"), "bytes", |b: &ZfsPoolBucket| {
                &b.size_bytes
            }),
            (
                tt!("ui.metrics.zfs.allocated"),
                "bytes",
                |b: &ZfsPoolBucket| &b.allocated_bytes,
            ),
            (tt!("ui.metrics.zfs.free"), "bytes", |b: &ZfsPoolBucket| {
                &b.free_bytes
            }),
        ]
        .into_iter()
        .filter_map(|(title, unit, stat)| {
            zfs_chart(&buckets, step_secs, title, unit, |b| b.timestamp, stat)
        })
        .collect::<Vec<_>>();
        if !charts.is_empty() {
            groups.push(super::super::metrics::Group {
                key: format!("zfs-pool:{pool}").into(),
                title: tt!("ui.metrics.zfs.pool", name = pool),
                charts,
            });
        }
    }
    let mut by_dataset: BTreeMap<(&str, &str), Vec<&ZfsDatasetBucket>> = BTreeMap::new();
    for bucket in datasets {
        by_dataset
            .entry((&bucket.dataset, bucket.dataset_type.as_str()))
            .or_default()
            .push(bucket);
    }
    for ((dataset, kind), buckets) in by_dataset {
        let charts = [
            (
                tt!("ui.metrics.zfs.used"),
                (|b| &b.used_bytes) as fn(&ZfsDatasetBucket) -> &insight_wire::metric::Stat,
            ),
            (tt!("ui.metrics.zfs.available"), |b: &ZfsDatasetBucket| {
                &b.available_bytes
            }),
            (tt!("ui.metrics.zfs.referenced"), |b: &ZfsDatasetBucket| {
                &b.referenced_bytes
            }),
            (tt!("ui.metrics.zfs.volsize"), |b: &ZfsDatasetBucket| {
                &b.volsize_bytes
            }),
            (
                tt!("ui.metrics.zfs.usedbydataset"),
                |b: &ZfsDatasetBucket| &b.usedbydataset_bytes,
            ),
            (
                tt!("ui.metrics.zfs.usedbysnapshots"),
                |b: &ZfsDatasetBucket| &b.usedbysnapshots_bytes,
            ),
            (
                tt!("ui.metrics.zfs.usedbychildren"),
                |b: &ZfsDatasetBucket| &b.usedbychildren_bytes,
            ),
            (
                tt!("ui.metrics.zfs.usedbyrefreservation"),
                |b: &ZfsDatasetBucket| &b.usedbyrefreservation_bytes,
            ),
            (tt!("ui.metrics.zfs.quota"), |b: &ZfsDatasetBucket| {
                &b.quota_bytes
            }),
            (tt!("ui.metrics.zfs.refquota"), |b: &ZfsDatasetBucket| {
                &b.refquota_bytes
            }),
            (tt!("ui.metrics.zfs.reservation"), |b: &ZfsDatasetBucket| {
                &b.reservation_bytes
            }),
            (
                tt!("ui.metrics.zfs.refreservation"),
                |b: &ZfsDatasetBucket| &b.refreservation_bytes,
            ),
        ]
        .into_iter()
        .filter_map(|(title, stat)| {
            zfs_chart(&buckets, step_secs, title, "bytes", |b| b.timestamp, stat)
        })
        .collect::<Vec<_>>();
        if !charts.is_empty() {
            groups.push(super::super::metrics::Group {
                key: format!("zfs-dataset:{dataset}:{kind}").into(),
                title: format!("{dataset} ({kind})").into(),
                charts,
            });
        }
    }
    groups
}

fn zfs_chart<B>(
    buckets: &[&B],
    step_secs: u32,
    title: std::borrow::Cow<'static, str>,
    unit: &'static str,
    at: fn(&B) -> Timestamp,
    stat: fn(&B) -> &insight_wire::metric::Stat,
) -> Option<super::super::metrics::Chart> {
    let data = super::super::metrics::rows(
        step_secs,
        super::super::metrics::filesystem_gap(step_secs),
        buckets.iter().copied(),
        |bucket| at(bucket).as_second(),
        |bucket| {
            Some((
                super::super::metrics::stat_values(stat(bucket))?,
                super::super::metrics::Extras::default(),
            ))
        },
    )?;
    Some(super::super::metrics::Chart {
        title,
        unit,
        data,
        ..Default::default()
    })
}

fn filesystem_charts(
    step_secs: u32,
    filesystems: &[FilesystemBucket],
) -> Vec<Option<super::super::metrics::Chart>> {
    let mut by_mount: BTreeMap<&str, Vec<&FilesystemBucket>> = BTreeMap::new();
    for bucket in filesystems {
        by_mount
            .entry(bucket.mount_point.as_str())
            .or_default()
            .push(bucket);
    }
    let gap_secs = super::super::metrics::filesystem_gap(step_secs);
    let mut charts = Vec::new();
    for (mount, buckets) in &by_mount {
        charts.push(fs_chart(
            tt!("ui.node.metrics.filesystem.utilization", mount = mount),
            buckets,
            step_secs,
            gap_secs,
            |bucket| &bucket.utilization,
            true,
        ));
        charts.push(fs_chart(
            tt!("ui.node.metrics.filesystem.inodes", mount = mount),
            buckets,
            step_secs,
            gap_secs,
            |bucket| &bucket.inodes_utilization,
            false,
        ));
    }
    charts
}

fn fs_chart(
    title: std::borrow::Cow<'static, str>,
    buckets: &[&FilesystemBucket],
    step_secs: u32,
    gap_secs: i64,
    stat: fn(&FilesystemBucket) -> &insight_wire::metric::Stat,
    detailed: bool,
) -> Option<super::super::metrics::Chart> {
    let data = super::super::metrics::rows(
        step_secs,
        gap_secs,
        buckets,
        |bucket| bucket.timestamp.as_second(),
        |bucket| {
            let extras = if detailed {
                super::super::metrics::Extras {
                    detail: bucket.used_bytes.avg,
                    total: bucket.total_bytes.avg,
                    ..Default::default()
                }
            } else {
                super::super::metrics::Extras::default()
            };
            Some((super::super::metrics::stat_values(stat(bucket))?, extras))
        },
    )?;
    Some(super::super::metrics::Chart {
        title,
        unit: "percent",
        detail_unit: detailed.then_some("bytes"),
        detail_label: detailed.then(|| tt!("ui.node.metrics.filesystem.used")),
        total_label: detailed.then(|| tt!("ui.node.metrics.filesystem.total")),
        data,
        ..Default::default()
    })
}

pub(super) fn routes() -> axum::Router<Arc<crate::boot::App>> {
    axum::Router::new().route("/nodes/{id}/metrics", axum::routing::get(get))
}

async fn get(
    authorized: super::super::auth::Authorized<operation::node::Metrics>,
    State(app): State<Arc<crate::boot::App>>,
    axum::extract::Path(id): axum::extract::Path<String>,
    OriginalUri(uri): OriginalUri,
) -> super::super::response::Result<Response> {
    let now = Timestamp::now();
    let node = operation::execute::<operation::node::Get>(
        &app,
        authorized.access(),
        operation::input::Id { id },
        now,
    )
    .await?;
    let window = super::super::range::Window::page(&uri, now)?;
    let input = operation::node::MetricsInput {
        id: node.id.clone(),
        query: operation::typ::metric::RangeQuery {
            start: Some(window.range.0),
            end: Some(window.range.1),
            max_buckets: Some(config::UI_BUCKETS),
        },
    };
    let (dashboard, output) = tokio::try_join!(
        super::super::dashboard::Dashboard::load(
            &app,
            authorized.access(),
            super::super::dashboard::Location::Nodes,
        ),
        super::super::insight::optional_query(operation::execute::<operation::node::Metrics>(
            &app,
            authorized.access(),
            input,
            now,
        )),
    )?;
    let state = super::super::metrics::state(output.map(|output| {
        charts(
            output.step_secs,
            &output.host,
            &output.filesystems,
            &output.zfs_pools,
            &output.zfs_datasets,
        )
    }));
    super::super::response::render(
        &Page {
            dashboard,
            id: node.id,
            hostname: node.hostname,
            window,
            state,
        },
        StatusCode::OK,
    )
}
