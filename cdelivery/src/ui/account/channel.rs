use std::sync::Arc;

use axum::{Extension, extract::State, response::Response, routing};
use http::StatusCode;
use strum::VariantArray;

use crate::boot::App;
use crate::locale::tt;
use crate::{concept, operation};

#[derive(askama::Template)]
#[template(path = "account/email.html")]
struct EmailPage {
    account: super::Account,
    configured: bool,
    email: String,
    enabled: bool,
}

#[derive(askama::Template)]
#[template(path = "account/im.html")]
struct ImPage {
    account: super::Account,
    platform: operation::typ::Im,
    configured: bool,
    binding: Option<String>,
    name: Option<String>,
    avatar: Option<String>,
    enabled: bool,
}

impl ImPage {
    fn slug(&self) -> &'static str {
        super::super::im_slug(self.platform)
    }

    fn title(&self) -> std::borrow::Cow<'static, str> {
        super::super::im_title(self.platform)
    }

    const fn uses_open_id(&self) -> bool {
        matches!(
            self.platform,
            operation::typ::Im::Feishu | operation::typ::Im::Lark
        )
    }

    const fn uses_member_id(&self) -> bool {
        matches!(self.platform, operation::typ::Im::Slack)
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NotificationForm {
    enabled: bool,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BindingForm {
    id: Option<String>,
}

pub(super) fn routes() -> axum::Router<Arc<App>> {
    let mut router = axum::Router::new()
        .route("/account/email", routing::get(email))
        .route("/account/email/notifications", routing::post(email_alerts));
    for platform in operation::typ::Im::VARIANTS.iter().copied() {
        let page = im_path(platform);
        router = router
            .route(&page, routing::get(im).layer(Extension(platform)))
            .route(
                &format!("{page}/binding"),
                routing::post(binding).layer(Extension(platform)),
            )
            .route(
                &format!("{page}/notifications"),
                routing::post(im_notifications).layer(Extension(platform)),
            );
    }
    router
}

async fn email(
    authorized: super::super::auth::Authorized<operation::user::MeGet>,
    State(app): State<Arc<App>>,
) -> super::super::response::Result<Response> {
    let configured = email_configured(&app, authorized.access()).await?;
    let (email, enabled) = if configured {
        let user = super::super::read::<operation::user::MeGet>(&app, authorized.access()).await?;
        (user.email, user.preferences.alert_email)
    } else {
        (String::new(), false)
    };
    super::super::response::render(
        &EmailPage {
            account: super::Account::new(authorized.access()),
            configured,
            email,
            enabled,
        },
        StatusCode::OK,
    )
}

async fn im(
    authorized: super::super::auth::Authorized<operation::user::MeGet>,
    State(app): State<Arc<App>>,
    Extension(platform): Extension<operation::typ::Im>,
) -> super::super::response::Result<Response> {
    let configured = im_configured(&app, authorized.access(), platform).await?;
    let (binding, enabled) = if configured {
        let (user, mut bindings) = tokio::try_join!(
            super::super::read::<operation::user::MeGet>(&app, authorized.access()),
            super::super::read::<operation::user::MeGetIm>(&app, authorized.access()),
        )?;
        (
            bindings.bindings.remove(&platform),
            user.preferences.alerts.contains(&platform),
        )
    } else {
        (None, false)
    };
    let (name, avatar) = if binding.is_some() {
        operation::execute::<operation::user::MeGetImProfile>(
            &app,
            authorized.access(),
            operation::user::ImPlatformInput { platform },
            jiff::Timestamp::now(),
        )
        .await
        .map(|profile| (profile.name, profile.avatar))
        .inspect_err(|error| tracing::warn!(error = ?error, "{} account profile lookup failed", super::super::im_slug(platform)))
        .unwrap_or_default()
    } else {
        (None, None)
    };
    super::super::response::render(
        &ImPage {
            account: super::Account::new(authorized.access()),
            platform,
            configured,
            binding,
            name,
            avatar,
            enabled,
        },
        StatusCode::OK,
    )
}

async fn email_alerts(
    authorized: super::super::auth::Authorized<operation::user::MeUpdatePreferences>,
    State(app): State<Arc<App>>,
    super::super::command::Payload(form): super::super::command::Payload<NotificationForm>,
) -> super::super::response::Result<Response> {
    authorized
        .execute(
            &app,
            operation::user::typ::UpdatePreferencesRequest {
                alert_email: Some(form.enabled),
                ..Default::default()
            },
        )
        .await?;
    Ok(super::super::router::redirect("/account/email"))
}

async fn binding(
    authorized: super::super::auth::Authorized<operation::user::MeSetIm>,
    State(app): State<Arc<App>>,
    Extension(platform): Extension<operation::typ::Im>,
    super::super::command::Payload(form): super::super::command::Payload<BindingForm>,
) -> super::super::response::Result<Response> {
    let input = operation::user::typ::SetImRequest {
        platform,
        id: form.id,
    };
    authorized.execute(&app, input).await?;
    Ok(super::super::router::redirect(&im_path(platform)))
}

async fn im_notifications(
    authorized: super::super::auth::Authorized<operation::user::MeUpdatePreferences>,
    State(app): State<Arc<App>>,
    Extension(platform): Extension<operation::typ::Im>,
    super::super::command::Payload(form): super::super::command::Payload<NotificationForm>,
) -> super::super::response::Result<Response> {
    let mut alerts = super::super::read::<operation::user::MeGet>(&app, authorized.access())
        .await?
        .preferences
        .alerts;
    if form.enabled {
        alerts.insert(platform);
    } else {
        alerts.remove(&platform);
    }
    authorized
        .execute(
            &app,
            operation::user::typ::UpdatePreferencesRequest {
                alerts: Some(alerts),
                ..Default::default()
            },
        )
        .await?;
    Ok(super::super::router::redirect(&im_path(platform)))
}

async fn im_configured(
    app: &App,
    access: &concept::permission::Access,
    platform: operation::typ::Im,
) -> crate::error::Result<bool> {
    match platform {
        operation::typ::Im::Feishu => Ok(super::super::read::<operation::settings::GetFeishu>(
            app, access,
        )
        .await?
        .feishu
        .is_some()),
        operation::typ::Im::Lark => Ok(super::super::read::<operation::settings::GetLark>(
            app, access,
        )
        .await?
        .lark
        .is_some()),
        operation::typ::Im::Slack => Ok(super::super::read::<operation::settings::GetSlack>(
            app, access,
        )
        .await?
        .slack
        .is_some()),
        operation::typ::Im::Telegram => Ok(super::super::read::<operation::settings::GetTelegram>(
            app, access,
        )
        .await?
        .telegram
        .is_some()),
        operation::typ::Im::Discord => Ok(super::super::read::<operation::settings::GetDiscord>(
            app, access,
        )
        .await?
        .discord
        .is_some()),
    }
}

async fn email_configured(
    app: &App,
    access: &concept::permission::Access,
) -> crate::error::Result<bool> {
    Ok(
        super::super::read::<operation::settings::GetEmail>(app, access)
            .await?
            .email
            .is_some(),
    )
}

fn im_path(platform: operation::typ::Im) -> String {
    format!("/account/{}", super::super::im_slug(platform))
}
