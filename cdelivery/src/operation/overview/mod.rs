pub(crate) mod typ;

use crate::{config, usecase};

super::define!(
    Get, super::typ::metric::RangeQuery => typ::Overview,
    "platform_overview", "Get cluster status and usage charts. metrics.status: Disabled when Insight is not fully deployed, Unavailable on query failure, Ready when charts exist. CPU, memory and disk rank all managed services over the full range by Service ID; each returns up to five Service series and Rest, with Host for CPU and memory. Series preserve ranking order; points are chronological. Samples belong to the bucket containing their timestamp. Missing values and negative Host residuals are null. Cluster cpu_total/memory_total provide historical capacity; network uses host counters. Each pressure metric ranks five nodes; filesystems rank ten mounts by peak utilization. node_names maps observed node IDs to names; missing names fall back to IDs. Access follows pingress_access_stats. max_buckets defaults to 20, accepts 1-720; range <=31 days. step_secs follows range/max_buckets, rounded up to a multiple of the current Agent sampling interval read for this request, at least one interval and capped at 1 day. Historical values use their recorded sampling durations.", Shared,
    super::Effect::Read, Extended, super::Media::Json;
    validate |input, now, report| {
        super::validate::append(
            report,
            garde::Path::empty(),
            super::validate::query_time_range(input.start, input.end, now),
        );
    };
    |app, _access, input, now| {
        let range = super::validate::query_range(input.start, input.end, now);
        let step_secs = config::metric_step_secs(range, input.max_buckets);
        let (summary, metrics) =
            usecase::overview::load(&app.docker, &app.insight, &app.key, range, step_secs).await?;
        Ok(typ::Overview::assemble(&summary, metrics, range))
    }
);
