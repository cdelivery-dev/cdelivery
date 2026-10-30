pub(crate) mod typ;

use crate::{concept, usecase};

super::input::composite_input!(UploadCertInput {
    #[garde(custom(super::validate::valid_domain))]
    domain: String,
    #[garde(dive)]
    cert: typ::UploadCertRequest,
});

super::define!(
    Get, super::input::Empty => typ::Pingress,
    "pingress_get", "Get Pingress deployment, version, and ACME status.", Shared,
    super::Effect::Read, Standard, super::Media::Json;
    |app, _access, _input| {
        Ok(usecase::pingress::get(&app.docker, &app.key).await?.into())
    }
);

super::define!(
    Enable, typ::CreatePingressRequest => typ::Pingress,
    "pingress_enable", "Deploy Pingress.", Admin,
    super::Effect::Write { destructive: false, idempotent: false },
    Standard, super::Media::Json;
    |app, _access, input| {
        Ok(usecase::pingress::enable(
            &app.docker,
            &app.key,
            &app.releases,
            &input.version,
            input.placement.into(),
        )
        .await?
        .into())
    }
);

super::define!(
    Update, typ::UpdatePingressRequest => typ::Pingress,
    "pingress_update", "Patch the Pingress deployment.", Admin,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, _access, input| {
        Ok(usecase::pingress::update(
            &app.docker,
            &app.key,
            &app.releases,
            input.version.as_deref(),
            input.placement.map(Into::into),
        )
        .await?
        .into())
    }
);

super::define!(
    Disable, super::input::Empty => super::Done,
    "pingress_disable", "Remove Pingress and automatic certificates; app domains or uploaded certificates block removal.", Admin,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, _access, _input| {
        usecase::pingress::disable(&app.docker, &app.key, &app.challenges).await?;
        Ok(super::Done::new())
    }
);

super::define!(
    Metrics, super::typ::metric::Query => super::typ::metric::ContainerSeries,
    "pingress_metrics", "Query bucketed metrics for the current Pingress service. Uses app_metrics units, averaging, sparse-data, and bucket rules (20 buckets by default; max_buckets 1-720). History is scoped to the current service ID; instance selects a container ID.", Shared,
    super::Effect::Read, Extended, super::Media::Json;
    validate |input, now, report| {
        super::validate::append(
            report,
            garde::Path::empty(),
            super::validate::query_time_range(input.start, input.end, now),
        );
    };
    |app, _access, input, now| {
        super::insight::container_series(
            app,
            usecase::insight::ContainerTarget::Pingress,
            input,
            now,
        )
        .await
    }
);

super::define!(
    Access, typ::AccessQuery => typ::AccessRecords,
    "pingress_access", "Query ingress access records newest first. For the next best-effort page, increase skip by the returned count. Filters combine with AND and match exactly, except search_path/search_client_ip: case-insensitive literal substrings, no regex or wildcards. Paths exclude query strings and are capped at 2048 bytes with a truncation marker; client IPs use canonical IPv6 without ports. domain is the serving managed domain; unmatched_host is an unmanaged valid Host; they never coexist. status is the HTTP status sent to the client; null means no response. ServerError covers status >=500; NoResponse selects null status. outcome is Pingress's verdict, independent of status. app_name, target_port, and service_id identify the selected upstream (including HttpsRedirect); all are null if none was selected. service_id is the app's service ID from the request-time routing snapshot, not the Pingress service ID. It separates app generations after routes refresh and supports deleted apps. error is a Pingora error-type token; node_id identifies the handling Pingress node.", Shared,
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
        let (skip, limit) = (input.skip, input.limit);
        let filter = input.into_filter()?;
        let records =
            usecase::pingress::access(&app.docker, &app.insight, range, skip, limit, filter)
                .await?;
        Ok(typ::AccessRecords { records })
    }
);

