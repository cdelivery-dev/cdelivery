pub(crate) mod gateway;

use twilight_model::channel::message::AllowedMentions;
use twilight_model::id::Id;
use twilight_model::id::marker::{ChannelMarker, UserMarker};

use crate::config;
use crate::error::{ErrorKind, Result};

const PLATFORM: &str = "Discord";
const MESSAGE_UNITS: usize = 2_000;

pub(crate) struct Rest {
    client: twilight_http::Client,
    token: String,
}

impl Rest {
    pub(crate) fn new(token: &str) -> Result<Self> {
        if token.is_empty() || token.chars().any(char::is_whitespace) {
            return Err(ErrorKind::im_api(PLATFORM, "bot token is invalid").into());
        }
        Ok(Self {
            client: twilight_http::Client::builder()
                .token(token.to_owned())
                .timeout(config::AGENT_CHANNEL_TIMEOUT.unsigned_abs())
                .default_allowed_mentions(AllowedMentions::default())
                .build(),
            token: token.to_owned(),
        })
    }

    pub(crate) async fn verify(&self) -> Result<()> {
        self.bot_user_id().await.map(|_| ())
    }

    pub(super) async fn send_dm(&self, id: &str, text: &str) -> Result<()> {
        let recipient = parse_id::<UserMarker>(id, "user")?;
        let channel = self
            .client
            .create_private_channel(recipient)
            .await?
            .model()
            .await?;
        self.send(channel.id, text).await
    }

    pub(super) async fn send_group(&self, id: &str, text: &str) -> Result<()> {
        self.send(parse_id::<ChannelMarker>(id, "channel")?, text)
            .await
    }

    async fn bot_user_id(&self) -> Result<Id<UserMarker>> {
        let user = self.client.current_user().await?.model().await?;
        if !user.bot {
            return Err(ErrorKind::im_api(PLATFORM, "credential does not identify a bot").into());
        }
        Ok(user.id)
    }

    async fn send(&self, channel: Id<ChannelMarker>, mut text: &str) -> Result<()> {
        loop {
            let mut units = 0;
            let mut end = text.len();
            for (at, character) in text.char_indices() {
                units += character.len_utf16();
                if units > MESSAGE_UNITS {
                    end = at;
                    break;
                }
            }
            let (chunk, remaining) = text.split_at(end);
            self.client.create_message(channel).content(chunk).await?;
            if remaining.is_empty() {
                break;
            }
            text = remaining;
        }
        Ok(())
    }
}

fn parse_id<M>(value: &str, kind: &str) -> Result<Id<M>> {
    value
        .parse::<Id<M>>()
        .map_err(|_| ErrorKind::im_api(PLATFORM, format!("invalid {kind} id")).into())
}
