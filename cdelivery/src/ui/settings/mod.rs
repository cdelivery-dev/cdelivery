mod agent;
mod channel;
mod user;
mod version;

use std::sync::Arc;

use axum::extract::{OriginalUri, State};
use axum::response::Response;

use crate::boot::App;
use crate::operation;

pub(super) fn routes() -> axum::Router<Arc<App>> {
    axum::Router::new()
        .merge(agent::routes())
        .merge(channel::routes())
        .merge(user::routes())
        .merge(version::routes())
}

async fn set<O: operation::catalog::Catalogued>(
    authorized: super::auth::Authorized<O>,
    State(app): State<Arc<App>>,
    OriginalUri(uri): OriginalUri,
    super::command::Payload(input): super::command::Payload<O::Input>,
) -> super::response::Result<Response> {
    authorized.execute(&app, input).await?;
    Ok(super::router::redirect(uri.path()))
}

async fn clear<O>(
    authorized: super::auth::Authorized<O>,
    State(app): State<Arc<App>>,
    OriginalUri(uri): OriginalUri,
    super::command::Payload(_): super::command::Payload<operation::input::Empty>,
) -> super::response::Result<Response>
where
    O: operation::catalog::Catalogued<Input = operation::input::Empty>,
{
    authorized.execute(&app, operation::input::Empty {}).await?;
    Ok(super::router::redirect(
        uri.path().trim_end_matches("/clear"),
    ))
}
