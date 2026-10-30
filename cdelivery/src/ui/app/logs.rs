use std::sync::Arc;

use askama::Template;
use axum::extract::{OriginalUri, State};
use axum::response::Response;
use http::StatusCode;
use jiff::Timestamp;
use serde::Deserialize;

use crate::locale::tt;
use crate::{concept, operation};

const PAGE_SIZE: u16 = operation::typ::default_query_limit();

mod source;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageQuery {
    range: Option<super::super::range::Range>,
    start: Option<Timestamp>,
    end: Option<Timestamp>,
    #[serde(default)]
    text: String,
    #[serde(default)]
    search_path: String,
    #[serde(default)]
    search_client_ip: String,
    #[serde(default)]
    method: String,
    #[serde(default)]
    status_class: String,
    source: Option<source::Source>,
    #[serde(default)]
    skip: u32,
}

impl PageQuery {
    fn select_source(&mut self, sources: &mut Vec<source::Source>) {
        match self.source.as_ref() {
            Some(selected) if sources.contains(selected) => {}
            Some(selected) => sources.push(selected.clone()),
            None => {
                self.source = sources.first().cloned();
                self.skip = 0;
            }
        }
    }
}

fn access_query(
    query: &PageQuery,
    start: Timestamp,
    end: Timestamp,
    skip: u32,
) -> crate::error::Result<operation::pingress::typ::AppAccessQuery> {
    Ok(operation::pingress::typ::AppAccessQuery {
        start: Some(start),
        end: Some(end),
        method: super::super::token(Some(&query.method))?,
        status_class: super::super::token(Some(&query.status_class))?,
        search_path: trimmed(&query.search_path),
        search_client_ip: trimmed(&query.search_client_ip),
        skip,
        limit: PAGE_SIZE,
        ..Default::default()
    })
}

