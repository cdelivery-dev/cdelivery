use garde::Validate;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::concept;

const fn default_role() -> super::super::typ::Role {
    super::super::typ::Role::Reader
}

#[macros_derive::validated_schema]
#[derive(Deserialize, ToSchema, Validate)]
pub(crate) struct CreateApiKeyRequest {
    #[garde(length(min = 1, max = 64))]
    pub(crate) name: String,
    /// Omit for a key that never expires.
    #[garde(skip)]
    pub(crate) expires_at: Option<Timestamp>,
    /// Defaults to `Reader`.
    #[garde(skip)]
    #[serde(default = "default_role")]
    pub(crate) role: super::super::typ::Role,
}

pub(super) fn future_or_absent(expires_at: Option<Timestamp>, now: Timestamp) -> garde::Result {
    if expires_at.is_none_or(|exp| exp > now) {
        Ok(())
    } else {
        Err(garde::Error::new(
            crate::locale::tt!("validation.future_or_absent").into_owned(),
        ))
    }
}

#[derive(Serialize, ToSchema)]
pub(crate) struct IssuedApiKey {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) key: String,
    pub(crate) created_at: Timestamp,
    pub(crate) expires_at: Option<Timestamp>,
    pub(crate) role: super::super::typ::Role,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct ApiKey {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) created_at: Timestamp,
    pub(crate) expires_at: Option<Timestamp>,
    pub(crate) role: super::super::typ::Role,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct ApiKeys {
    pub(crate) keys: Vec<ApiKey>,
}

impl From<concept::user::ApiKey> for ApiKey {
    fn from(k: concept::user::ApiKey) -> Self {
        Self {
            id: k.id().to_string(),
            name: k.name().clone(),
            created_at: k.created_at(),
            expires_at: k.expires_at(),
            role: k.role().into(),
        }
    }
}
