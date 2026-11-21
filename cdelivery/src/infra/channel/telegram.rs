pub(crate) mod poll;

use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::config;
use crate::error::{ErrorKind, Result};

const BASE: &str = "https://api.telegram.org";
pub(super) const PLATFORM: &str = "Telegram";
const MESSAGE_MAX: usize = 4_096;

pub(crate) struct Rest {
    client: reqwest::Client,
    bot_token: String,
}

impl Rest {
    pub(crate) fn new(bot_token: &str) -> Result<Self> {
        if bot_token.is_empty() || bot_token.chars().any(char::is_whitespace) {
            return Err(ErrorKind::im_api(PLATFORM, "bot token is invalid").into());
        }
        Ok(Self {
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(config::AGENT_CHANNEL_TIMEOUT.unsigned_abs())
                .build()?,
            bot_token: bot_token.to_owned(),
        })
    }

    pub(crate) async fn verify(&self) -> Result<()> {
        self.identity().await.map(|_| ())
    }

    pub(super) async fn send(&self, chat_id: &str, mut text: &str) -> Result<()> {
        if text.is_empty() {
            return Err(ErrorKind::im_api(PLATFORM, "message is empty").into());
        }
        let chat_id = chat_id
            .parse::<i64>()
            .map_err(|_| ErrorKind::im_api(PLATFORM, "chat ID is not a signed decimal integer"))?;
        if chat_id == 0 {
            return Err(ErrorKind::im_api(PLATFORM, "chat ID is zero").into());
        }
        while !text.is_empty() {
            let end = text
                .char_indices()
                .nth(MESSAGE_MAX)
                .map_or(text.len(), |(index, _)| index);
            let (chunk, remaining) = text.split_at(end);
            self.call::<serde_json::Value>(
                "sendMessage",
                &serde_json::json!({"chat_id": chat_id, "text": chunk}),
            )
            .await?;
            text = remaining;
        }
        Ok(())
    }

    async fn identity(&self) -> Result<Identity> {
        let user = self.call::<User>("getMe", &serde_json::json!({})).await?;
        let username = user.username.filter(|value| !value.is_empty());
        if !user.is_bot || user.id <= 0 {
            return Err(
                ErrorKind::im_api(PLATFORM, "getMe returned an invalid bot identity").into(),
            );
        }
        let Some(username) = username else {
            return Err(
                ErrorKind::im_api(PLATFORM, "getMe returned a bot without username").into(),
            );
        };
        Ok(Identity {
            id: user.id,
            username,
        })
    }

    async fn delete_webhook(&self) -> Result<()> {
        if self
            .call::<bool>(
                "deleteWebhook",
                &serde_json::json!({"drop_pending_updates": false}),
            )
            .await?
        {
            Ok(())
        } else {
            Err(ErrorKind::im_api(PLATFORM, "deleteWebhook returned false").into())
        }
    }

    async fn get_updates(&self, offset: u64) -> Result<Vec<poll::Update>> {
        self.call(
            "getUpdates",
            &serde_json::json!({
                "offset": offset,
                "limit": 100,
                "timeout": 20,
                "allowed_updates": ["message"],
            }),
        )
        .await
    }

    async fn call<T>(&self, method: &str, body: &serde_json::Value) -> Result<T>
    where
        T: DeserializeOwned,
    {
        loop {
            let response = self
                .client
                .post(format!("{BASE}/bot{}/{method}", self.bot_token))
                .json(body)
                .send()
                .await
                .map_err(reqwest::Error::without_url)?;
            let status = response.status();
            let bytes = response
                .bytes()
                .await
                .map_err(reqwest::Error::without_url)?;
            let envelope: Envelope<T> =
                serde_json::from_slice(&bytes).map_err(|source| ErrorKind::ImApi {
                    platform: PLATFORM,
                    detail: format!("{method} failed with http {status}: invalid response"),
                    source: Some(source),
                })?;
            if status.is_success() && envelope.ok {
                return envelope.result.ok_or_else(|| {
                    ErrorKind::im_api(
                        PLATFORM,
                        format!("{method} failed with http {status}: response has no result"),
                    )
                    .into()
                });
            }
            if let Some(after) = envelope
                .parameters
                .and_then(|parameters| parameters.retry_after)
            {
                tokio::time::sleep(std::time::Duration::from_secs(after)).await;
                continue;
            }
            let code = envelope
                .error_code
                .map_or_else(String::new, |code| format!(" api {code}"));
            let description = envelope
                .description
                .map_or_else(String::new, |description| {
                    format!(": {}", description.replace(&self.bot_token, "[REDACTED]"))
                });
            return Err(ErrorKind::im_api(
                PLATFORM,
                format!("{method} failed with http {status}{code}{description}"),
            )
            .into());
        }
    }
}

struct Identity {
    id: i64,
    username: String,
}

#[derive(Deserialize)]
struct Envelope<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
    error_code: Option<i64>,
    parameters: Option<ResponseParameters>,
}

#[derive(Deserialize)]
struct ResponseParameters {
    retry_after: Option<u64>,
}

#[derive(Deserialize)]
struct User {
    id: i64,
    is_bot: bool,
    username: Option<String>,
}