super::define!(
    AccessStats, typ::AccessStatsQuery => typ::AccessStats,
    "pingress_access_stats", "Query bucketed ingress access statistics using pingress_access filters. Buckets start at start in [start,end); step_secs = ceil(range/max_buckets), clamped 1s-1 day. max_buckets defaults to 20, accepts 1-720. Different starts produce different grids. Missing buckets and zero-count entries mean zero. QPS uses request completion times and actual bucket width, including a shorter final bucket. Byte counts are totals; duration_us/upstream_ttfb_us use microseconds with sample_count, approximate p50/p95/p99, and exact max. TTFB runs from request start to first upstream response header, including routing/connect time; only requests receiving that header contribute. status_classes/outcomes give counts and request_count ratios; ServerError is status >=500, NoResponse means no HTTP response. group_by_status=true adds sparse count/QPS series by exact status, ascending with null last. Overall buckets are authoritative if concurrent ingestion makes by_status differ.", Shared,
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
        let step_secs = crate::config::metric_step_secs(range, input.max_buckets);
        let group_by_status = input.group_by_status;
        let filter = input.into_filter()?;
        let (rows, status_rows) = usecase::pingress::access_stats(
            &app.docker,
            &app.insight,
            range,
            step_secs,
            group_by_status,
            filter,
        )
        .await?;
        Ok(typ::AccessStats::assemble(
            range,
            step_secs,
            rows,
            status_rows,
        ))
    }
);

super::define!(
    Logs, super::typ::log::Query => super::typ::log::Records,
    "pingress_logs", "Query Pingress logs newest first for its current service ID; use pingress_access for HTTP records. Filters combine with AND; words are case-insensitive substrings, including phrases; source is stdout or stderr. For the next best-effort page, increase skip by the returned count.", Shared,
    super::Effect::Read, Extended, super::Media::Json;
    validate |input, now, report| {
        super::validate::append(
            report,
            garde::Path::empty(),
            super::validate::query_time_range(input.start, input.end, now),
        );
    };
    |app, _access, input, now| {
        let (service_id, records) = usecase::insight::logs(
            &app.docker,
            &app.insight,
            usecase::insight::ContainerTarget::Pingress,
            input.into_params(now),
        )
        .await?;
        Ok(super::typ::log::Records::new(service_id, records))
    }
);

super::define!(
    EnableAccount, typ::EnableAcmeRequest => typ::AcmeAccount,
    "acme_enable_account", "Ensure an ACME account exists; return its URI and whether it was created. Requires Pingress.", Admin,
    super::Effect::Write { destructive: false, idempotent: true },
    Standard, super::Media::Json;
    |app, _access, input| {
        let ca = input.ca.map_or(concept::acme::CA::LetsEncrypt, Into::into);
        let (uri, created) = usecase::acme::enable_account(&app.docker, &app.key, ca).await?;
        Ok(typ::AcmeAccount { uri, created })
    }
);

super::define!(
    DisableAccount, super::input::Empty => super::Done,
    "acme_disable_account", "Remove the ACME account and automatic certificates; retain uploaded certificates.", Admin,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, _access, _input| {
        usecase::acme::disable_account(&app.docker, &app.key, &app.challenges).await?;
        Ok(super::Done::new())
    }
);

super::define!(
    UploadCert, UploadCertInput => super::Done,
    "acme_upload_cert", "Upload or replace a domain certificate; uploaded certificates take priority and never renew automatically. Requires Pingress.", Admin,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, _access, input| {
        usecase::acme::upload_cert(
            &app.docker,
            &app.key,
            input.domain.parse()?,
            input.cert.cert_chain_pem,
            input.cert.private_key_pem,
        )
        .await?;
        Ok(super::Done::new())
    }
);

super::define!(
    RemoveCert, super::input::Domain => super::Done,
    "acme_remove_cert", "Remove an uploaded certificate.", Admin,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, _access, input| {
        usecase::acme::remove_cert(&app.docker, &app.key, &input.domain.parse()?).await?;
        Ok(super::Done::new())
    }
);

super::define!(
    ListCerts, super::input::Empty => typ::Certs,
    "acme_list_certs", "List HTTPS app domains and stored certificates.", Shared,
    super::Effect::Read, Standard, super::Media::Json;
    |app, _access, _input, now| {
        let certs = usecase::acme::list_certs(&app.docker, &app.key)
            .await?
            .into_iter()
            .map(|(domain, apps, cert)| typ::CertEntry::new(&domain, apps, cert.as_ref(), now))
            .collect();
        Ok(typ::Certs { certs })
    }
);
