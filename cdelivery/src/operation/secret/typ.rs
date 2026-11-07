use base64::Engine;
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::{concept, error};

macros::mirror_enum! {
    #[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
    Kind both concept::secret::Kind { Text, File }
}

#[macros_derive::validated_schema]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Validate)]
pub(crate) struct Reference {
    #[garde(custom(super::super::validate::valid_name))]
    pub(crate) name: String,
    #[garde(length(min = 1, max = 255))]
    pub(crate) id: String,
}

impl TryFrom<&Reference> for concept::secret::Reference {
    type Error = error::Error;

    fn try_from(reference: &Reference) -> error::Result<Self> {
        Ok(Self::new(reference.name.parse()?, reference.id.clone()))
    }
}

impl From<&concept::secret::Reference> for Reference {
    fn from(reference: &concept::secret::Reference) -> Self {
        Self {
            name: reference.name().to_string(),
            id: reference.id().clone(),
        }
    }
}

#[macros_derive::validated_schema]
#[derive(Deserialize, ToSchema, Validate)]
pub(crate) enum Content {
    /// Multiline UTF-8 text, preserved verbatim; NUL is forbidden.
    Text(#[garde(length(min = 1, max = 511_999))] String),
    /// Base64-encoded file bytes; decoded content must be at most 511999 bytes.
    File(#[garde(length(min = 1, max = 682_668))] String),
}

impl Content {
    pub(super) fn decode(self) -> error::Result<(concept::secret::Kind, Vec<u8>)> {
        match self {
            Self::Text(text) => Ok((concept::secret::Kind::Text, text.into_bytes())),
            Self::File(data) => Ok((
                concept::secret::Kind::File,
                base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .map_err(|_| error::ErrorKind::NotBase64)?,
            )),
        }
    }
}

#[macros_derive::validated_schema]
#[derive(Deserialize, ToSchema, Validate)]
pub(crate) struct CreateRequest {
    #[garde(custom(super::super::validate::valid_name))]
    pub(crate) name: String,
    #[garde(inner(length(chars, max = 64)))]
    #[serde(default)]
    pub(crate) description: Option<String>,
    #[garde(dive)]
    pub(crate) content: Content,
}

#[macros_derive::validated_schema]
#[derive(Deserialize, ToSchema, IntoParams, Validate)]
#[into_params(parameter_in = Query)]
pub(crate) struct Query {
    #[garde(skip)]
    #[serde(default)]
    pub(crate) kind: Option<Kind>,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct Secret {
    pub(crate) name: String,
    pub(crate) id: String,
    pub(crate) kind: Kind,
    pub(crate) description: Option<String>,
    pub(crate) created_at: jiff::Timestamp,
}

impl From<concept::secret::Secret> for Secret {
    fn from(secret: concept::secret::Secret) -> Self {
        Self {
            name: secret.reference().name().to_string(),
            id: secret.reference().id().clone(),
            kind: secret.kind().into(),
            description: secret.description().clone(),
            created_at: secret.created_at(),
        }
    }
}

#[derive(Serialize, ToSchema)]
pub(crate) struct Secrets {
    pub(crate) secrets: Vec<Secret>,
}
