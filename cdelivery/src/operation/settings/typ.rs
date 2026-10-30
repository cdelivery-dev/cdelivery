use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::concept;
use crate::locale::Locale;

macros::mirror_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
    SettingsProvider both concept::setting::Provider {
        OpenAI, Anthropic, Gemini, DeepSeek, OpenRouter, Ollama, OpenAICompatible
    }
}

macros::mirror_enum! {
    #[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
    DutyPolicy both concept::setting::DutyPolicy { EarlyRisk, NearCritical, LimitOrFailure }
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct AgentSettings {
    pub(crate) provider: SettingsProvider,
    pub(crate) model: String,
    pub(crate) api_key_set: bool,
    pub(crate) endpoint: Option<String>,
    pub(crate) language: Locale,
    pub(crate) duty_policy: DutyPolicy,
    pub(crate) patrol_interval: super::super::typ::Duration,
    pub(crate) role: super::super::typ::Role,
    pub(crate) duty_enabled: bool,
}

impl AgentSettings {
    pub(super) fn assemble(
        agent: &concept::setting::Agent,
        duty_enabled: bool,
    ) -> crate::error::Result<Self> {
        Ok(Self {
            provider: agent.provider().into(),
            model: agent.model().clone(),
            api_key_set: agent.api_key().is_some(),
            endpoint: agent.endpoint().clone(),
            language: agent.language(),
            duty_policy: agent.duty_policy().into(),
            patrol_interval: agent.patrol_interval().try_into()?,
            role: agent.role().into(),
            duty_enabled,
        })
    }
}

#[derive(Serialize, ToSchema)]
pub(crate) struct SettingsAgent {
    pub(crate) agent: Option<AgentSettings>,
}

#[macros_derive::validated_schema(patch)]
#[derive(Deserialize, ToSchema, Validate)]
pub(crate) struct UpdateAgentSettingsRequest {
    #[garde(skip)]
    pub(crate) language: Option<Locale>,
    #[garde(skip)]
    pub(crate) duty_policy: Option<DutyPolicy>,
    /// `Minutes` or `Hours`, from 15 minutes to 24 hours.
    #[garde(inner(custom(super::super::validate::settings_patrol_interval)))]
    pub(crate) patrol_interval: Option<super::super::typ::Duration>,
    #[garde(skip)]
    pub(crate) role: Option<super::super::typ::Role>,
    /// Controls AI Duty only; ordinary alerts are independent. Defaults to false for new model configurations; omitted updates preserve the setting.
    #[garde(skip)]
    pub(crate) duty_enabled: Option<bool>,
    #[garde(skip)]
    pub(crate) provider: Option<SettingsProvider>,
    #[garde(inner(length(min = 1, max = 255)))]
    pub(crate) model: Option<String>,
    /// Omit for `Ollama`. Other providers require it unless `provider` and `endpoint` are unchanged, which keeps the stored key.
    #[garde(inner(length(min = 1, max = 8192)))]
    pub(crate) api_key: Option<String>,
    /// Required for `Ollama` and `OpenAICompatible`; clear it with null when switching to another provider.
    #[garde(inner(inner(custom(super::super::validate::settings_endpoint))))]
    pub(crate) endpoint: Option<Option<String>>,
}

#[macros_derive::validated_schema]
#[derive(Deserialize, ToSchema, Validate)]
pub(crate) struct DiscoverAgentModelsRequest {
    #[garde(skip)]
    pub(crate) provider: SettingsProvider,
    /// Omit for `Ollama`. Other providers require it unless `provider` and `endpoint` match the saved settings, which reuses the stored key.
    #[garde(inner(length(min = 1, max = 8192)))]
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    /// Required for `Ollama` and `OpenAICompatible`; omit for other providers.
    #[garde(inner(custom(super::super::validate::settings_endpoint)))]
    #[serde(default)]
    pub(crate) endpoint: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct AgentModels {
    pub(crate) models: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct SmtpSettings {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) password_set: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct ImapSettings {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) password_set: bool,
    pub(crate) authserv_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct EmailSettings {
    pub(crate) address: String,
    pub(crate) smtp: SmtpSettings,
    pub(crate) imap: Option<ImapSettings>,
}

impl From<&concept::setting::Email> for EmailSettings {
    fn from(email: &concept::setting::Email) -> Self {
        Self {
            address: email.address().clone(),
            smtp: SmtpSettings {
                host: email.smtp().host().clone(),
                port: email.smtp().port(),
                password_set: !email.smtp().password().is_empty(),
            },
            imap: email.imap().as_ref().map(|imap| ImapSettings {
                host: imap.host().clone(),
                port: imap.port(),
                password_set: !imap.password().is_empty(),
                authserv_id: imap.authserv_id().clone(),
            }),
        }
    }
}

#[macros_derive::validated_schema(patch)]
#[derive(Default, Deserialize, ToSchema, Validate)]
pub(crate) struct UpdateSmtpSettingsRequest {
    #[garde(inner(custom(super::super::validate::settings_host)))]
    pub(crate) host: Option<String>,
    #[garde(inner(range(min = 1)))]
    pub(crate) port: Option<u16>,
    #[garde(inner(length(min = 1, max = 4096)))]
    pub(crate) password: Option<String>,
}

#[macros_derive::validated_schema(patch)]
#[derive(Default, Deserialize, ToSchema, Validate)]
pub(crate) struct UpdateImapSettingsRequest {
    #[garde(inner(custom(super::super::validate::settings_host)))]
    pub(crate) host: Option<String>,
    #[garde(inner(range(min = 1)))]
    pub(crate) port: Option<u16>,
    #[garde(inner(length(min = 1, max = 4096)))]
    pub(crate) password: Option<String>,
    #[garde(inner(custom(super::super::validate::settings_host)))]
    pub(crate) authserv_id: Option<String>,
}

#[macros_derive::validated_schema(patch)]
#[derive(Deserialize, ToSchema, Validate)]
pub(crate) struct UpdateEmailSettingsRequest {
    #[garde(inner(email, length(max = 254)))]
    pub(crate) address: Option<String>,
    #[garde(dive)]
    pub(crate) smtp: Option<UpdateSmtpSettingsRequest>,
    #[garde(dive)]
    pub(crate) imap: Option<Option<UpdateImapSettingsRequest>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct FeishuSettings {
    pub(crate) app_id: String,
    pub(crate) app_secret_set: bool,
}

impl From<&concept::setting::Feishu> for FeishuSettings {
    fn from(feishu: &concept::setting::Feishu) -> Self {
        Self {
            app_id: feishu.app_id().clone(),
            app_secret_set: !feishu.app_secret().is_empty(),
        }
    }
}

#[macros_derive::validated_schema(patch)]
#[derive(Deserialize, ToSchema, Validate)]
pub(crate) struct UpdateFeishuSettingsRequest {
    #[garde(inner(length(min = 1, max = 255)))]
    pub(crate) app_id: Option<String>,
    #[garde(inner(length(min = 1, max = 4096)))]
    pub(crate) app_secret: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct SlackSettings {
    pub(crate) bot_token_set: bool,
    pub(crate) app_token_set: bool,
}

impl From<&concept::setting::Slack> for SlackSettings {
    fn from(slack: &concept::setting::Slack) -> Self {
        Self {
            bot_token_set: !slack.bot_token().is_empty(),
            app_token_set: !slack.app_token().is_empty(),
        }
    }
}

#[macros_derive::validated_schema(patch)]
#[derive(Deserialize, ToSchema, Validate)]
pub(crate) struct UpdateSlackSettingsRequest {
    #[garde(inner(length(min = 1, max = 4096)))]
    pub(crate) bot_token: Option<String>,
    #[garde(inner(length(min = 1, max = 4096)))]
    pub(crate) app_token: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct BotSettings {
    pub(crate) bot_token_set: bool,
}

impl From<&String> for BotSettings {
    fn from(token: &String) -> Self {
        Self {
            bot_token_set: !token.is_empty(),
        }
    }
}

#[macros_derive::validated_schema(patch)]
#[derive(Deserialize, ToSchema, Validate)]
pub(crate) struct UpdateBotSettingsRequest {
    #[garde(inner(length(min = 1, max = 4096)))]
    pub(crate) bot_token: Option<String>,
}

macro_rules! wrappers {
    ($($channel:ident {
        get { $get:ident, $wrapper:ident, $($get_rest:tt)* }
        set { $set:ident, $store:ident, $request:ident, $response:ident, $($set_rest:tt)* }
        clear $clear:tt
        path $path:tt
    })+) => {$(
        #[derive(Serialize, ToSchema)]
        pub(crate) struct $wrapper {
            pub(crate) $channel: Option<$response>,
        }
    )+};
}

super::channels!(wrappers);
