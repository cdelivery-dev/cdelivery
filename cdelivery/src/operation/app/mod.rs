pub(crate) mod typ;

use crate::{concept, usecase};

#[derive(serde::Serialize, utoipa::ToSchema)]
pub(crate) enum Updated {
    Applied(Box<typ::view::AppSpec>),
    Guarded(Box<super::guardian::typ::Guardian>),
}

pub(super) async fn get(
    app: &crate::boot::App,
    name: &str,
) -> crate::error::Result<typ::view::AppSpec> {
    view(
        app,
        usecase::app::get(&app.docker, &app.key, &name.parse()?).await?,
    )
    .await
}

async fn view(
    app: &crate::boot::App,
    spec: concept::app::App,
) -> crate::error::Result<typ::view::AppSpec> {
    let mut view: typ::view::AppSpec = spec.try_into()?;
    if let Some(change) = view
        .rollback
        .as_mut()
        .and_then(typ::view::Rollback::change_mut)
        && let Some(id) = change.operator.user_id()
    {
        change.operator_name = match usecase::user::find_by_id(&app.docker, &app.key, id).await {
            Ok(user) => user.map(|user| user.name()),
            Err(error) => {
                tracing::warn!(error = ?error, "app change user {id} lookup failed");
                None
            }
        };
    }
    Ok(view)
}

super::input::composite_input!(UpdateInput {
    #[garde(custom(super::validate::valid_name))]
    name: String,
    #[garde(dive)]
    changes: typ::mutation::Changes,
});

super::input::composite_input!(MetricsInput {
    #[garde(custom(super::validate::valid_name))]
    name: String,
    #[garde(dive)]
    query: super::typ::metric::Query,
});

super::input::composite_input!(LogsInput {
    #[garde(custom(super::validate::valid_name))]
    name: String,
    #[garde(dive)]
    query: super::typ::log::AppQuery,
});

super::input::composite_input!(AccessInput {
    #[garde(custom(super::validate::valid_name))]
    name: String,
    #[garde(dive)]
    query: super::pingress::typ::AppAccessQuery,
});

super::define!(
    List, super::input::Empty => typ::view::Apps,
    "app_list", "List app names and groups.", Shared, super::Effect::Read,
    Standard, super::Media::Json;
    |app, _access, _input| {
        Ok(typ::view::Apps {
            apps: usecase::app::list(&app.docker)
                .await?
                .into_iter()
                .map(|(name, group)| typ::view::AppSummary {
                    name: name.to_string(),
                    group: group.to_string(),
                })
                .collect(),
        })
    }
);

super::define!(
    Get, super::input::Name => typ::view::AppSpec,
    "app_get", "Get an app's configuration, last change, and rollback state.", Shared,
    super::Effect::Read, Standard, super::Media::Json;
    |app, _access, input| {
        get(app, &input.name).await
    }
);

super::define!(
    Status, super::input::Name => typ::view::AppStatus,
    "app_status", "Get current app instance counts and runtime diagnostics; readiness does not include XHealth. Does not change recorded conditions.", Shared,
    super::Effect::Read, Standard, super::Media::Json;
    |app, _access, input| {
        let (instances, diagnostics) = usecase::app::status(&app.docker, &input.name.parse()?).await?;
        Ok(typ::view::AppStatus {
            observed_at: jiff::Timestamp::now(),
            instances: instances.into(),
            diagnostics,
        })
    }
);

super::define!(
    Create, typ::mutation::CreateAppRequest => typ::view::AppSpec,
    "app_create", "Create and deploy an app; omitted options use platform defaults. Platform routes override app routes on the same hostname, leaving those app routes configured but unreachable.", Operator,
    super::Effect::Write { destructive: false, idempotent: false },
    Standard, super::Media::Json;
    |app, access, input, now| {
        let uploads = input
            .xconfigs
            .iter()
            .map(usecase::app::UploadParams::try_from)
            .collect::<crate::error::Result<Vec<_>>>()?;
        let desired = concept::app::Desired::try_from(&input)?;
        let created = usecase::app::create(
            &app.docker,
            &app.key,
            desired,
            uploads,
            access.actor().operator(),
            now,
        )
        .await?;
        view(app, created).await
    }
);

