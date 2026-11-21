use std::sync::Arc;

use axum::Extension;
use axum::extract::{OriginalUri, Path, State};
use axum::response::Response;

use crate::{concept, operation};

pub(super) enum Target<'a> {
    App(&'a operation::app::typ::view::AppSpec),
    Insight,
    Pingress,
}

impl Target<'_> {
    pub(super) fn location(&self) -> super::dashboard::Location {
        match self {
            Self::App(spec) => super::dashboard::Location::App(spec.name.clone()),
            Self::Insight => super::dashboard::Location::Insight,
            Self::Pingress => super::dashboard::Location::Pingress,
        }
    }
}

pub(super) fn routes() -> axum::Router<Arc<crate::boot::App>> {
    axum::Router::new()
        .route("/apps/{name}/events", axum::routing::get(get))
        .route("/insight/events", axum::routing::get(get))
        .route("/pingress/events", axum::routing::get(get))
        .route("/apps/{name}/metrics", axum::routing::get(get))
        .route("/insight/metrics", axum::routing::get(get))
        .route("/pingress/metrics", axum::routing::get(get))
}

async fn get(
    Extension(access): Extension<concept::permission::Access>,
    State(app): State<Arc<crate::boot::App>>,
    name: Option<Path<String>>,
    OriginalUri(uri): OriginalUri,
) -> super::response::Result<Response> {
    let now = jiff::Timestamp::now();
    let spec = match name {
        Some(Path(name)) => Some(super::app::load_app(&app, &access, name).await?),
        None => None,
    };
    let target = match spec.as_ref() {
        Some(spec) => Target::App(spec),
        None if uri.path().starts_with("/insight/") => Target::Insight,
        None => Target::Pingress,
    };
    if uri.path().ends_with("/events") {
        super::events::render(&app, &access, target, &uri).await
    } else {
        super::metrics::page::render(&app, &access, target, &uri, now).await
    }
}
