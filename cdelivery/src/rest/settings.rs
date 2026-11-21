use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use macros_derive::route;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::boot::App as AppState;
use crate::error::Result;
use crate::operation;

#[route(get, "/settings/agent", ok(operation::settings::typ::SettingsAgent))]
async fn find_agent(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::settings::GetAgent>,
) -> Result<Json<operation::settings::typ::SettingsAgent>> {
}

#[route(patch, "/settings/agent", ok(operation::settings::typ::AgentSettings))]
async fn patch_agent(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::settings::SetAgent>,
    super::extract::Body(dto): super::extract::Body<
        operation::settings::typ::UpdateAgentSettingsRequest,
    >,
) -> Result<Json<operation::settings::typ::AgentSettings>> {
}

#[route(delete, "/settings/agent", no_content)]
async fn delete_agent(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::settings::ClearAgent>,
) -> Result<StatusCode> {
}

#[route(
    post,
    "/settings/agent/models",
    ok(operation::settings::typ::AgentModels)
)]
async fn discover_models(
    State(state): State<Arc<AppState>>,
    authorized: super::extract::Authorized<operation::settings::DiscoverModels>,
    super::extract::Body(dto): super::extract::Body<
        operation::settings::typ::DiscoverAgentModelsRequest,
    >,
) -> Result<Json<operation::settings::typ::AgentModels>> {
}

macro_rules! channel_routes {
    ($($channel:ident {
        get { $get:ident, $wrapper:ident, $($get_rest:tt)* }
        set { $set:ident, $store:ident, $request:ident, $response:ident, $($set_rest:tt)* }
        clear { $clear:ident, $($clear_rest:tt)* }
        path $path:tt
    })+) => {
        $(mod $channel {
            use std::sync::Arc;

            use axum::Json;
            use axum::extract::State;
            use axum::http::StatusCode;
            use macros_derive::route;

            use crate::boot::App as AppState;
            use crate::error::Result;
            use crate::operation;

            #[route(get, $path, ok(operation::settings::typ::$wrapper))]
            pub(super) async fn find(
                State(state): State<Arc<AppState>>,
                authorized: super::super::extract::Authorized<operation::settings::$get>,
            ) -> Result<Json<operation::settings::typ::$wrapper>> {
            }

            #[route(patch, $path, ok(operation::settings::typ::$response))]
            pub(super) async fn patch(
                State(state): State<Arc<AppState>>,
                authorized: super::super::extract::Authorized<operation::settings::$set>,
                super::super::extract::Body(dto): super::super::extract::Body<
                    operation::settings::typ::$request,
                >,
            ) -> Result<Json<operation::settings::typ::$response>> {
            }

            #[route(delete, $path, no_content)]
            pub(super) async fn delete(
                State(state): State<Arc<AppState>>,
                authorized: super::super::extract::Authorized<operation::settings::$clear>,
            ) -> Result<StatusCode> {
            }
        })+

        pub(crate) fn routes() -> OpenApiRouter<Arc<AppState>> {
            OpenApiRouter::new()
                .routes(routes!(find_agent, patch_agent, delete_agent))
                .routes(routes!(discover_models))
                $(.routes(routes!($channel::find, $channel::patch, $channel::delete)))+
        }
    };
}

operation::settings::channels!(channel_routes);
