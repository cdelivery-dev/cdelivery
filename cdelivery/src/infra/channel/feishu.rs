pub(crate) mod ws;

use jiff::{SignedDuration, Timestamp};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tokio::sync::{Mutex, OnceCell};

use crate::concept;
use crate::config;
use crate::error::{ErrorKind, Result};

pub(crate) struct Rest {
    client: reqwest::Client,
    base: &'static str,
    platform: &'static str,
    app_id: String,
    app_secret: String,
    token: Mutex<Option<(String, Timestamp)>>,
    bot: OnceCell<String>,
}

impl Rest {
    pub(crate) fn feishu(cfg: &concept::setting::Feishu) -> Result<Self> {
        Self::new(cfg, "https://open.feishu.cn", concept::message::Im::Feishu)
    }

    pub(crate) fn lark(cfg: &concept::setting::Feishu) -> Result<Self> {
        Self::new(
            cfg,
            "https://open.larksuite.com",
            concept::message::Im::Lark,
        )
    }

    fn new(
        cfg: &concept::setting::Feishu,
        base: &'static str,
        im: concept::message::Im,
    ) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(config::AGENT_CHANNEL_TIMEOUT.unsigned_abs())
                .build()?,
            base,
            platform: im.name(),
            app_id: cfg.app_id().clone(),
            app_secret: cfg.app_secret().clone(),
            token: Mutex::new(None),
            bot: OnceCell::new(),
        })
    }

    pub(crate) async fn verify(&self) -> Result<()> {
        self.bot_open_id().await?;
        ws::discover(self).await.map(|_| ())
    }

    pub(crate) async fn observe_profile(
        &self,
        open_id: &str,
    ) -> Result<(Option<String>, Option<String>)> {
        let path = format!("/open-apis/contact/v3/users/{open_id}?user_id_type=open_id");
        let body: serde_json::Value = parse(self.platform, &self.get(&path).await?)?;
        Ok((
            super::field(&body, "/data/user/name"),
            super::field(&body, "/data/user/avatar/avatar_72"),
        ))
    }

    pub(super) async fn send(
        &self,
        receive_id_type: &str,
        receive_id: &str,
        text: &str,
    ) -> Result<()> {
        let token = self.tenant_token().await?;
        let body = self
            .client
            .post(format!(
                "{}/open-apis/im/v1/messages?receive_id_type={receive_id_type}",
                self.base
            ))
            .bearer_auth(token)
            .json(&serde_json::json!({
                "receive_id": receive_id,
                "msg_type": "text",
                "content": serde_json::json!({"text": text}).to_string(),
            }))
            .send()
            .await?
            .bytes()
            .await?;
        ensure_ok(self.platform, &body)
    }

    pub(crate) async fn bot_open_id(&self) -> Result<String> {
        self.bot
            .get_or_try_init(|| async {
                let body = self.get("/open-apis/bot/v3/info").await?;
                parse_bot(self.platform, &body)
            })
            .await
            .cloned()
    }

    async fn get(&self, path: &str) -> Result<bytes::Bytes> {
        let token = self.tenant_token().await?;
        Ok(self
            .client
            .get(format!("{}{path}", self.base))
            .bearer_auth(token)
            .send()
            .await?
            .bytes()
            .await?)
    }

    async fn tenant_token(&self) -> Result<String> {
        let mut cached = self.token.lock().await;
        let now = Timestamp::now();
        if let Some((token, fresh_until)) = cached.as_ref()
            && now < *fresh_until
        {
            return Ok(token.clone());
        }
        let body = self
            .client
            .post(format!(
                "{}/open-apis/auth/v3/tenant_access_token/internal",
                self.base
            ))
            .json(&serde_json::json!({
                "app_id": self.app_id,
                "app_secret": self.app_secret,
            }))
            .send()
            .await?
            .bytes()
            .await?;
        let (token, expire) = parse_token(self.platform, &body)?;
        let ttl =
            SignedDuration::from_secs(expire).saturating_sub(config::AGENT_FEISHU_TOKEN_MARGIN);
        *cached = Some((token.clone(), config::expiry(now, ttl)));
        Ok(token)
    }
}

fn ensure_ok(platform: &'static str, body: &[u8]) -> Result<()> {
    #[derive(Deserialize)]
    struct Head {
        code: i64,
        #[serde(default)]
        msg: String,
    }
    let Head { code, msg } = serde_json::from_slice(body)?;
    if code == 0 {
        Ok(())
    } else {
        Err(ErrorKind::im_api(platform, format!("{code} {msg}")).into())
    }
}

fn parse<T: DeserializeOwned>(platform: &'static str, body: &[u8]) -> Result<T> {
    ensure_ok(platform, body)?;
    Ok(serde_json::from_slice(body)?)
}

fn parse_token(platform: &'static str, body: &[u8]) -> Result<(String, i64)> {
    #[derive(Deserialize)]
    struct Body {
        tenant_access_token: String,
        expire: i64,
    }
    parse(platform, body).map(|body: Body| (body.tenant_access_token, body.expire))
}

fn parse_bot(platform: &'static str, body: &[u8]) -> Result<String> {
    #[derive(Deserialize)]
    struct Body {
        bot: Bot,
    }
    #[derive(Deserialize)]
    struct Bot {
        open_id: String,
    }
    parse(platform, body).map(|body: Body| body.bot.open_id)
}
