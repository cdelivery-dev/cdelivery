use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::concept;
use crate::error::{Error, ErrorKind, Result};

#[macros_derive::validated_schema]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Validate)]
pub(crate) struct ImageAuth {
    #[garde(length(min = 1, max = 255))]
    pub(crate) username: String,
    /// Required on create; on update, omit to keep the stored token if the registry and `username` are unchanged.
    #[garde(inner(length(min = 1, max = 4096)))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(write_only)]
    pub(crate) token: Option<String>,
}

impl TryFrom<&ImageAuth> for concept::image::ImageAuth {
    type Error = Error;

    fn try_from(auth: &ImageAuth) -> Result<Self> {
        let token = auth
            .token
            .clone()
            .ok_or_else(|| ErrorKind::SecretRequired("image_auth.token".into()))?;
        Ok(Self::new(auth.username.clone(), token))
    }
}

impl From<&concept::image::ImageAuth> for ImageAuth {
    fn from(auth: &concept::image::ImageAuth) -> Self {
        Self {
            username: auth.username().clone(),
            token: None,
        }
    }
}
