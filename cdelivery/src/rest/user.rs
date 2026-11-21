use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use macros_derive::route;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::boot::App;
use crate::error::Result;
use crate::operation;

#[route(patch, "/me/preferences", no_content)]
async fn update_preferences(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::user::MeUpdatePreferences>,
    super::extract::Body(dto): super::extract::Body<operation::user::typ::UpdatePreferencesRequest>,
) -> Result<StatusCode> {
}

#[route(get, "/users", ok(operation::user::typ::Users))]
async fn list_users(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::user::List>,
) -> Result<Json<operation::user::typ::Users>> {
}

#[route(post, "/users", created(operation::user::typ::ManagedUser))]
async fn create_user(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::user::Create>,
    super::extract::Body(dto): super::extract::Body<operation::user::typ::CreateUserRequest>,
) -> Result<(StatusCode, Json<operation::user::typ::ManagedUser>)> {
}

#[route(get, "/users/{id:user id}", ok(operation::user::typ::ManagedUser))]
async fn find_user(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::user::Get>,
    super::extract::Path(id): super::extract::Path<Uuid>,
) -> Result<Json<operation::user::typ::ManagedUser>> {
}

#[route(patch, "/users/{id:user id}", ok(operation::user::typ::ManagedUser))]
async fn update_user(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::user::Update>,
    super::extract::Path(id): super::extract::Path<Uuid>,
    super::extract::Body(dto): super::extract::Body<operation::user::typ::UpdateUserRequest>,
) -> Result<Json<operation::user::typ::ManagedUser>> {
}

#[route(delete, "/users/{id:user id}", no_content)]
async fn delete_user(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::user::Delete>,
    super::extract::Path(id): super::extract::Path<Uuid>,
) -> Result<StatusCode> {
}

#[route(get, "/me", ok(operation::user::typ::SelfUser))]
async fn me(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::user::MeGet>,
) -> Result<Json<operation::user::typ::SelfUser>> {
}

#[route(patch, "/me", no_content)]
async fn update_me(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::user::MeUpdate>,
    super::extract::Body(dto): super::extract::Body<operation::user::typ::UpdateProfileRequest>,
) -> Result<StatusCode> {
}

#[route(put, "/me/password", no_content)]
async fn change_password(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::user::MeChangePassword>,
    super::extract::Body(dto): super::extract::Body<operation::user::typ::ChangePasswordRequest>,
) -> Result<StatusCode> {
}

#[route(get, "/me/totp/enrollment", ok(operation::user::typ::TotpEnrollment))]
async fn prepare_totp(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::user::MePrepareTotp>,
) -> Result<Json<operation::user::typ::TotpEnrollment>> {
}

#[route(post, "/me/totp", no_content)]
async fn enable_totp(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::user::MeEnableTotp>,
    super::extract::Body(dto): super::extract::Body<operation::user::typ::EnableTotpRequest>,
) -> Result<StatusCode> {
}

#[route(post, "/me/totp/disable", no_content)]
async fn disable_totp(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::user::MeDisableTotp>,
    super::extract::Body(dto): super::extract::Body<operation::user::typ::DisableTotpRequest>,
) -> Result<StatusCode> {
}

#[route(get, "/me/im", ok(operation::user::typ::ImBindings))]
async fn im_bindings(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::user::MeGetIm>,
) -> Result<Json<operation::user::typ::ImBindings>> {
}

#[route(
    get,
    "/me/im/{platform:IM platform}/profile",
    ok(operation::user::typ::ImProfile)
)]
async fn im_profile(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::user::MeGetImProfile>,
    super::extract::Path(platform): super::extract::Path<operation::typ::Im>,
) -> Result<Json<operation::user::typ::ImProfile>> {
}

#[route(put, "/me/im", no_content)]
async fn set_im(
    State(state): State<Arc<App>>,
    authorized: super::extract::Authorized<operation::user::MeSetIm>,
    super::extract::Body(dto): super::extract::Body<operation::user::typ::SetImRequest>,
) -> Result<StatusCode> {
}

pub(crate) fn routes() -> OpenApiRouter<Arc<App>> {
    OpenApiRouter::new()
        .routes(routes!(list_users, create_user))
        .routes(routes!(find_user, update_user, delete_user))
        .routes(routes!(me, update_me))
        .routes(routes!(change_password))
        .routes(routes!(prepare_totp))
        .routes(routes!(enable_totp))
        .routes(routes!(disable_totp))
        .routes(routes!(update_preferences))
        .routes(routes!(im_bindings, set_im))
        .routes(routes!(im_profile))
}
