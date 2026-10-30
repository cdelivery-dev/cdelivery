use getset::{CopyGetters, Getters};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub(crate) enum Subject {
    User {
        id: Uuid,
        name: String,
        api_key: Option<Uuid>,
    },
    Duty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Origin {
    Rest,
    Mcp,
    Agent,
}

#[derive(Debug, Clone, CopyGetters, Getters)]
pub(crate) struct Actor {
    #[getset(get = "pub(crate)")]
    subject: Subject,
    #[getset(get_copy = "pub(crate)")]
    origin: Origin,
}

impl Actor {
    pub(crate) fn user(id: Uuid, name: String, api_key: Option<Uuid>, origin: Origin) -> Self {
        Self {
            subject: Subject::User { id, name, api_key },
            origin,
        }
    }

    pub(crate) fn duty() -> Self {
        Self {
            subject: Subject::Duty,
            origin: Origin::Agent,
        }
    }

    pub(crate) fn user_id(&self) -> Option<Uuid> {
        match self.subject {
            Subject::User { id, .. } => Some(id),
            Subject::Duty => None,
        }
    }

    pub(crate) fn operator(&self) -> super::app::Operator {
        match &self.subject {
            Subject::Duty => super::app::Operator::Duty,
            Subject::User { id, api_key, .. } => match (self.origin, api_key.is_some()) {
                (Origin::Agent, _) => super::app::Operator::Agent { user_id: *id },
                (Origin::Rest | Origin::Mcp, true) => super::app::Operator::ApiKey { user_id: *id },
                (Origin::Rest | Origin::Mcp, false) => super::app::Operator::User { id: *id },
            },
        }
    }
}

impl std::fmt::Display for Actor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.subject, self.origin) {
            (Subject::Duty, _) => f.write_str(crate::config::DUTY_OPERATOR),
            (Subject::User { name, .. }, Origin::Rest) => f.write_str(name),
            (Subject::User { name, .. }, Origin::Mcp) => write!(f, "{name} via mcp"),
            (Subject::User { name, .. }, Origin::Agent) => write!(f, "{name} via agent"),
        }
    }
}
