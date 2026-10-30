use std::sync::Arc;

use axum::{extract::State, response::Response, routing};
use http::StatusCode;

use crate::boot::App;
use crate::locale::tt;
use crate::operation;

#[derive(askama::Template)]
#[template(path = "settings/email.html")]
struct EmailPage {
    account: super::super::account::Account,
    email: Option<operation::settings::typ::EmailSettings>,
}

#[derive(askama::Template)]
#[template(path = "settings/im.html")]
struct ImPage {
    account: super::super::account::Account,
    platform: operation::typ::Im,
    settings: Option<operation::settings::typ::FeishuSettings>,
}

impl ImPage {
    fn slug(&self) -> &'static str {
        super::super::im_slug(self.platform)
    }

    fn title(&self) -> std::borrow::Cow<'static, str> {
        super::super::im_title(self.platform)
    }
}

#[derive(askama::Template)]
#[template(path = "settings/slack.html")]
struct SlackPage {
    account: super::super::account::Account,
    slack: Option<operation::settings::typ::SlackSettings>,
}

#[derive(askama::Template)]
#[template(path = "settings/bot.html")]
struct BotPage {
    account: super::super::account::Account,
    platform: operation::typ::Im,
    settings: Option<operation::settings::typ::BotSettings>,
    help: std::borrow::Cow<'static, str>,
}

impl BotPage {
    fn slug(&self) -> &'static str {
        super::super::im_slug(self.platform)
    }

    fn title(&self) -> std::borrow::Cow<'static, str> {
        super::super::im_title(self.platform)
    }
}

macro_rules! channel_routes {
    ($($channel:ident {
        get $get:tt
        set { $set:ident, $($set_rest:tt)* }
        clear { $clear:ident, $($clear_rest:tt)* }
        path $path:tt
    })+) => {
        pub(super) fn routes() -> axum::Router<Arc<App>> {
            axum::Router::new()$(
                .route(
                    $path,
                    routing::get($channel).post(super::set::<operation::settings::$set>),
                )
                .route(
                    concat!($path, "/clear"),
                    routing::post(super::clear::<operation::settings::$clear>),
                ))+
        }
    };
}

operation::settings::channels!(channel_routes);

async fn email(
    authorized: super::super::auth::Authorized<operation::settings::SetEmail>,
    State(app): State<Arc<App>>,
) -> super::super::response::Result<Response> {
    let settings =
        super::super::read::<operation::settings::GetEmail>(&app, authorized.access()).await?;
    super::super::response::render(
        &EmailPage {
            account: super::super::account::Account::new(authorized.access()),
            email: settings.email,
        },
        StatusCode::OK,
    )
}

async fn feishu(
    authorized: super::super::auth::Authorized<operation::settings::SetFeishu>,
    State(app): State<Arc<App>>,
) -> super::super::response::Result<Response> {
    let settings =
        super::super::read::<operation::settings::GetFeishu>(&app, authorized.access()).await?;
    super::super::response::render(
        &ImPage {
            account: super::super::account::Account::new(authorized.access()),
            platform: operation::typ::Im::Feishu,
            settings: settings.feishu,
        },
        StatusCode::OK,
    )
}

async fn lark(
    authorized: super::super::auth::Authorized<operation::settings::SetLark>,
    State(app): State<Arc<App>>,
) -> super::super::response::Result<Response> {
    let settings =
        super::super::read::<operation::settings::GetLark>(&app, authorized.access()).await?;
    super::super::response::render(
        &ImPage {
            account: super::super::account::Account::new(authorized.access()),
            platform: operation::typ::Im::Lark,
            settings: settings.lark,
        },
        StatusCode::OK,
    )
}

async fn slack(
    authorized: super::super::auth::Authorized<operation::settings::SetSlack>,
    State(app): State<Arc<App>>,
) -> super::super::response::Result<Response> {
    let settings =
        super::super::read::<operation::settings::GetSlack>(&app, authorized.access()).await?;
    super::super::response::render(
        &SlackPage {
            account: super::super::account::Account::new(authorized.access()),
            slack: settings.slack,
        },
        StatusCode::OK,
    )
}

async fn telegram(
    authorized: super::super::auth::Authorized<operation::settings::SetTelegram>,
    State(app): State<Arc<App>>,
) -> super::super::response::Result<Response> {
    let settings =
        super::super::read::<operation::settings::GetTelegram>(&app, authorized.access()).await?;
    super::super::response::render(
        &BotPage {
            account: super::super::account::Account::new(authorized.access()),
            platform: operation::typ::Im::Telegram,
            settings: settings.telegram,
            help: tt!("ui.settings.channels.telegram.help"),
        },
        StatusCode::OK,
    )
}

async fn discord(
    authorized: super::super::auth::Authorized<operation::settings::SetDiscord>,
    State(app): State<Arc<App>>,
) -> super::super::response::Result<Response> {
    let settings =
        super::super::read::<operation::settings::GetDiscord>(&app, authorized.access()).await?;
    super::super::response::render(
        &BotPage {
            account: super::super::account::Account::new(authorized.access()),
            platform: operation::typ::Im::Discord,
            settings: settings.discord,
            help: tt!("ui.settings.channels.discord.help"),
        },
        StatusCode::OK,
    )
}
