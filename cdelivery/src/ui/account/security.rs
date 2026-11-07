use std::sync::Arc;

use axum::{extract::State, response::Response, routing};
use axum_extra::extract::CookieJar;
use http::StatusCode;

use crate::boot::App;
use crate::locale::tt;
use crate::operation;

#[derive(askama::Template)]
#[template(path = "account/password.html")]
struct PasswordPage {
    account: super::Account,
}

#[derive(askama::Template)]
#[template(path = "account/two_factor.html")]
struct TwoFactorPage {
    account: super::Account,
    enrollment: Option<Enrollment>,
}

struct Enrollment {
    secret_base32: String,
    uri: String,
    qr: String,
}

pub(super) fn routes() -> axum::Router<Arc<App>> {
    axum::Router::new()
        .route("/account/password", routing::get(password).post(change))
        .route("/account/two-factor", routing::get(two_factor))
        .route("/account/two-factor/enable", routing::post(enable))
        .route("/account/two-factor/disable", routing::post(disable))
}

async fn password(
    authorized: super::super::auth::Authorized<operation::user::MeChangePassword>,
) -> super::super::response::Result<Response> {
    super::super::response::render(
        &PasswordPage {
            account: super::Account::new(authorized.access()),
        },
        StatusCode::OK,
    )
}

async fn change(
    authorized: super::super::auth::Authorized<operation::user::MeChangePassword>,
    State(app): State<Arc<App>>,
    jar: CookieJar,
    super::super::command::Payload(input): super::super::command::Payload<
        operation::user::typ::ChangePasswordRequest,
    >,
) -> super::super::response::Result<Response> {
    authorized.execute(&app, input).await?;
    Ok(super::super::auth::login_redirect(jar))
}

async fn two_factor(
    authorized: super::super::auth::Authorized<operation::user::MeGet>,
    State(app): State<Arc<App>>,
) -> super::super::response::Result<Response> {
    let account = super::Account::new(authorized.access());
    let user = authorized.execute(&app, operation::input::Empty {}).await?;
    let enrollment = if user.totp_enabled {
        None
    } else {
        let enrollment =
            super::super::read::<operation::user::MePrepareTotp>(&app, authorized.access()).await?;
        Some(Enrollment {
            qr: totp_rs::Totp::from_url(&enrollment.uri)
                .and_then(|totp| totp.to_qr_base64())
                .map_err(crate::error::ErrorKind::Totp)?,
            secret_base32: enrollment.secret_base32,
            uri: enrollment.uri,
        })
    };
    super::super::response::render(
        &TwoFactorPage {
            account,
            enrollment,
        },
        StatusCode::OK,
    )
}

async fn enable(
    authorized: super::super::auth::Authorized<operation::user::MeEnableTotp>,
    State(app): State<Arc<App>>,
    super::super::command::Payload(input): super::super::command::Payload<
        operation::user::typ::EnableTotpRequest,
    >,
) -> super::super::response::Result<Response> {
    authorized.execute(&app, input).await?;
    Ok(super::super::router::redirect("/account/two-factor"))
}

async fn disable(
    authorized: super::super::auth::Authorized<operation::user::MeDisableTotp>,
    State(app): State<Arc<App>>,
    super::super::command::Payload(input): super::super::command::Payload<
        operation::user::typ::DisableTotpRequest,
    >,
) -> super::super::response::Result<Response> {
    authorized.execute(&app, input).await?;
    Ok(super::super::router::redirect("/account/two-factor"))
}
