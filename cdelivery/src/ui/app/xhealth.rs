use std::sync::Arc;

use askama::Template;
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use serde::Deserialize;

use crate::locale::tt;
use crate::{concept, operation};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Form<T> {
    spec: T,
    command_line: Option<String>,
}

#[derive(Template)]
#[template(path = "app/xhealth.html")]
struct Page {
    dashboard: super::super::dashboard::Dashboard,
    name: String,
    description: Option<String>,
    domains: Vec<operation::app::typ::mutation::DomainSpec>,
    ports: Vec<operation::app::typ::network::Port>,
    can_edit: bool,
    current_version: Option<String>,
    target_version: Option<String>,
    xhealth: Option<operation::xhealth::typ::XHealth>,
    command_line: Option<String>,
}

pub(super) fn routes() -> axum::Router<Arc<crate::boot::App>> {
    axum::Router::new()
        .route("/apps/{name}/xhealth", axum::routing::get(get))
        .route("/apps/{name}/xhealth/create", post(create))
        .route("/apps/{name}/xhealth/update", post(update))
        .route(
            "/apps/{name}/xhealth/run",
            post(action::<operation::xhealth::Run>),
        )
        .route(
            "/apps/{name}/xhealth/disable",
            post(action::<operation::xhealth::Disable>),
        )
}

async fn get(
    authorized: super::super::auth::Authorized<operation::xhealth::Get>,
    State(app): State<Arc<crate::boot::App>>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> super::super::response::Result<Response> {
    let spec = super::load_app(&app, authorized.access(), name).await?;
    let xhealth = match authorized
        .execute(
            &app,
            operation::input::App {
                app: spec.name.clone(),
            },
        )
        .await
    {
        Ok(xhealth) => Some(xhealth),
        Err(error) if matches!(error.kind(), crate::error::ErrorKind::XHealthNotEnabled) => None,
        Err(error) => return Err(error.into()),
    };
    let (current_version, command_line) = match xhealth.as_ref().map(|value| &value.method) {
        Some(operation::xhealth::typ::XHealthMethod::Hurl { image }) => {
            (Some(image.tag().clone()), None)
        }
        Some(operation::xhealth::typ::XHealthMethod::Custom { command }) => {
            (None, Some(super::join_command(command)?))
        }
        None => (None, None),
    };
    let can_edit = authorized
        .access()
        .allows(concept::permission::Requirement::Operator);
    let (dashboard, [latest_version]) = tokio::join!(
        super::dashboard(&app, authorized.access(), &spec.name),
        super::super::latest_versions(
            &app,
            authorized.access(),
            can_edit,
            [concept::release::Component::Hurl],
        ),
    );
    let target_version = super::super::target_version(current_version.as_deref(), latest_version)?;
    super::super::response::render(
        &Page {
            dashboard: dashboard?,
            name: spec.name,
            description: spec.description,
            domains: spec.domains,
            ports: spec.ports,
            can_edit,
            current_version,
            target_version,
            xhealth,
            command_line,
        },
        http::StatusCode::OK,
    )
}

async fn create(
    authorized: super::super::auth::Authorized<operation::xhealth::Create>,
    State(app): State<Arc<crate::boot::App>>,
    axum::extract::Path(name): axum::extract::Path<String>,
    super::super::command::Payload(mut form): super::super::command::Payload<
        Form<operation::xhealth::typ::CreateXHealthRequest>,
    >,
) -> super::super::response::Result<Response> {
    apply_method_command(&mut form.spec.method, form.command_line.as_deref())?;
    let input = operation::xhealth::CreateInput {
        app: name.clone(),
        spec: form.spec,
    };
    authorized.execute(&app, input).await?;
    Ok(super::super::router::redirect(&format!(
        "/apps/{name}/xhealth"
    )))
}

async fn update(
    authorized: super::super::auth::Authorized<operation::xhealth::Update>,
    State(app): State<Arc<crate::boot::App>>,
    axum::extract::Path(name): axum::extract::Path<String>,
    super::super::command::Payload(mut form): super::super::command::Payload<
        Form<operation::xhealth::typ::UpdateXHealthRequest>,
    >,
) -> super::super::response::Result<Response> {
    if let Some(method) = form.spec.method.as_mut() {
        apply_method_command(method, form.command_line.as_deref())?;
    }
    let input = operation::xhealth::UpdateInput {
        app: name.clone(),
        changes: form.spec,
    };
    authorized.execute(&app, input).await?;
    Ok(super::super::router::redirect(&format!(
        "/apps/{name}/xhealth"
    )))
}

async fn action<O>(
    authorized: super::super::auth::Authorized<O>,
    State(app): State<Arc<crate::boot::App>>,
    axum::extract::Path(name): axum::extract::Path<String>,
    super::super::command::Payload(_): super::super::command::Payload<operation::input::Empty>,
) -> super::super::response::Result<Response>
where
    O: operation::catalog::Catalogued<Input = operation::input::App>,
{
    authorized
        .execute(&app, operation::input::App { app: name.clone() })
        .await?;
    Ok(super::super::router::redirect(&format!(
        "/apps/{name}/xhealth"
    )))
}

fn apply_method_command(
    method: &mut operation::xhealth::typ::XHealthMethodInput,
    command_line: Option<&str>,
) -> crate::error::Result<()> {
    match method {
        operation::xhealth::typ::XHealthMethodInput::Hurl { .. } => Ok(()),
        operation::xhealth::typ::XHealthMethodInput::Custom { command } => {
            *command = super::required_command(command_line.unwrap_or_default())?;
            Ok(())
        }
    }
}