super::define!(
    Update, UpdateInput => Updated,
    "app_update", "Patch an app; omitted fields stay unchanged, nullable fields clear with null. Guarded updates return Guarded instead of Applied; while one is pending, further updates fail. Platform routes override app routes on the same hostname, leaving those app routes configured but unreachable.", Operator,
    super::Effect::Write { destructive: true, idempotent: false },
    Standard, super::Media::Json;
    |app, access, input, now| {
        let name: concept::name::Name = input.name.parse()?;
        let changes = concept::app::Changes::try_from(&input.changes)?;
        let actions = input
            .changes
            .xconfigs
            .as_ref()
            .map(|xs| {
                xs.iter()
                    .map(usecase::app::XConfigParams::try_from)
                    .collect()
            })
            .transpose()?;
        let outcome = usecase::app::update(
            &app.docker,
            &app.key,
            &name,
            changes,
            actions,
            access.actor().operator(),
        )
        .await
        .map_err(|error| error.nested("changes"))?;
        Ok(match outcome {
            concept::app::UpdateOutcome::Applied(spec) => {
                Updated::Applied(Box::new(view(app, *spec).await?))
            }
            concept::app::UpdateOutcome::Guarded => Updated::Guarded(Box::new(
                super::guardian::get(app, name.as_str(), now).await?,
            )),
        })
    }
);

super::define!(
    Delete, super::input::Name => super::Done,
    "app_delete", "Delete an app and its runtime resources.", Operator,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, _access, input| {
        usecase::app::delete(&app.docker, &input.name.parse()?).await?;
        Ok(super::Done::new())
    }
);

super::define!(
    Rollback, super::input::Name => typ::view::AppSpec,
    "app_rollback", "Roll an app back to its previous spec.", Operator,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, _access, input, now| {
        let spec = usecase::app::rollback(&app.docker, &app.key, &input.name.parse()?, now).await?;
        view(app, spec).await
    }
);

super::define!(
    Restart, super::input::Name => super::Done,
    "app_restart", "Restart all app containers.", Operator,
    super::Effect::Write { destructive: true, idempotent: false },
    Standard, super::Media::Json;
    |app, _access, input| {
        usecase::app::restart(&app.docker, &input.name.parse()?).await?;
        Ok(super::Done::new())
    }
);

super::define!(
    Metrics, MetricsInput => super::typ::metric::ContainerSeries,
    "app_metrics", "Query bucketed metrics for the app's current service ID; recreating an app starts separate history. Each point has replicas (distinct containers) and per-series {min,avg,max}. Replicas have equal weight: avg is the mean of replica bucket averages; multiplying by replicas does not give a service total. min/max preserve replica extremes and sample spikes. cpu_usage/cpu_limit use cores; memory_* use bytes except memory_utilization. CPU and memory utilization are cpu_usage/cpu_limit and memory_working_set/memory_limit. Unlimited limits use node capacity; the memory limit applies to memory_current. cpu_throttled is the interval's throttled share, clamped 0-1; psi_* are pressure-stall ratios 0-1. pids/pids_limit are counts; oom_kill_rate is kills/s, disk and network rates bytes/s, disk IOPS ops/s, tcp_retrans_rate segments/s. step_secs = ceil(range/max_buckets), clamped 1s-1 day and raised to the Insight sampling interval when available; max_buckets defaults to 20, accepts 1-720; range <=31 days. instance selects a container_id from instances. Missing buckets or fields mean no valid data.", Shared,
    super::Effect::Read, Extended, super::Media::Json;
    validate |input, now, report| {
        super::validate::append(
            report,
            garde::Path::new("query"),
            super::validate::query_time_range(input.query.start, input.query.end, now),
        );
    };
    |app, _access, input, now| {
        let name = input.name.parse()?;
        super::insight::container_series(
            app,
            usecase::insight::ContainerTarget::App(&name),
            input.query,
            now,
        )
        .await
    }
);

