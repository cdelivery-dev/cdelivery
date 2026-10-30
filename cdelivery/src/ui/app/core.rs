use std::sync::Arc;

use askama::Template;
use axum::extract::{OriginalUri, State};
use axum::response::Response;
use http::StatusCode;
use jiff::Timestamp;

use crate::boot::App;
use crate::concept;
use crate::error::ErrorKind;
use crate::locale::tt;
use crate::operation;

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct CreateForm {
    request: operation::app::typ::mutation::CreateAppRequest,
    #[serde(default)]
    command_line: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateForm {
    changes: operation::app::typ::mutation::Changes,
    #[serde(default)]
    command_line: Option<String>,
}

#[derive(Template)]
#[template(path = "app/create.html")]
struct CreatePage {
    dashboard: super::super::dashboard::Dashboard,
    group: String,
    form: CreateForm,
    nodes: operation::node::typ::Nodes,
    template: Option<&'static super::template::Database>,
}

#[derive(Template)]
#[template(path = "app/overview.html")]
struct OverviewPage {
    dashboard: super::super::dashboard::Dashboard,
    spec: operation::app::typ::view::AppSpec,
    instances: std::result::Result<operation::overview::typ::ServiceInstances, String>,
    guardian: std::result::Result<Option<operation::guardian::typ::Guardian>, String>,
    metrics: std::result::Result<super::super::metrics::MetricsState, String>,
    conditions: super::super::conditions::Result,
    window: super::super::range::Window,
    attribution: Option<String>,
    command_line: String,
    can_edit: bool,
}

impl OverviewPage {
    fn guardian(&self) -> Option<&operation::guardian::typ::Guardian> {
        self.guardian.as_ref().ok().and_then(Option::as_ref)
    }

    fn rollback_available(&self) -> bool {
        matches!(
            self.spec.rollback,
            Some(operation::app::typ::view::Rollback::Available(_))
        )
    }
}

#[derive(Template)]
#[template(path = "app/spec.html")]
struct SpecPage {
    dashboard: super::super::dashboard::Dashboard,
    spec: operation::app::typ::view::AppSpec,
    command_line: String,
    nodes: operation::node::typ::Nodes,
}

impl SpecPage {
    fn mount_node_label(&self) -> String {
        self.spec
            .mounts
            .as_ref()
            .map_or_else(String::new, |mounts| {
                let id = &mounts.node_id;
                self.nodes
                    .nodes
                    .iter()
                    .find(|node| &node.id == id)
                    .map_or_else(
                        || id.clone(),
                        |node| {
                            let addr = if node.addr.is_empty() {
                                "—"
                            } else {
                                &node.addr
                            };
                            format!("{} ({addr})", node.hostname)
                        },
                    )
            })
    }
}

pub(super) fn routes() -> axum::Router<Arc<App>> {
    axum::Router::new()
        .route(
            "/groups/{group}/apps/.create",
            axum::routing::get(create_form),
        )
        .route(
            "/groups/{group}/apps/.template/{template}",
            axum::routing::get(template_form),
        )
        .route("/apps", axum::routing::post(create))
        .route("/apps/{name}", axum::routing::get(get))
        .route("/apps/{name}/spec", axum::routing::get(spec).post(update))
        .route("/apps/{name}/delete", axum::routing::post(delete))
        .route("/apps/{name}/restart", axum::routing::post(restart))
        .route("/apps/{name}/rollback", axum::routing::post(rollback))
        .route(
            "/apps/{name}/guardian/retry",
            axum::routing::post(guardian_retry),
        )
        .route(
            "/apps/{name}/guardian/skip",
            axum::routing::post(guardian_skip),
        )
        .route(
            "/apps/{name}/guardian/cancel",
            axum::routing::post(guardian_cancel),
        )
}

async fn create_form(
    authorized: super::super::auth::Authorized<operation::app::Create>,
    State(app): State<Arc<App>>,
    axum::extract::Path(group): axum::extract::Path<String>,
) -> super::super::response::Result<Response> {
    render_create(&app, &authorized, group, None).await
}

async fn template_form(
    authorized: super::super::auth::Authorized<operation::app::Create>,
    State(app): State<Arc<App>>,
    axum::extract::Path((group, template)): axum::extract::Path<(String, String)>,
) -> super::super::response::Result<Response> {
    let database = super::template::DATABASES
        .iter()
        .find(|database| database.id == template)
        .ok_or(ErrorKind::RouteNotFound)?;
    render_create(&app, &authorized, group, Some(database)).await
}

async fn render_create(
    app: &App,
    authorized: &super::super::auth::Authorized<operation::app::Create>,
    group: String,
    template: Option<&'static super::template::Database>,
) -> super::super::response::Result<Response> {
    let (dashboard, nodes) = tokio::try_join!(
        super::super::dashboard::Dashboard::load(
            app,
            authorized.access(),
            super::super::dashboard::Location::AppCreate(group.clone()),
        ),
        operation::execute::<operation::node::List>(
            app,
            authorized.access(),
            operation::input::Empty {},
            Timestamp::now(),
        ),
    )?;
    if !dashboard.groups.iter().any(|link| link.name == group) {
        return Err(ErrorKind::GroupNotFound.into());
    }
    let form = match template {
        Some(database) => template_create_form(group.clone(), database)?,
        None => initial_create_form(group.clone())?,
    };
    super::super::response::render(
        &CreatePage {
            dashboard,
            group,
            form,
            nodes,
            template,
        },
        StatusCode::OK,
    )
}

async fn create(
    authorized: super::super::auth::Authorized<operation::app::Create>,
    State(app): State<Arc<App>>,
    super::super::command::Payload(mut form): super::super::command::Payload<CreateForm>,
) -> super::super::response::Result<Response> {
    form.request
        .set_command(optional_command(&form.command_line)?);
    let created = authorized.execute(&app, form.request).await?;
    Ok(super::super::router::redirect(&format!(
        "/apps/{}",
        created.name
    )))
}

async fn get(
    authorized: super::super::auth::Authorized<operation::app::Get>,
    State(app): State<Arc<App>>,
    axum::extract::Path(name): axum::extract::Path<String>,
    OriginalUri(uri): OriginalUri,
) -> super::super::response::Result<Response> {
    let spec = authorized
        .execute(&app, operation::input::Name { name })
        .await?;
    let command_line = command_text(&spec)?;
    let app_name = spec.name.clone();
    let now = Timestamp::now();
    let window = super::super::range::Window::page(&uri, now)?;
    window.validate(now)?;
    let attribution = describe_operator(spec.rollback.as_ref());
    let (dashboard, guardian, metrics, conditions, status) = tokio::join!(
        super::dashboard(&app, authorized.access(), &spec.name),
        operation::execute::<operation::guardian::Get>(
            &app,
            authorized.access(),
            operation::input::Name { name: app_name },
            now,
        ),
        super::super::metrics::page::app_overview(
            &app,
            authorized.access(),
            &spec.name,
            !spec.domains.is_empty(),
            window.range,
            now,
        ),
        super::super::conditions::load(&app, authorized.access()),
        operation::execute::<operation::app::Status>(
            &app,
            authorized.access(),
            operation::input::Name {
                name: spec.name.clone(),
            },
            now,
        ),
    );
    let conditions =
        conditions.map(|conditions| super::super::conditions::for_app(conditions, &spec));
    let guardian = match guardian {
        Ok(guardian) => Ok(Some(guardian)),
        Err(error) if matches!(error.kind(), ErrorKind::GuardianNotFound) => Ok(None),
        Err(error) => Err(super::super::read_error(
            &error,
            &format!("app {} guardian unavailable", spec.name),
        )),
    };
    let metrics = metrics.map_err(|error| {
        super::super::read_error(
            &error,
            &format!("app {} overview metrics unavailable", spec.name),
        )
    });
    let instances = status.map(|status| status.instances).map_err(|error| {
        super::super::read_error(&error, &format!("app {} status unavailable", spec.name))
    });
    super::super::response::render(
        &OverviewPage {
            dashboard: dashboard?,
            spec,
            instances,
            guardian,
            metrics,
            conditions,
            window,
            attribution,
            command_line,
            can_edit: authorized
                .access()
                .allows(concept::permission::Requirement::Operator),
        },
        StatusCode::OK,
    )
}

async fn spec(
    authorized: super::super::auth::Authorized<operation::app::Update>,
    State(app): State<Arc<App>>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> super::super::response::Result<Response> {
    let spec = super::load_app(&app, authorized.access(), name).await?;
    let command_line = command_text(&spec)?;
    let (dashboard, nodes) = tokio::try_join!(
        super::dashboard(&app, authorized.access(), &spec.name),
        operation::execute::<operation::node::List>(
            &app,
            authorized.access(),
            operation::input::Empty {},
            Timestamp::now(),
        ),
    )?;
    super::super::response::render(
        &SpecPage {
            dashboard,
            spec,
            command_line,
            nodes,
        },
        StatusCode::OK,
    )
}

async fn update(
    authorized: super::super::auth::Authorized<operation::app::Update>,
    State(app): State<Arc<App>>,
    axum::extract::Path(name): axum::extract::Path<String>,
    super::super::command::Payload(mut form): super::super::command::Payload<UpdateForm>,
) -> super::super::response::Result<Response> {
    if let Some(command_line) = form.command_line {
        form.changes.set_command(optional_command(&command_line)?);
    }
    authorized
        .execute(
            &app,
            operation::app::UpdateInput {
                name: name.clone(),
                changes: form.changes,
            },
        )
        .await?;
    Ok(super::super::router::redirect(&format!("/apps/{name}")))
}

async fn delete(
    authorized: super::super::auth::Authorized<operation::app::Delete>,
    State(app): State<Arc<App>>,
    axum::extract::Path(name): axum::extract::Path<String>,
    super::super::command::Payload(_): super::super::command::Payload<operation::input::Empty>,
) -> super::super::response::Result<Response> {
    authorized
        .execute(&app, operation::input::Name { name })
        .await?;
    Ok(super::super::router::redirect("/groups"))
}

async fn restart(
    authorized: super::super::auth::Authorized<operation::app::Restart>,
    State(app): State<Arc<App>>,
    axum::extract::Path(name): axum::extract::Path<String>,
    super::super::command::Payload(_): super::super::command::Payload<operation::input::Empty>,
) -> super::super::response::Result<Response> {
    action::<operation::app::Restart>(&app, &authorized, name).await
}

async fn rollback(
    authorized: super::super::auth::Authorized<operation::app::Rollback>,
    State(app): State<Arc<App>>,
    axum::extract::Path(name): axum::extract::Path<String>,
    super::super::command::Payload(_): super::super::command::Payload<operation::input::Empty>,
) -> super::super::response::Result<Response> {
    action::<operation::app::Rollback>(&app, &authorized, name).await
}

async fn guardian_retry(
    authorized: super::super::auth::Authorized<operation::guardian::Retry>,
    State(app): State<Arc<App>>,
    axum::extract::Path(name): axum::extract::Path<String>,
    super::super::command::Payload(_): super::super::command::Payload<operation::input::Empty>,
) -> super::super::response::Result<Response> {
    action::<operation::guardian::Retry>(&app, &authorized, name).await
}

async fn guardian_skip(
    authorized: super::super::auth::Authorized<operation::guardian::Skip>,
    State(app): State<Arc<App>>,
    axum::extract::Path(name): axum::extract::Path<String>,
    super::super::command::Payload(_): super::super::command::Payload<operation::input::Empty>,
) -> super::super::response::Result<Response> {
    action::<operation::guardian::Skip>(&app, &authorized, name).await
}

async fn guardian_cancel(
    authorized: super::super::auth::Authorized<operation::guardian::Cancel>,
    State(app): State<Arc<App>>,
    axum::extract::Path(name): axum::extract::Path<String>,
    super::super::command::Payload(_): super::super::command::Payload<operation::input::Empty>,
) -> super::super::response::Result<Response> {
    action::<operation::guardian::Cancel>(&app, &authorized, name).await
}

async fn action<O>(
    app: &App,
    authorized: &super::super::auth::Authorized<O>,
    name: String,
) -> super::super::response::Result<Response>
where
    O: operation::Operation<Input = operation::input::Name> + operation::catalog::Catalogued,
{
    authorized
        .execute(app, operation::input::Name { name: name.clone() })
        .await?;
    Ok(super::super::router::redirect(&format!("/apps/{name}")))
}

fn initial_create_form(group: String) -> crate::error::Result<CreateForm> {
    Ok(CreateForm {
        request: operation::app::typ::mutation::CreateAppRequest {
            name: String::new(),
            description: None,
            group,
            image: String::new(),
            image_auth: None,
            replicas: operation::app::typ::replicas::Replicas::Fixed(1),
            ports: Vec::new(),
            domains: Vec::new(),
            xconfigs: Vec::new(),
            mounts: None,
            container_options: None,
            resources: None,
            placement: None,
            log_driver: Some((&concept::app::LogDriver::default()).try_into()?),
            restart_policy: Some(concept::app::RestartPolicy::default().try_into()?),
            update_config: Some(concept::app::UpdateConfig::default().try_into()?),
        },
        command_line: String::new(),
    })
}

fn template_create_form(
    group: String,
    database: &super::template::Database,
) -> crate::error::Result<CreateForm> {
    let mut form = initial_create_form(group)?;
    let request = &mut form.request;
    database.image.clone_into(&mut request.image);
    request.ports = vec![operation::app::typ::network::Port {
        target: database.port,
        published: database.port,
        protocol: operation::app::typ::network::PortProtocol::Tcp,
    }];
    request.mounts = Some(operation::app::typ::container::NodeMounts {
        node_id: String::new(),
        mounts: Vec::new(),
    });
    let password = operation::app::typ::container::EnvVar {
        name: database.password.to_owned(),
        value: random_password()?,
    };
    request.container_options = Some(operation::app::typ::container::ContainerOptions {
        command: Vec::new(),
        env: std::iter::once(password)
            .chain(database.env.iter().map(|(name, value)| {
                operation::app::typ::container::EnvVar {
                    name: (*name).to_owned(),
                    value: (*value).to_owned(),
                }
            }))
            .collect(),
    });
    Ok(form)
}

fn random_password() -> crate::error::Result<String> {
    let mut bytes = [0u8; 15];
    getrandom::fill(&mut bytes)?;
    Ok(base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        bytes,
    ))
}

fn optional_command(value: &str) -> crate::error::Result<Vec<String>> {
    super::split_command(value).ok_or_else(|| {
        let mut report = garde::Report::new();
        report.append(
            garde::Path::new("command_line"),
            garde::Error::new(tt!("ui.app.container.command.invalid").into_owned()),
        );
        report.into()
    })
}

fn command_text(spec: &operation::app::typ::view::AppSpec) -> crate::error::Result<String> {
    let command = spec
        .container_options
        .as_ref()
        .map_or(&[][..], |options| options.command.as_slice());
    super::join_command(command)
}

fn describe_operator(rollback: Option<&operation::app::typ::view::Rollback>) -> Option<String> {
    let change = rollback?.change()?;
    let name = change
        .operator_name
        .clone()
        .or_else(|| change.operator.user_id().map(|id| id.to_string()))
        .unwrap_or_default();
    Some(
        match change.operator {
            operation::app::typ::view::Operator::User { .. } => {
                tt!("ui.app.changes.byUser", name = name)
            }
            operation::app::typ::view::Operator::ApiKey { .. } => {
                tt!("ui.app.changes.byApiKey", name = name)
            }
            operation::app::typ::view::Operator::Agent { .. } => {
                tt!("ui.app.changes.byAgent", name = name)
            }
            operation::app::typ::view::Operator::Duty => tt!("ui.app.changes.byDuty"),
        }
        .into_owned(),
    )
}
