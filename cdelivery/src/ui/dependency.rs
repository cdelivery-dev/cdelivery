use std::sync::Arc;

use askama::Template;
use axum::{extract::State, response::Response};
use http::StatusCode;

use crate::boot::App;
use crate::error::ErrorKind;
use crate::locale::tt;
use crate::operation;

#[derive(Template)]
#[template(path = "dependency/list.html")]
struct ListPage {
    dashboard: super::dashboard::Dashboard,
    dependencies: Vec<operation::dependency::typ::Dependency>,
}

#[derive(Template)]
#[template(path = "dependency/create.html")]
struct CreatePage {
    dashboard: super::dashboard::Dashboard,
    provider: operation::dependency::typ::Provider,
    apps: Vec<operation::app::typ::view::AppSummary>,
}

#[derive(Template)]
#[template(path = "dependency/detail.html")]
struct DetailPage {
    dashboard: super::dashboard::Dashboard,
    dependency: operation::dependency::typ::Dependency,
    latest: Option<operation::change::typ::Change>,
    changes_error: Option<String>,
    sizes: Vec<operation::dependency::typ::PlanetScaleSize>,
    sizes_error: Option<String>,
}

impl DetailPage {
    fn active(&self) -> bool {
        self.latest
            .as_ref()
            .is_some_and(operation::change::typ::Change::active)
    }
}

pub(super) fn routes() -> axum::Router<Arc<App>> {
    axum::Router::new()
        .route("/dependencies", axum::routing::get(list).post(create))
        .route(
            "/dependencies/.create",
            axum::routing::get(
                |_: super::auth::Authorized<operation::dependency::Create>| async {
                    super::router::redirect("/dependencies/.create/neon")
                },
            ),
        )
        .route(
            "/dependencies/.create/{provider}",
            axum::routing::get(create_form),
        )
        .route("/dependencies/{name}", axum::routing::get(get))
        .route("/dependencies/{name}/delete", axum::routing::post(delete))
}

async fn list(
    State(app): State<Arc<App>>,
    authorized: super::auth::Authorized<operation::dependency::List>,
) -> super::response::Result<Response> {
    let (dependencies, dashboard) = tokio::try_join!(
        authorized.execute(&app, operation::dependency::typ::Query { group: None }),
        super::dashboard::Dashboard::load(
            &app,
            authorized.access(),
            super::dashboard::Location::Dependencies
        ),
    )?;
    super::response::render(
        &ListPage {
            dashboard,
            dependencies: dependencies.dependencies,
        },
        StatusCode::OK,
    )
}

async fn create_form(
    State(app): State<Arc<App>>,
    authorized: super::auth::Authorized<operation::dependency::Create>,
    axum::extract::Path(provider): axum::extract::Path<String>,
) -> super::response::Result<Response> {
    let provider = match provider.as_str() {
        "neon" => operation::dependency::typ::Provider::Neon,
        "planetscale" => operation::dependency::typ::Provider::PlanetScale,
        "postgresql" => operation::dependency::typ::Provider::Local,
        _ => return Err(ErrorKind::RouteNotFound.into()),
    };
    let (dashboard, apps) = tokio::try_join!(
        super::dashboard::Dashboard::load(
            &app,
            authorized.access(),
            super::dashboard::Location::Dependencies,
        ),
        async {
            if matches!(provider, operation::dependency::typ::Provider::Local) {
                super::read::<operation::app::List>(&app, authorized.access())
                    .await
                    .map(|apps| apps.apps)
            } else {
                Ok(Vec::new())
            }
        },
    )?;
    super::response::render(
        &CreatePage {
            dashboard,
            provider,
            apps,
        },
        StatusCode::OK,
    )
}

async fn get(
    State(app): State<Arc<App>>,
    authorized: super::auth::Authorized<operation::dependency::Get>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> super::response::Result<Response> {
    let dependency = authorized
        .execute(&app, operation::input::Name { name })
        .await?;
    let (dashboard, changes) = tokio::join!(
        super::dashboard::Dashboard::load(
            &app,
            authorized.access(),
            super::dashboard::Location::Dependencies,
        ),
        operation::execute::<operation::change::List>(
            &app,
            authorized.access(),
            operation::input::Empty {},
            jiff::Timestamp::now(),
        ),
    );
    let (latest, changes_error) = match changes {
        Ok(changes) => {
            let latest = changes.changes.into_iter().find(|change| {
                matches!(
                    &change.target,
                    operation::change::typ::Target::PostgreSQL { dependency: reference, .. }
                        if reference.id == dependency.id
                )
            });
            (latest, None)
        }
        Err(error) => (
            None,
            Some(super::read_error(
                &error,
                &format!("dependency {} changes unavailable", dependency.name),
            )),
        ),
    };
    let dashboard = dashboard?;
    let (sizes, sizes_error) = if matches!(
        dependency.provider,
        operation::dependency::typ::Provider::PlanetScale
    ) && dashboard.can_create_app
        && changes_error.is_none()
        && latest.as_ref().is_none_or(|change| !change.active())
    {
        match operation::execute::<operation::dependency::PlanetScaleSizes>(
            &app,
            authorized.access(),
            operation::dependency::typ::PlanetScaleSizesQuery {
                dependency: Some(dependency.name.clone()),
            },
            jiff::Timestamp::now(),
        )
        .await
        {
            Ok(sizes) => (sizes.sizes, None),
            Err(error) => (
                Vec::new(),
                Some(super::read_error(&error, "PlanetScale sizes unavailable")),
            ),
        }
    } else {
        (Vec::new(), None)
    };
    super::response::render(
        &DetailPage {
            dashboard,
            dependency,
            latest,
            changes_error,
            sizes,
            sizes_error,
        },
        StatusCode::OK,
    )
}

async fn create(
    State(app): State<Arc<App>>,
    authorized: super::auth::Authorized<operation::dependency::Create>,
    super::command::Payload(payload): super::command::Payload<
        operation::dependency::typ::CreateRequest,
    >,
) -> super::response::Result<Response> {
    authorized.execute(&app, payload).await?;
    Ok(super::router::redirect("/dependencies"))
}

async fn delete(
    State(app): State<Arc<App>>,
    authorized: super::auth::Authorized<operation::dependency::Delete>,
    axum::extract::Path(name): axum::extract::Path<String>,
    super::command::Payload(_): super::command::Payload<operation::input::Empty>,
) -> super::response::Result<Response> {
    authorized
        .execute(&app, operation::input::Name { name })
        .await?;
    Ok(super::router::redirect("/dependencies"))
}