super::define!(
    Logs, LogsInput => super::typ::log::Records,
    "app_logs", "Query an app log target newest first for its current service ID; earlier service generations are excluded. Use app_log_sources to discover current targets. Logs must have been collected by Insight. Multiple runs may be included; container_id selects a container. Filters combine with AND; words are case-insensitive substrings, including phrases; source is stdout or stderr. For the next best-effort page, increase skip by the returned count.", Shared,
    super::Effect::Read, Extended, super::Media::Json;
    validate |input, now, report| {
        super::validate::append(
            report,
            garde::Path::new("query"),
            super::validate::query_time_range(input.query.start, input.query.end, now),
        );
    };
    |app, _access, input, now| {
        let name = input.name.parse()?;
        let job = input.query.job.as_deref().map(str::parse).transpose()?;
        let target = match (&input.query.target, job.as_ref()) {
            (super::typ::log::AppTarget::XJob, Some(job)) => {
                usecase::insight::ContainerTarget::XJob(&name, job)
            }
            (super::typ::log::AppTarget::XJob, None) => {
                return Err(crate::error::ErrorKind::FieldRequired("query.job".to_owned()).into());
            }
            (_, Some(_)) => {
                return Err(crate::error::ErrorKind::FieldUnsupported("query.job".to_owned()).into());
            }
            (super::typ::log::AppTarget::App, None) => usecase::insight::ContainerTarget::App(&name),
            (super::typ::log::AppTarget::XHealth, None) => usecase::insight::ContainerTarget::XHealth(&name),
            (super::typ::log::AppTarget::GuardianApp, None) => usecase::insight::ContainerTarget::GuardianApp(&name),
            (super::typ::log::AppTarget::GuardianCheck, None) => usecase::insight::ContainerTarget::GuardianCheck(&name),
        };
        let (service_id, records) = usecase::insight::logs(
            &app.docker,
            &app.insight,
            target,
            input.query.into_params(now),
        )
        .await?;
        Ok(super::typ::log::Records::new(service_id, records))
    }
);

super::define!(
    LogSources, super::input::Name => super::typ::log::Sources,
    "app_log_sources", "List an app's current log targets configured to send logs to Insight. Targets disappear when their services are removed or stop using Insight; availability does not imply that Insight has collected logs.", Shared,
    super::Effect::Read, Standard, super::Media::Json;
    |app, _access, input| {
        Ok(super::typ::log::Sources {
            sources: usecase::app::log_sources(&app.docker, &input.name.parse()?)
                .await?
                .into_iter()
                .map(Into::into)
                .collect(),
        })
    }
);

super::define!(
    Access, AccessInput => super::pingress::typ::AccessRecords,
    "app_access", "Query HTTP access records newest first for the app's current service ID; earlier generations are excluded. Results may be empty after recreation until routes refresh. For the next best-effort page, increase skip by the returned count. Filters follow pingress_access: AND, exact matches except case-insensitive literal search_path/search_client_ip. Paths exclude query strings and are capped at 2048 bytes; client IPs use canonical IPv6 without ports. Only Proxied, UpstreamFailed, and HttpsRedirect records carry service IDs.", Shared,
    super::Effect::Read, Extended, super::Media::Json;
    validate |input, now, report| {
        super::validate::append(
            report,
            garde::Path::new("query"),
            super::validate::query_time_range(input.query.start, input.query.end, now),
        );
    };
    |app, _access, input, now| {
        let name = input.name.parse()?;
        let range = super::validate::query_range(input.query.start, input.query.end, now);
        let (skip, limit) = (input.query.skip, input.query.limit);
        let filter = input.query.into_filter()?;
        let records = usecase::app::access(
            &app.docker,
            &app.insight,
            &name,
            range,
            skip,
            limit,
            filter,
        )
        .await?;
        Ok(super::pingress::typ::AccessRecords { records })
    }
);

pub(crate) struct DownloadXConfig;

impl super::Operation for DownloadXConfig {
    type Input = super::input::AppConfig;
    type Output = super::Artifact;

    const ID: &'static str = "app_download_xconfig";
    const DESCRIPTION: &'static str = "Download an app's configuration file.";
    const REQUIREMENT: crate::concept::permission::Requirement =
        crate::concept::permission::Requirement::Shared;
    const EFFECT: super::Effect = super::Effect::Read;
    const BUDGET: super::Budget = super::Budget::Standard;
    const MEDIA: super::Media = super::Media::Artifact("application/octet-stream");

    async fn run(
        app: &crate::boot::App,
        _access: &crate::concept::permission::Access,
        input: Self::Input,
        _now: jiff::Timestamp,
    ) -> crate::error::Result<Self::Output> {
        let bytes = usecase::app::download_xconfig(
            &app.docker,
            &app.key,
            &input.app.parse()?,
            &input.config,
        )
        .await?;
        Ok(super::Artifact {
            uri: format!(
                "cdelivery:/apps/{}/xconfig/{}/file",
                input.app, input.config
            ),
            filename: input.config,
            bytes,
        })
    }

    fn payload(output: Self::Output) -> crate::error::Result<super::Payload> {
        super::artifact_payload::<Self>(output)
    }
}
