use std::borrow::Cow;
use std::collections::BTreeMap;

use jiff::Timestamp;
use pingress_wire::access::StatusClass;
use strum::VariantArray;

use crate::locale::tt;
use crate::{concept, operation};

struct Slot<'a> {
    at: Timestamp,
    bucket: Option<&'a operation::pingress::typ::AccessBucket>,
}

fn slots(stats: &operation::pingress::typ::AccessStats) -> Vec<Slot<'_>> {
    let step = jiff::SignedDuration::from_secs(i64::from(stats.step_secs));
    let end = stats.end;
    let by_start: BTreeMap<_, _> = stats.buckets.iter().map(|b| (b.start, b)).collect();
    std::iter::successors(Some(stats.start), move |at| {
        Some(crate::config::expiry(*at, step))
    })
    .take_while(move |at| *at < end)
    .map(|at| Slot {
        at,
        bucket: by_start.get(&at).copied(),
    })
    .collect()
}

fn chart(
    slots: &[Slot],
    step_secs: u32,
    title: Cow<'static, str>,
    unit: &'static str,
    stat_labels: Option<&'static str>,
    values_at: impl Fn(&Slot) -> Option<([f64; 3], Option<f64>)>,
) -> Option<super::Chart> {
    let data = super::rows(
        step_secs,
        i64::from(step_secs),
        slots,
        |slot| slot.at.as_second(),
        |slot| {
            values_at(slot).map(|(values, detail)| {
                (
                    values,
                    super::Extras {
                        detail,
                        ..Default::default()
                    },
                )
            })
        },
    )?;
    Some(super::Chart {
        title,
        unit,
        stat_labels,
        data,
        ..Default::default()
    })
}

fn filled_chart(
    slots: &[Slot],
    step_secs: u32,
    title: Cow<'static, str>,
    unit: &'static str,
    value: fn(&operation::pingress::typ::AccessBucket) -> f64,
) -> Option<super::Chart> {
    chart(slots, step_secs, title, unit, None, |slot| {
        Some(([slot.bucket.map_or(0.0, value); 3], None))
    })
}

pub(in crate::ui) fn charts(stats: &operation::pingress::typ::AccessStats) -> Vec<super::Group> {
    if stats.buckets.is_empty() {
        return Vec::new();
    }
    let slots = slots(stats);
    let step = stats.step_secs;
    let qps = filled_chart(&slots, step, tt!("ui.metrics.chart.qps"), "events", |b| {
        b.aggregate.qps
    });
    let Some(statuses) = stats.by_status.as_deref() else {
        return super::groups! {
            "pingress-requests" => (tt!("ui.metrics.group.pingressRequests"), [qps]);
        };
    };
    let errors = chart(
        &slots,
        step,
        tt!("ui.metrics.chart.errorRatio"),
        "percent",
        None,
        |slot| {
            let bucket = slot.bucket?;
            Some((
                [class_ratio(bucket, StatusClass::ServerError); 3],
                Some(class_ratio(bucket, StatusClass::ClientError)),
            ))
        },
    )
    .map(|chart| super::Chart {
        detail_unit: Some("percent"),
        detail_label: Some(tt!("ui.metrics.chart.errorRatioDetail")),
        ..chart
    });
    let duration = duration_chart(&slots, step);
    let ttfb = chart(
        &slots,
        step,
        tt!("ui.metrics.chart.accessTtfb"),
        "duration",
        Some("p50,p95,p99"),
        |slot| percentiles(&slot.bucket?.aggregate.upstream_ttfb_us),
    );
    let request_rate = filled_chart(
        &slots,
        step,
        tt!("ui.metrics.chart.requestRate"),
        "bytes-rate",
        |b| concept::pingress::rate(b.aggregate.request_bytes, b.end.duration_since(b.start)),
    );
    let response_rate = filled_chart(
        &slots,
        step,
        tt!("ui.metrics.chart.responseRate"),
        "bytes-rate",
        |b| concept::pingress::rate(b.aggregate.response_bytes, b.end.duration_since(b.start)),
    );
    super::groups! {
        "pingress-requests" => (tt!("ui.metrics.group.pingressRequests"), [qps, errors]);
        "pingress-latency" => (tt!("ui.metrics.group.pingressLatency"), [duration, ttfb]);
        "pingress-traffic" => (tt!("ui.metrics.group.pingressTraffic"), [request_rate, response_rate]);
        "pingress-status" => (
            tt!("ui.metrics.group.pingressStatus"),
            statuses
                .iter()
                .map(|series| status_chart(&slots, step, series))
        );
    }
}

fn status_chart(
    slots: &[Slot],
    step_secs: u32,
    series: &operation::pingress::typ::StatusSeries,
) -> Option<super::Chart> {
    let by_start: BTreeMap<_, _> = series.buckets.iter().map(|b| (b.start, b.qps)).collect();
    let title = if let Some(code) = series.status {
        Cow::Owned(code.to_string())
    } else {
        tt!("ui.metrics.chart.noStatus")
    };
    chart(slots, step_secs, title, "events", None, |slot| {
        let qps = by_start.get(&slot.at).copied().unwrap_or_default();
        Some(([qps; 3], None))
    })
}

pub(in crate::ui) fn class_ratio(
    bucket: &operation::pingress::typ::AccessBucket,
    class: StatusClass,
) -> f64 {
    bucket
        .status_classes
        .iter()
        .find(|entry| entry.status_class == class)
        .map_or(0.0, |entry| entry.ratio)
}

fn percentiles(
    distribution: &operation::pingress::typ::Distribution,
) -> Option<([f64; 3], Option<f64>)> {
    let p = [distribution.p50?, distribution.p95?, distribution.p99?];
    Some((p.map(|micros| micros / 1e6), None))
}

fn duration_chart(slots: &[Slot], step_secs: u32) -> Option<super::Chart> {
    chart(
        slots,
        step_secs,
        tt!("ui.metrics.chart.accessDuration"),
        "duration",
        Some("p50,p95,p99"),
        |slot| percentiles(&slot.bucket?.aggregate.duration_us),
    )
}

pub(in crate::ui) fn overview_qps(
    stats: &operation::pingress::typ::AccessStats,
) -> Option<(Vec<&'static str>, String)> {
    let slots = slots(stats);
    let classes: Vec<StatusClass> = StatusClass::VARIANTS
        .iter()
        .copied()
        .filter(|class| {
            stats
                .buckets
                .iter()
                .any(|bucket| class_ratio(bucket, *class) > 0.0)
        })
        .collect();
    if classes.is_empty() {
        None
    } else {
        super::rows(
            stats.step_secs,
            i64::from(stats.step_secs),
            &slots,
            |slot| slot.at.as_second(),
            |slot| {
                let values: Vec<f64> = classes
                    .iter()
                    .map(|class| {
                        slot.bucket.map_or(0.0, |bucket| {
                            bucket.aggregate.qps * class_ratio(bucket, *class)
                        })
                    })
                    .collect();
                Some((values, super::Extras::default()))
            },
        )
        .map(|data| (classes.iter().map(|class| class.as_str()).collect(), data))
    }
}
