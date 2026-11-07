use getset::{CopyGetters, Getters};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config;

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    strum::VariantArray,
)]
pub(crate) enum Im {
    Feishu,
    Lark,
    Slack,
    Telegram,
    Discord,
}

impl Im {
    pub(crate) const fn key(self) -> &'static str {
        match self {
            Self::Feishu => "feishu",
            Self::Lark => "lark",
            Self::Slack => "slack",
            Self::Telegram => "telegram",
            Self::Discord => "discord",
        }
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Feishu => "Feishu",
            Self::Lark => "Lark",
            Self::Slack => "Slack",
            Self::Telegram => "Telegram",
            Self::Discord => "Discord",
        }
    }
}

#[derive(Debug, Getters, CopyGetters, macros_derive::Ctor)]
pub(crate) struct Inbound {
    #[getset(get = "pub(crate)")]
    event_id: String,
    #[getset(get = "pub(crate)")]
    group_chat_id: Option<String>,
    #[getset(get = "pub(crate)")]
    sender_id: String,
    #[getset(get = "pub(crate)")]
    text: String,
    #[getset(get_copy = "pub(crate)")]
    timestamp: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Timeline {
    Dm { im: Im, id: String, user: Uuid },
    Group { im: Im, chat_id: String, user: Uuid },
    Email { user: Uuid },
}

impl Timeline {
    pub(crate) fn user(&self) -> Uuid {
        match self {
            Self::Dm { user, .. } | Self::Group { user, .. } | Self::Email { user } => *user,
        }
    }
}

impl std::fmt::Display for Timeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dm { im, id, user } => write!(f, "{}:{id}:{user}", im.key()),
            Self::Group { im, chat_id, user } => {
                write!(f, "{}_group:{chat_id}:{user}", im.key())
            }
            Self::Email { user } => write!(f, "email:{user}"),
        }
    }
}

#[derive(Debug, Clone, Getters, CopyGetters)]
pub(crate) struct Message {
    #[getset(get_copy = "pub(crate)")]
    id: Uuid,
    #[getset(get = "pub(crate)")]
    timeline: Timeline,
    #[getset(get_copy = "pub(crate)")]
    role: insight_wire::message::Role,
    #[getset(get = "pub(crate)")]
    author: String,
    #[getset(get = "pub(crate)")]
    text: String,
    #[getset(get_copy = "pub(crate)")]
    timestamp: Timestamp,
}

impl Message {
    pub(crate) fn with_id(
        id: Uuid,
        timeline: Timeline,
        role: insight_wire::message::Role,
        author: String,
        text: String,
        timestamp: Timestamp,
    ) -> Self {
        Self {
            id,
            timeline,
            role,
            author,
            text: clip(text, config::AGENT_TEXT_MAX),
            timestamp,
        }
    }
}

fn clip(text: String, escaped_budget: usize) -> String {
    let mut used = 0;
    for (at, c) in text.char_indices() {
        used += escaped_len(c);
        if used > escaped_budget {
            let mut text = text;
            text.truncate(at);
            return text;
        }
    }
    text
}

fn escaped_len(c: char) -> usize {
    match c {
        '"' | '\\' | '\x08' | '\x0c' | '\n' | '\r' | '\t' => 2,
        c if (c as u32) < 0x20 => 6,
        c => c.len_utf8(),
    }
}
