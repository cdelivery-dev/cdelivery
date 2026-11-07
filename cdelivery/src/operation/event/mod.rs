use crate::usecase;

pub(crate) mod typ;

super::define!(
    Conditions, super::input::Empty => typ::Conditions,
    "event_conditions", "Read timestamped runtime, health-check and monitoring diagnostics. RuntimeIssue covers current demanded tasks, paused updates and instance shortages; stopped task history is excluded. Use app_status for apps and platform_overview for platform services. Shortages alone wait two minutes before alerting; current errors alert immediately. Records are in memory; check observed_at for freshness. XHealthFailed is separate; use xhealth_get for the current round.", Shared,
    super::Effect::Read, Standard, super::Media::Json;
    |_app, _access, _input, _now| {
        Ok(typ::Conditions {
            conditions: usecase::event::conditions(),
        })
    }
);

super::input::composite_input!(ListInput {
    #[garde(dive)]
    query: typ::EventQuery,
});

super::define!(
    List, ListInput => typ::EventRecords,
    "event_list", "Query platform events newest first. Returns resolved start/end even when records is empty. If no reliable time reference is available, omit end and use the returned end as server time; use limit=1 only when probing for time alone. An explicit end is only a requested boundary. For best-effort paging, reuse the returned start/end and increase skip by the returned count.", Shared, super::Effect::Read,
    Extended, super::Media::Json;
    validate |input, now, report| {
        super::validate::append(
            report,
            garde::Path::new("query"),
            super::validate::query_time_range(input.query.start, input.query.end, now),
        );
    };
    |app, _access, input, now| {
        let (start, end) = super::validate::query_range(input.query.start, input.query.end, now);
        let records = usecase::event::list(
            &app.docker,
            &app.insight,
            (start, end),
            input.query.skip,
            input.query.limit,
            input.query.kind,
            input.query.subject,
        )
        .await?;
        Ok(typ::EventRecords {
            start,
            end,
            records,
        })
    }
);
