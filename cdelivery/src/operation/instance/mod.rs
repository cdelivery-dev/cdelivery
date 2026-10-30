pub(crate) mod typ;

use crate::{config, usecase};

super::define!(
    Get, super::input::Empty => typ::Instance,
    "instance_get", "Get the platform image, version, and public domain.", Shared,
    super::Effect::Read, Standard, super::Media::Json;
    |app, _access, _input| {
        let (image, domain) = usecase::instance::configuration(&app.docker).await?;
        Ok(typ::Instance {
            version: image.tag().clone(),
            image: image.to_string(),
            domain: domain.map(|rule| typ::Domain {
                domain: rule.route().domain.clone(),
                https: rule.https(),
            }),
        })
    }
);

super::define!(
    Logs, super::typ::log::Query => super::typ::log::Records,
    "instance_logs", "Query CDelivery logs newest first for its current service ID; events and chats are separate. Filters combine with AND; words are case-insensitive substrings, including phrases; source is stdout or stderr. For the next best-effort page, increase skip by the returned count.", Shared,
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
            usecase::insight::ContainerTarget::Instance,
            input.into_params(now),
        )
        .await?;
        Ok(super::typ::log::Records::new(service_id, records))
    }
);

super::define!(
    VersionList, super::input::Empty => typ::Versions,
    "version_list", "List the latest component versions.", Shared,
    super::Effect::Read, Standard, super::Media::Json;
    |app, _access, _input| {
        Ok(typ::Versions(usecase::release::list(&app.releases)?.iter().map(|(component, release)| {
            (component.as_ref().to_owned(), release.into())
        }).collect()))
    }
);

super::define!(
    Update, typ::UpdateInstanceRequest => super::Done,
    "instance_update", "Patch the platform image or domain; omitted fields stay unchanged, null domain clears it. Platform routes override app routes on the same hostname, leaving those app routes configured but unreachable.", Admin,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, _access, input| {
        update(app, input).await?;
        Ok(super::Done::new())
    }
);

async fn update(
    app: &crate::boot::App,
    dto: typ::UpdateInstanceRequest,
) -> crate::error::Result<()> {
    let typ::UpdateInstanceRequest { version, domain } = dto;
    let domain = match domain {
        None => None,
        Some(None) => Some(None),
        Some(Some(domain)) => Some(Some(
            pingress_wire::Route {
                domain: domain.domain,
                https: domain.https,
                target: config::PUBLIC_PORT,
                path_prefix: None,
                header: None,
                cookie: None,
            }
            .try_into()?,
        )),
    };
    usecase::instance::update(
        &app.docker,
        &app.key,
        &app.releases,
        version.as_deref(),
        domain,
    )
    .await
}
