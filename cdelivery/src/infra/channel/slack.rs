pub(crate) mod ws;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use tokio::sync::OnceCell;

use crate::concept;
use crate::config;
use crate::error::{ErrorKind, Result};

const BASE: &str = "https://slack.com/api";
const PLATFORM: &str = concept::message::Im::Slack.name();

pub(crate) struct Rest {
    client: reqwest::Client,
    bot_token: String,
    app_token: String,
    bot: OnceCell<String>,
}

impl Rest {
    pub(crate) fn new(cfg: &concept::setting::Slack) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(config::AGENT_CHANNEL_TIMEOUT.unsigned_abs())
                .build()?,
            bot_token: cfg.bot_token().clone(),
            app_token: cfg.app_token().clone(),
            bot: OnceCell::new(),
        })
    }

    pub(crate) async fn verify(&self) -> Result<()> {
        self.bot_user_id().await?;
        self.connections_open().await.map(|_| ())
    }

    pub(crate) async fn send(&self, channel: &str, text: &str) -> Result<()> {
        let body = self
            .call(
                "chat.postMessage",
                &self.bot_token,
                Some(serde_json::json!({"channel": channel, "text": escape(text)})),
            )
            .await?;
        ensure_ok(&body)
    }

    pub(crate) async fn observe_profile(
        &self,
        id: &str,
    ) -> Result<(Option<String>, Option<String>)> {
        let body = self
            .call("users.info", &self.bot_token, Some(json!({"user": id})))
            .await?;
        let body: serde_json::Value = parse(&body)?;
        Ok((
            super::field(&body, "/user/profile/display_name")
                .or_else(|| super::field(&body, "/user/profile/real_name")),
            super::field(&body, "/user/profile/image_72"),
        ))
    }

    pub(crate) async fn bot_user_id(&self) -> Result<String> {
        self.bot
            .get_or_try_init(|| async {
                let body = self.call("auth.test", &self.bot_token, None).await?;
                parse_user(&body)
            })
            .await
            .cloned()
    }

    pub(crate) async fn connections_open(&self) -> Result<String> {
        let body = self
            .call("apps.connections.open", &self.app_token, None)
            .await?;
        parse_url(&body)
    }

    async fn call(
        &self,
        method: &str,
        token: &str,
        body: Option<serde_json::Value>,
    ) -> Result<bytes::Bytes> {
        let mut request = self
            .client
            .post(format!("{BASE}/{method}"))
            .bearer_auth(token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await?;
        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("?")
                .to_owned();
            return Err(ErrorKind::im_api(
                PLATFORM,
                format!("{method} rate limited, retry after {after}s"),
            )
            .into());
        }
        if !status.is_success() {
            return Err(
                ErrorKind::im_api(PLATFORM, format!("{method} failed with http {status}")).into(),
            );
        }
        Ok(response.bytes().await?)
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn ensure_ok(body: &[u8]) -> Result<()> {
    #[derive(Deserialize)]
    struct Head {
        ok: bool,
        #[serde(default)]
        error: String,
    }
    let Head { ok, error } = serde_json::from_slice(body)?;
    if ok {
        Ok(())
    } else {
        Err(ErrorKind::im_api(PLATFORM, error).into())
    }
}

fn parse<T: DeserializeOwned>(body: &[u8]) -> Result<T> {
    ensure_ok(body)?;
    Ok(serde_json::from_slice(body)?)
}

fn parse_user(body: &[u8]) -> Result<String> {
    #[derive(Deserialize)]
    struct Body {
        user_id: String,
    }
    parse(body).map(|body: Body| body.user_id)
}

fn parse_url(body: &[u8]) -> Result<String> {
    #[derive(Deserialize)]
    struct Body {
        url: String,
    }
    parse(body).map(|body: Body| body.url)
}
