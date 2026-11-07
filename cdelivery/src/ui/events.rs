use askama::Template;
use axum::response::Response;
use http::StatusCode;
use insight_wire::event::Kind;
use jiff::Timestamp;
use serde::Deserialize;
use strum::VariantArray;

use crate::locale::tt;
use crate::{concept, operation};

const PAGE_SIZE: u16 = 50;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Query {
    range: Option<super::range::Range>,
    start: Option<Timestamp>,
    end: Option<Timestamp>,
    kind: Option<String>,
    #[serde(default)]
    skip: u32,
}

#[derive(Template)]
#[template(path = "events.html")]
struct Page<'a> {
    dashboard: super::dashboard::Dashboard,
    target: super::target::Target<'a>,
    can_edit: bool,
    action: String,
    window: super::range::Window,
    kind: &'static str,
    skip: u32,
    conditions: super::conditions::Result,
    history: std::result::Result<Option<operation::event::typ::EventRecords>, String>,
}

impl Page<'_> {
    fn pages(&self, len: usize) -> (Option<String>, Option<String>) {
        super::pages(self.skip, len, PAGE_SIZE, |skip| {
            super::page_url(
                &self.action,
                self.window.range,
                &[("kind", self.kind)],
                skip,
            )
        })
    }
}

pub(super) async fn render(
    app: &crate::boot::App,
    access: &concept::permission::Access,
    target: super::target::Target<'_>,
    uri: &http::Uri,
) -> super::response::Result<Response> {
    let axum::extract::Query(query) = axum::extract::Query::<Query>::try_from_uri(uri)
        .map_err(|error| crate::error::ErrorKind::InvalidBody(error.body_text()))?;
    let now = Timestamp::now();
    let preset = query.range.or_else(|| {
        (query.start.is_none() && query.end.is_none()).then_some(super::range::Range::H24)
    });
    let window = super::range::Window::preset(preset, query.start, query.end, now);
    window.validate(now)?;
    let kind: Option<Kind> = super::token(query.kind.as_deref())?;
    let input = operation::event::ListInput {
        query: operation::event::typ::EventQuery {
            start: Some(window.range.0),
            end: Some(window.range.1),
            kind,
            subject: None,
            skip: query.skip,
            limit: PAGE_SIZE,
        },
    };
    let conditions = super::conditions::load(app, access)
        .await
        .map(|conditions| match &target {
            super::target::Target::App(spec) => super::conditions::for_app(conditions, spec),
            _ => conditions,
        });
    let (dashboard, history) = tokio::try_join!(
        super::dashboard::Dashboard::load(app, access, target.location()),
        async {
            Ok::<_, crate::error::Error>(
                super::insight::optional_query(operation::execute::<operation::event::List>(
                    app,
                    access,
                    input,
                    Timestamp::now(),
                ))
                .await,
            )
        },
    )?;
    super::response::render(
        &Page {
            dashboard,
            target,
            can_edit: access.allows(concept::permission::Requirement::Operator),
            action: uri.path().to_owned(),
            window,
            kind: kind.map_or("", Kind::as_str),
            skip: query.skip,
            conditions,
            history: history
                .map_err(|error| super::read_error(&error, "event history query failed")),
        },
        StatusCode::OK,
    )
}

#[expect(clippy::trivially_copy_pass_by_ref)]
pub(super) fn kind_title(kind: &Kind) -> String {
    let key = format!("event.{}", kind.as_str());
    tt!(&key).into_owned()
}
