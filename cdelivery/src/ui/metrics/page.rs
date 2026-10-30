use askama::Template;
use axum::response::Response;
use http::{StatusCode, Uri};
use jiff::Timestamp;
use serde::Deserialize;

use crate::locale::tt;
use crate::{concept, config, operation};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Query {
    range: Option<super::super::range::Range>,
    start: Option<Timestamp>,
    end: Option<Timestamp>,
    #[serde(default)]
    instance: String,
    component: Option<operation::insight::typ::Component>,
}

enum LoadTarget<'a> {
    App { name: &'a str, has_domains: bool },
    Insight(operation::insight::typ::Component),
    Pingress,
}

#[derive(Template)]
#[template(path = "metrics.html")]
struct Page<'a> {
    dashboard: super::super::dashboard::Dashboard,
    target: super::super::target::Target<'a>,
    can_edit: bool,
    action: String,
    window: super::super::range::Window,
    instance: String,
    instances: Vec<super::container::Instance>,
    show_controls: bool,
    component: &'static str,
    state: Option<super::MetricsState>,
}

pub(in crate::ui) async fn render(
    app: &crate::boot::App,
    access: &concept::permission::Access,
    target: super::super::target::Target<'_>,
    uri: &Uri,
    now: Timestamp,
) -> super::super::response::Result<Response> {
    let axum::extract::Query(query) = axum::extract::Query::<Query>::try_from_uri(uri)
        .map_err(|error| crate::error::ErrorKind::InvalidBody(error.body_text()))?;
    let is_insight = matches!(&target, super::super::target::Target::Insight);
    if !is_insight && query.component.is_some() {
        return Err(crate::error::ErrorKind::InvalidBody(
            "unknown query field `component`".to_owned(),
        )
        .into());
    }
    let window = super::super::range::Window::preset(query.range, query.start, query.end, now);
    let location = target.location();
    let load_target = match &target {
        super::super::target::Target::App(spec) => LoadTarget::App {
            name: &spec.name,
            has_domains: !spec.domains.is_empty(),
        },
        super::super::target::Target::Insight => {
            LoadTarget::Insight(query.component.unwrap_or_default())
        }
        super::super::target::Target::Pingress => LoadTarget::Pingress,
    };
    let (dashboard, loaded) = tokio::try_join!(
        super::super::dashboard::Dashboard::load(app, access, location),
        load(
            app,
            access,
            &load_target,
            &query.instance,
            window.range,
            true,
            now,
        ),
    )?;
    let (instances, state) = match loaded {
        Some((instances, state)) => (instances, Some(state)),
        None => (Vec::new(), None),
    };
    let action = uri.path().to_owned();
    let component: &'static str = if is_insight {
        query.component.unwrap_or_default().into()
    } else {
        ""
    };
    let show_controls = is_insight || !matches!(&state, Some(super::MetricsState::Disabled));
    super::super::response::render(
        &Page {
            dashboard,
            target,
            can_edit: access.allows(concept::permission::Requirement::Operator),
            action,
            window,
            instance: query.instance,
            instances,
            show_controls,
            component,
            state,
        },
        StatusCode::OK,
    )
}

pub(in crate::ui) async fn app_overview(
    app: &crate::boot::App,
    access: &concept::permission::Access,
    name: &str,
    has_domains: bool,
    range: (Timestamp, Timestamp),
    now: Timestamp,
) -> crate::error::Result<super::MetricsState> {
    let target = LoadTarget::App { name, has_domains };
    Ok(load(app, access, &target, "", range, false, now)
        .await?
        .map_or(super::MetricsState::Disabled, |(_, state)| state))
}

async fn load(
    app: &crate::boot::App,
    access: &concept::permission::Access,
    target: &LoadTarget<'_>,
    instance: &str,
    range: (Timestamp, Timestamp),
    full: bool,
    now: Timestamp,
) -> crate::error::Result<Option<(Vec<super::container::Instance>, super::MetricsState)>> {
    if matches!(target, LoadTarget::Pingress)
        && super::super::pingress::load_pingress(app, access)
            .await?
            .is_none()
    {
        return Ok(None);
    }
    let metric_query = operation::typ::metric::Query {
        start: Some(range.0),
        end: Some(range.1),
        instance: (!instance.is_empty()).then(|| instance.to_owned()),
        max_buckets: Some(config::UI_BUCKETS),
    };
    let metrics_future = Box::pin(async {
        match target {
            LoadTarget::App { name, .. } => {
                super::super::insight::optional_query(
                    operation::execute::<operation::app::Metrics>(
                        app,
                        access,
                        operation::app::MetricsInput {
                            name: (*name).to_owned(),
                            query: metric_query,
                        },
                        now,
                    ),
                )
                .await
            }
            LoadTarget::Pingress => {
                super::super::insight::optional_query(operation::execute::<
                    operation::pingress::Metrics,
                >(
                    app, access, metric_query, now
                ))
                .await
            }
            LoadTarget::Insight(component) => {
                super::super::insight::optional_query(operation::execute::<
                    operation::insight::Metrics,
                >(
                    app,
                    access,
                    operation::insight::MetricsInput {
                        component: *component,
                        query: metric_query,
                    },
                    now,
                ))
                .await
            }
        }
    });
    let (metrics, access_metric_groups) = tokio::try_join!(
        metrics_future,
        access_groups(app, access, target, range, full, now),
    )?;
    Ok(Some(match (metrics, access_metric_groups) {
        (Some(output), Some(groups)) => (
            if full {
                let nodes = super::super::load_nodes(app, access, !output.instances.is_empty())
                    .await
                    .map_or_else(
                        |error| {
                            tracing::warn!(error = ?error, "node lookup failed for metric instance labels");
                            Vec::new()
                        },
                        |nodes| nodes.nodes,
                    );
                super::container::instances(&output.instances, instance, &nodes)
            } else {
                Vec::new()
            },
            super::state(Some(super::container::charts(
                full,
                output.step_secs,
                &output.points,
                groups,
            ))),
        ),
        _ => (Vec::new(), super::MetricsState::Disabled),
    }))
}

async fn access_groups(
    app: &crate::boot::App,
    access: &concept::permission::Access,
    target: &LoadTarget<'_>,
    range: (Timestamp, Timestamp),
    group_by_status: bool,
    now: Timestamp,
) -> crate::error::Result<Option<Vec<super::Group>>> {
    let app_name = match target {
        LoadTarget::App {
            name,
            has_domains: true,
            ..
        } => Some((*name).to_owned()),
        LoadTarget::App { .. } | LoadTarget::Insight(_) => return Ok(Some(Vec::new())),
        LoadTarget::Pingress => None,
    };
    let input = operation::pingress::typ::AccessStatsQuery {
        start: Some(range.0),
        end: Some(range.1),
        app_name,
        group_by_status,
        max_buckets: Some(config::UI_BUCKETS),
        ..Default::default()
    };
    let output = super::super::insight::optional_query(operation::execute::<
        operation::pingress::AccessStats,
    >(app, access, input, now))
    .await?;
    Ok(output.map(|stats| super::access::charts(&stats)))
}
