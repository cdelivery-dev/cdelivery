pub(crate) mod typ;

use crate::usecase;

pub(in crate::operation) async fn metric_window(
    app: &crate::boot::App,
    start: Option<jiff::Timestamp>,
    end: Option<jiff::Timestamp>,
    max_buckets: Option<u16>,
    now: jiff::Timestamp,
) -> ((jiff::Timestamp, jiff::Timestamp), u32) {
    let range = super::validate::query_range(start, end, now);
    let step_secs = crate::config::metric_step_secs(range, max_buckets);
    let step_secs = usecase::insight::step_floor(&app.docker, step_secs).await;
    (range, step_secs)
}

pub(in crate::operation) async fn container_series(
    app: &crate::boot::App,
    target: usecase::insight::ContainerTarget<'_>,
    query: super::typ::metric::Query,
    now: jiff::Timestamp,
) -> crate::error::Result<super::typ::metric::ContainerSeries> {
    let (range, step_secs) =
        metric_window(app, query.start, query.end, query.max_buckets, now).await;
    let (points, instances) = usecase::insight::container_metrics(
        &app.docker,
        &app.insight,
        target,
        range,
        query.instance,
        step_secs,
    )
    .await?;
    Ok(super::typ::metric::ContainerSeries {
        step_secs,
        instances,
        points,
    })
}

super::input::composite_input!(MetricsInput {
    #[garde(skip)]
    component: typ::Component,
    #[garde(dive)]
    query: super::typ::metric::Query,
});

super::define!(
    Overview, super::input::Empty => typ::Insight,
    "insight_overview", "Get Insight deployment status.", Shared,
    super::Effect::Read, Standard, super::Media::Json;
    |app, _access, _input| {
        usecase::insight::overview(&app.docker, &app.key)
            .await?
            .try_into()
    }
);

super::define!(
    Metrics, MetricsInput => super::typ::metric::ContainerSeries,
    "insight_metrics", "Query bucketed metrics for the current Insight Server or Agent service. Uses app_metrics units, averaging, sparse-data, and bucket rules (20 buckets by default; max_buckets 1-720). History is scoped to the component's current service ID; instance selects a container ID.", Shared,
    super::Effect::Read, Extended, super::Media::Json;
    validate |input, now, report| {
        super::validate::append(
            report,
            garde::Path::new("query"),
            super::validate::query_time_range(input.query.start, input.query.end, now),
        );
    };
    |app, _access, input, now| {
        let target = match input.component {
            typ::Component::Server => {
                usecase::insight::ContainerTarget::InsightServer
            }
            typ::Component::Agent => usecase::insight::ContainerTarget::InsightAgent,
        };
        container_series(app, target, input.query, now).await
    }
);

super::define!(
    Enable, typ::EnableInsightRequest => typ::Insight,
    "insight_enable", "Deploy or configure Insight; omitted fields retain current values. First deployment requires storage, retention_days, and placement; each missing component requires its version. sample_interval_secs: 1-60, default 5. flush_interval: maximum flush delay, default 3 Minutes.", Admin,
    super::Effect::Write { destructive: true, idempotent: true },
    Extended, super::Media::Json;
    |app, _access, input| {
        usecase::insight::enable(
            &app.docker,
            &app.key,
            &app.releases,
            input.storage.map(Into::into),
            input.retention_days,
            input.placement.map(Into::into),
            input.server_version,
            input.agent_version,
            input.sample_interval_secs,
            input.flush_interval.map(Into::into),
        )
        .await?
        .try_into()
    }
);

super::define!(
    Disable, super::input::Empty => super::Done,
    "insight_disable", "Remove Insight deployments; retain stored data.", Admin,
    super::Effect::Write { destructive: true, idempotent: true },
    Extended, super::Media::Json;
    |app, _access, _input| {
        usecase::insight::disable(&app.docker).await?;
        Ok(super::Done::new())
    }
);
