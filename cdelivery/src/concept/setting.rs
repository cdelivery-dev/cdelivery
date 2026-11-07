use getset::{CopyGetters, Getters};
use jiff::SignedDuration;
use serde::{Deserialize, Serialize};

use crate::locale::Locale;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Provider {
    OpenAI,
    Anthropic,
    Gemini,
    DeepSeek,
    OpenRouter,
    Ollama,
    OpenAICompatible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum DutyPolicy {
    EarlyRisk,
    NearCritical,
    LimitOrFailure,
}

#[derive(
    Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters, macros_derive::Ctor,
)]
pub(crate) struct Agent {
    #[getset(get_copy = "pub(crate)")]
    provider: Provider,
    #[getset(get = "pub(crate)")]
    model: String,
    #[getset(get = "pub(crate)")]
    api_key: Option<String>,
    #[getset(get = "pub(crate)")]
    endpoint: Option<String>,
    #[getset(get_copy = "pub(crate)")]
    language: Locale,
    #[getset(get_copy = "pub(crate)")]
    duty_policy: DutyPolicy,
    #[getset(get_copy = "pub(crate)")]
    patrol_interval: SignedDuration,
    #[getset(get_copy = "pub(crate)")]
    role: super::user::Role,
}

#[derive(
    Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters, macros_derive::Ctor,
)]
pub(crate) struct Smtp {
    #[getset(get = "pub(crate)")]
    host: String,
    #[getset(get_copy = "pub(crate)")]
    port: u16,
    #[getset(get = "pub(crate)")]
    password: String,
}

#[derive(
    Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters, macros_derive::Ctor,
)]
pub(crate) struct Imap {
    #[getset(get = "pub(crate)")]
    host: String,
    #[getset(get_copy = "pub(crate)")]
    port: u16,
    #[getset(get = "pub(crate)")]
    password: String,
    #[getset(get = "pub(crate)")]
    authserv_id: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Getters, macros_derive::Ctor)]
#[getset(get = "pub(crate)")]
pub(crate) struct Email {
    address: String,
    smtp: Smtp,
    imap: Option<Imap>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Getters, macros_derive::Ctor)]
#[getset(get = "pub(crate)")]
pub(crate) struct Feishu {
    app_id: String,
    app_secret: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Getters, macros_derive::Ctor)]
#[getset(get = "pub(crate)")]
pub(crate) struct Slack {
    bot_token: String,
    app_token: String,
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize, CopyGetters)]
#[serde(default)]
pub(crate) struct Settings {
    agent: Option<Agent>,
    email: Option<Email>,
    feishu: Option<Feishu>,
    lark: Option<Feishu>,
    slack: Option<Slack>,
    telegram: Option<String>,
    discord: Option<String>,
    #[getset(get_copy = "pub(crate)")]
    duty_paused: bool,
}

impl Settings {
    pub(crate) fn agent(&self) -> Option<&Agent> {
        self.agent.as_ref()
    }

    pub(crate) fn duty_agent(&self) -> Option<&Agent> {
        self.agent.as_ref().filter(|_| !self.duty_paused)
    }

    pub(crate) fn email(&self) -> Option<&Email> {
        self.email.as_ref()
    }

    pub(crate) fn feishu(&self) -> Option<&Feishu> {
        self.feishu.as_ref()
    }

    pub(crate) fn lark(&self) -> Option<&Feishu> {
        self.lark.as_ref()
    }

    pub(crate) fn slack(&self) -> Option<&Slack> {
        self.slack.as_ref()
    }

    pub(crate) fn telegram(&self) -> Option<&String> {
        self.telegram.as_ref()
    }

    pub(crate) fn discord(&self) -> Option<&String> {
        self.discord.as_ref()
    }

    pub(crate) fn set_agent(&mut self, agent: Option<Agent>) {
        self.agent = agent;
    }

    pub(crate) fn set_email(&mut self, email: Option<Email>) {
        self.email = email;
    }

    pub(crate) fn set_feishu(&mut self, feishu: Option<Feishu>) {
        self.feishu = feishu;
    }

    pub(crate) fn set_lark(&mut self, lark: Option<Feishu>) {
        self.lark = lark;
    }

    pub(crate) fn set_slack(&mut self, slack: Option<Slack>) {
        self.slack = slack;
    }

    pub(crate) fn set_telegram(&mut self, telegram: Option<String>) {
        self.telegram = telegram;
    }

    pub(crate) fn set_discord(&mut self, discord: Option<String>) {
        self.discord = discord;
    }

    pub(crate) fn set_duty_paused(&mut self, paused: bool) {
        self.duty_paused = paused;
    }
}

impl super::versioned::Aggregate for Settings {}
