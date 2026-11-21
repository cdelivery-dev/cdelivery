mod apikey;
mod channel;
mod security;

use std::sync::Arc;

use axum::extract::State;
use axum::response::Response;
use axum_extra::extract::CookieJar;
use http::StatusCode;

use crate::boot::App;
use crate::concept;
use crate::locale::tt;
use crate::operation;

pub(super) struct Account {
    pub(super) user_name: String,
    pub(super) initials: String,
}

impl Account {
    pub(super) fn new(access: &concept::permission::Access) -> Self {
        let user_name = access.actor().to_string();
        let initials = super::dashboard::initials(&user_name);
        Self {
            user_name,
            initials,
        }
    }
}

#[derive(askama::Template)]
#[template(path = "account/profile.html")]
struct ProfilePage {
    account: Account,
    user: operation::user::typ::SelfUser,
}

#[derive(askama::Template)]
#[template(path = "account/preferences.html")]
struct PreferencesPage {
    account: Account,
    preferences: operation::user::typ::Preferences,
}

pub(super) fn routes() -> axum::Router<Arc<App>> {
    axum::Router::new()
        .route("/account", axum::routing::get(profile).post(update_profile))
        .route(
            "/account/preferences",
            axum::routing::get(preferences).post(update_preferences),
        )
        .route("/logout", axum::routing::post(logout))
        .merge(channel::routes())
        .merge(security::routes())
        .merge(apikey::routes())
}

async fn profile(
    authorized: super::auth::Authorized<operation::user::MeGet>,
    State(app): State<Arc<App>>,
) -> super::response::Result<Response> {
    let account = Account::new(authorized.access());
    let user = authorized.execute(&app, operation::input::Empty {}).await?;
    super::response::render(&ProfilePage { account, user }, StatusCode::OK)
}

async fn update_profile(
    authorized: super::auth::Authorized<operation::user::MeUpdate>,
    State(app): State<Arc<App>>,
    super::command::Payload(input): super::command::Payload<
        operation::user::typ::UpdateProfileRequest,
    >,
) -> super::response::Result<Response> {
    authorized.execute(&app, input).await?;
    Ok(super::router::redirect("/account"))
}

async fn preferences(
    authorized: super::auth::Authorized<operation::user::MeGet>,
    State(app): State<Arc<App>>,
) -> super::response::Result<Response> {
    let account = Account::new(authorized.access());
    let preferences = authorized
        .execute(&app, operation::input::Empty {})
        .await?
        .preferences;
    let page = PreferencesPage {
        account,
        preferences,
    };
    super::response::render(&page, StatusCode::OK)
}

async fn update_preferences(
    authorized: super::auth::Authorized<operation::user::MeUpdatePreferences>,
    State(app): State<Arc<App>>,
    super::command::Payload(input): super::command::Payload<
        operation::user::typ::UpdatePreferencesRequest,
    >,
) -> super::response::Result<Response> {
    authorized.execute(&app, input).await?;
    Ok(super::router::redirect("/account/preferences"))
}

async fn logout(
    _authorized: super::auth::Authorized<operation::user::MeGet>,
    jar: CookieJar,
    super::command::Payload(_): super::command::Payload<operation::input::Empty>,
) -> Response {
    super::auth::login_redirect(jar)
}
