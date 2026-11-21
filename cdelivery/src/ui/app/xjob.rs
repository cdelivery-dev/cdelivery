use std::sync::Arc;

use askama::Template;
use axum::extract::State;
use axum::response::Response;
use http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::locale::tt;
use crate::{concept, operation};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateForm {
    job: String,
    spec: operation::xjob::typ::CreateXJobRequest,
    command_line: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateForm {
    changes: operation::xjob::typ::UpdateXJobRequest,
    command_line: Option<String>,
}

#[derive(Template)]
#[template(path = "app/xjob/list.html")]
struct ListPage {
    dashboard: super::super::dashboard::Dashboard,
    name: String,
    description: Option<String>,
    can_edit: bool,
    jobs: Vec<operation::xjob::typ::XJob>,
}

#[derive(Template)]
#[template(path = "app/xjob/create.html")]
struct CreatePage {
    dashboard: super::super::dashboard::Dashboard,
    name: String,
    form: XJobForm<'static>,
}

#[derive(Template)]
#[template(path = "app/xjob/detail.html")]
struct DetailPage {
    dashboard: super::super::dashboard::Dashboard,
    name: String,
    can_edit: bool,
    job: operation::xjob::typ::XJob,
    command_line: String,
}

impl DetailPage {
    fn form(&self) -> XJobForm<'_> {
        XJobForm {
            interval: self.job.interval,
            timeout: self.job.timeout,
            command_line: &self.command_line,
        }
    }
}

#[derive(Serialize)]
struct XJobForm<'a> {
    interval: operation::typ::Duration,
    timeout: operation::typ::Duration,
    command_line: &'a str,
}

pub(super) fn routes() -> axum::Router<Arc<crate::boot::App>> {
    axum::Router::new()
        .route("/apps/{name}/xjobs", axum::routing::get(list).post(create))
        .route(
            "/apps/{name}/xjobs/.create",
            axum::routing::get(create_form),
        )
        .route("/apps/{name}/xjobs/{job}", axum::routing::get(get))
        .route(
            "/apps/{name}/xjobs/{job}/update",
            axum::routing::post(update),
        )
        .route("/apps/{name}/xjobs/{job}/run", axum::routing::post(run))
        .route(
            "/apps/{name}/xjobs/{job}/delete",
            axum::routing::post(delete),
        )
}

async fn list(
    authorized: super::super::auth::Authorized<operation::xjob::List>,
    State(app): State<Arc<crate::boot::App>>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> super::super::response::Result<Response> {
    let spec = super::load_app(&app, authorized.access(), name).await?;
    let jobs = authorized
        .execute(
            &app,
            operation::input::App {
                app: spec.name.clone(),
            },
        )
        .await?
        .jobs;
    let can_edit = authorized
        .access()
        .allows(concept::permission::Requirement::Operator);
    let dashboard = super::dashboard(&app, authorized.access(), &spec.name).await?;
    super::super::response::render(
        &ListPage {
            dashboard,
            name: spec.name,
            description: spec.description,
            can_edit,
            jobs,
        },
        StatusCode::OK,
    )
}

async fn create_form(
    authorized: super::super::auth::Authorized<operation::xjob::Create>,
    State(app): State<Arc<crate::boot::App>>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> super::super::response::Result<Response> {
    let spec = super::load_app(&app, authorized.access(), name).await?;
    let dashboard = super::dashboard(&app, authorized.access(), &spec.name).await?;
    super::super::response::render(
        &CreatePage {
            dashboard,
            name: spec.name,
            form: XJobForm {
                interval: operation::typ::Duration::Minutes(30),
                timeout: operation::typ::Duration::Minutes(30),
                command_line: "",
            },
        },
        StatusCode::OK,
    )
}

async fn create(
    authorized: super::super::auth::Authorized<operation::xjob::Create>,
    State(app): State<Arc<crate::boot::App>>,
    axum::extract::Path(name): axum::extract::Path<String>,
    super::super::command::Payload(mut form): super::super::command::Payload<CreateForm>,
) -> super::super::response::Result<Response> {
    form.spec.command = super::required_command(&form.command_line)?;
    let input = operation::xjob::CreateInput {
        app: name.clone(),
        job: form.job,
        spec: form.spec,
    };
    authorized.execute(&app, input).await?;
    Ok(super::super::router::redirect(&format!(
        "/apps/{name}/xjobs"
    )))
}

async fn get(
    authorized: super::super::auth::Authorized<operation::xjob::Get>,
    State(app): State<Arc<crate::boot::App>>,
    axum::extract::Path((name, job)): axum::extract::Path<(String, String)>,
) -> super::super::response::Result<Response> {
    let spec = super::load_app(&app, authorized.access(), name).await?;
    let job = authorized
        .execute(
            &app,
            operation::input::AppJob {
                app: spec.name.clone(),
                job,
            },
        )
        .await?;
    let can_edit = authorized
        .access()
        .allows(concept::permission::Requirement::Operator);
    let dashboard = super::dashboard(&app, authorized.access(), &spec.name).await?;
    let command_line = super::join_command(&job.command)?;
    super::super::response::render(
        &DetailPage {
            dashboard,
            name: spec.name,
            can_edit,
            job,
            command_line,
        },
        StatusCode::OK,
    )
}

async fn update(
    authorized: super::super::auth::Authorized<operation::xjob::Update>,
    State(app): State<Arc<crate::boot::App>>,
    axum::extract::Path((name, job)): axum::extract::Path<(String, String)>,
    super::super::command::Payload(mut form): super::super::command::Payload<UpdateForm>,
) -> super::super::response::Result<Response> {
    form.changes.command = form
        .command_line
        .as_deref()
        .map(super::required_command)
        .transpose()?;
    let location = format!("/apps/{name}/xjobs/{job}");
    let input = operation::xjob::UpdateInput {
        app: name,
        job,
        changes: form.changes,
    };
    authorized.execute(&app, input).await?;
    Ok(super::super::router::redirect(&location))
}

async fn run(
    authorized: super::super::auth::Authorized<operation::xjob::Run>,
    State(app): State<Arc<crate::boot::App>>,
    axum::extract::Path((name, job)): axum::extract::Path<(String, String)>,
    super::super::command::Payload(_): super::super::command::Payload<operation::input::Empty>,
) -> super::super::response::Result<Response> {
    authorized
        .execute(
            &app,
            operation::input::AppJob {
                app: name.clone(),
                job,
            },
        )
        .await?;
    Ok(super::super::router::redirect(&format!(
        "/apps/{name}/xjobs"
    )))
}

async fn delete(
    authorized: super::super::auth::Authorized<operation::xjob::Delete>,
    State(app): State<Arc<crate::boot::App>>,
    axum::extract::Path((name, job)): axum::extract::Path<(String, String)>,
    super::super::command::Payload(_): super::super::command::Payload<operation::input::Empty>,
) -> super::super::response::Result<Response> {
    authorized
        .execute(
            &app,
            operation::input::AppJob {
                app: name.clone(),
                job,
            },
        )
        .await?;
    Ok(super::super::router::redirect(&format!(
        "/apps/{name}/xjobs"
    )))
}
