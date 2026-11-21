use serde::{Deserialize, Serialize};

pub const INDEX_ID: &str = "cdelivery/message";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    User,
    Agent,
}

impl Role {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "User",
            Self::Agent => "Agent",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    pub id: uuid::Uuid,
    pub timestamp: jiff::Timestamp,
    pub timeline: String,
    pub role: Role,
    pub author: String,
    pub text: String,
}