fn trimmed(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[derive(Template)]
#[template(path = "app/logs.html")]
struct Page {
    dashboard: super::super::dashboard::Dashboard,
    name: String,
    description: Option<String>,
    can_edit: bool,
    sources: Vec<source::Source>,
    source_error: Option<String>,
    query: PageQuery,
    window: super::super::range::Window,
    state: LogsState,
}

enum LogsState {
    InsightDisabled,
    NoSources,
    Unavailable,
    Failed(String),
    Logs(Vec<insight_wire::log::Log>),
    Access(Vec<AccessRow>),
}

impl Page {
    fn selected(&self, source: &source::Source) -> bool {
        self.query.source.as_ref() == Some(source)
    }

    fn access_selected(&self) -> bool {
        matches!(self.query.source, Some(source::Source::Access))
    }

    fn pages(&self, len: usize) -> (Option<String>, Option<String>) {
        let source = self
            .query
            .source
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default();
        super::super::pages(self.query.skip, len, PAGE_SIZE, |skip| {
            super::super::page_url(
                &format!("/apps/{}/logs", self.name),
                self.window.range,
                &[
                    ("source", &source),
                    ("text", self.query.text.trim()),
                    ("search_path", &self.query.search_path),
                    ("search_client_ip", &self.query.search_client_ip),
                    ("method", &self.query.method),
                    ("status_class", &self.query.status_class),
                ],
                skip,
            )
        })
    }
}

struct AccessRow {
    time: String,
    method: String,
    path: String,
    status: String,
    duration: String,
    client_ip: String,
    detail: AccessDetail,
}

struct AccessDetail {
    domain: String,
    outcome: String,
    node_id: String,
    target_port: String,
    request_bytes: String,
    response_bytes: String,
    upstream_ttfb: String,
    service_id: String,
    error: String,
}

pub(super) fn routes() -> axum::Router<Arc<crate::boot::App>> {
    axum::Router::new().route("/apps/{name}/logs", axum::routing::get(get))
}

async fn get(
    authorized: super::super::auth::Authorized<operation::app::Logs>,
    State(app): State<Arc<crate::boot::App>>,
    axum::extract::Path(name): axum::extract::Path<String>,
    OriginalUri(uri): OriginalUri,
) -> super::super::response::Result<Response> {
    let now = Timestamp::now();
    let spec = super::load_app(&app, authorized.access(), name).await?;
    let axum::extract::Query(mut query) = axum::extract::Query::<PageQuery>::try_from_uri(&uri)
        .map_err(|error| crate::error::ErrorKind::InvalidBody(error.body_text()))?;
    let window = super::super::range::Window::preset(query.range, query.start, query.end, now);
    window.validate(now)?;
    let (mut sources, source_error) = source::list(&app, authorized.access(), &spec, now).await;
    let source_unavailable = source_error.is_none()
        && query
            .source
            .as_ref()
            .is_some_and(|source| !sources.contains(source));
    query.select_source(&mut sources);
    let state = async {
        if source_unavailable {
            return Ok(LogsState::Unavailable);
        }
        match query.source.as_ref() {
            None => Ok::<_, crate::error::Error>(LogsState::NoSources),
            Some(source::Source::Access) => {
                let output = operation::execute::<operation::app::Access>(
                    &app,
                    authorized.access(),
                    operation::app::AccessInput {
                        name: spec.name.clone(),
                        query: access_query(&query, window.range.0, window.range.1, query.skip)?,
                    },
                    now,
                )
                .await?;
                Ok(LogsState::Access(
                    output.records.into_iter().map(access_row).collect(),
                ))
            }
            Some(source) => {
                let (target, job) = source.target()?;
                let output = operation::execute::<operation::app::Logs>(
                    &app,
                    authorized.access(),
                    operation::app::LogsInput {
                        name: spec.name.clone(),
                        query: operation::typ::log::AppQuery {
                            target,
                            job,
                            start: Some(window.range.0),
                            end: Some(window.range.1),
                            words: split_words(&query.text),
                            source: None,
                            node_id: None,
                            container_id: None,
                            skip: query.skip,
                            limit: PAGE_SIZE,
                        },
                    },
                    now,
                )
                .await?;
                Ok(LogsState::Logs(output.records))
            }
        }
    };
    let (dashboard, state) = tokio::join!(
        super::dashboard(&app, authorized.access(), &spec.name),
        state,
    );
    let state = match state {
        Ok(state) => state,
        Err(error) if matches!(error.kind(), crate::error::ErrorKind::InsightNotEnabled) => {
            LogsState::InsightDisabled
        }
        Err(error)
            if matches!(
                error.kind(),
                crate::error::ErrorKind::XHealthNotEnabled
                    | crate::error::ErrorKind::XJobNotFound
                    | crate::error::ErrorKind::GuardianNotFound
            ) =>
        {
            LogsState::Unavailable
        }
        Err(error) => LogsState::Failed(super::super::read_error(&error, "app log query failed")),
    };
    super::super::response::render(
        &Page {
            dashboard: dashboard?,
            name: spec.name,
            description: spec.description,
            can_edit: authorized
                .access()
                .allows(concept::permission::Requirement::Operator),
            sources,
            source_error,
            query,
            window,
            state,
        },
        StatusCode::OK,
    )
}

fn split_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut rest = text.trim_start();
    while !rest.is_empty() {
        let quoted = ['"', '\''].into_iter().find_map(|quote| {
            let (word, tail) = rest.strip_prefix(quote)?.split_once(quote)?;
            (tail.is_empty() || tail.starts_with(char::is_whitespace)).then_some((word, tail))
        });
        let (word, tail) =
            quoted.unwrap_or_else(|| rest.split_once(char::is_whitespace).unwrap_or((rest, "")));
        if !word.is_empty() {
            words.push(word.to_owned());
        }
        rest = tail.trim_start();
    }
    words
}

fn dash(value: Option<String>) -> String {
    value.unwrap_or_else(|| "—".to_owned())
}

fn access_row(record: pingress_wire::access::Access) -> AccessRow {
    AccessRow {
        time: record.timestamp.to_string(),
        method: record.method,
        path: record.path,
        status: dash(record.status.map(|code| code.to_string())),
        duration: duration_text(record.duration_us),
        client_ip: dash(record.client_ip),
        detail: AccessDetail {
            domain: dash(record.domain),
            outcome: record.outcome.as_str().to_owned(),
            node_id: record.node_id,
            target_port: dash(record.target_port.map(|port| port.to_string())),
            request_bytes: super::super::bytes_text(record.request_body_bytes),
            response_bytes: super::super::bytes_text(record.response_body_bytes),
            upstream_ttfb: dash(record.upstream_ttfb_us.map(duration_text)),
            service_id: dash(record.service_id),
            error: dash(record.error),
        },
    }
}

#[expect(clippy::cast_precision_loss)]
fn duration_text(us: u64) -> String {
    if us >= 1_000_000 {
        format!("{:.2} s", us as f64 / 1e6)
    } else if us >= 1_000 {
        format!("{:.1} ms", us as f64 / 1e3)
    } else {
        format!("{us} µs")
    }
}
