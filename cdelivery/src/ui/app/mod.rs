use std::sync::Arc;

use crate::locale::tt;
use crate::operation;

pub(super) mod core;
pub(super) mod logs;
pub(super) mod template;
pub(super) mod xhealth;
pub(super) mod xjob;

pub(super) fn routes() -> axum::Router<Arc<crate::boot::App>> {
    axum::Router::new()
        .merge(core::routes())
        .merge(logs::routes())
        .merge(xhealth::routes())
        .merge(xjob::routes())
}

pub(super) async fn load_app(
    app: &crate::boot::App,
    access: &crate::concept::permission::Access,
    name: String,
) -> crate::error::Result<operation::app::typ::view::AppSpec> {
    operation::execute::<operation::app::Get>(
        app,
        access,
        operation::input::Name { name },
        jiff::Timestamp::now(),
    )
    .await
}

async fn dashboard(
    app: &crate::boot::App,
    access: &crate::concept::permission::Access,
    name: &str,
) -> crate::error::Result<super::dashboard::Dashboard> {
    super::dashboard::Dashboard::load(
        app,
        access,
        super::dashboard::Location::App(name.to_owned()),
    )
    .await
}

fn split_command(value: &str) -> Option<Vec<String>> {
    if value.contains('\0') {
        return None;
    }
    shlex::split(value)
}

fn join_command(command: &[String]) -> crate::error::Result<String> {
    shlex::try_join(command.iter().map(String::as_str))
        .map_err(|_| crate::error::ErrorKind::InvalidOperationOutput.into())
}

fn required_command(value: &str) -> crate::error::Result<Vec<String>> {
    if let Some(command) = split_command(value)
        && !command.is_empty()
    {
        return Ok(command);
    }
    let mut report = garde::Report::new();
    report.append(
        garde::Path::new("command_line"),
        garde::Error::new(tt!("ui.app.command.invalid").into_owned()),
    );
    Err(report.into())
}
