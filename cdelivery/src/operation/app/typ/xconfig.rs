use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::{Error, ErrorKind, Result};
use crate::usecase;
use crate::{concept, config};

const _: () = assert!(config::XCONFIG_DATA_MAX.div_ceil(3) * 4 == 1_365_332);

#[macros_derive::validated_schema]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Validate)]
pub(crate) struct XConfigUpload {
    /// File path inside the container.
    #[garde(
        length(min = 2, max = 255),
        custom(super::super::super::validate::valid_xconfig_target)
    )]
    pub(crate) target: String,
    #[garde(length(min = 1, max = 1_365_332))]
    pub(crate) data_base64: String,
}

impl TryFrom<&XConfigUpload> for usecase::app::UploadParams {
    type Error = Error;

    fn try_from(upload: &XConfigUpload) -> Result<Self> {
        Ok(Self::new(
            upload.target.clone(),
            STANDARD
                .decode(upload.data_base64.as_bytes())
                .map_err(|_| ErrorKind::NotBase64)?,
        ))
    }
}

#[macros_derive::validated_schema]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Validate)]
pub(crate) struct XConfigRef {
    #[garde(
        length(min = 2, max = 255),
        custom(super::super::super::validate::valid_xconfig_target)
    )]
    pub(crate) target: String,
    #[garde(length(min = 1, max = 255))]
    pub(crate) config_name: String,
    #[garde(length(min = 1, max = 255))]
    pub(crate) config_id: String,
}

super::super::super::typ::mirror_struct! {
    XConfigRef(x: &XConfigRef) <-> concept::app::XConfigRef(c: &concept::app::XConfigRef) {
        target: x.target.clone() => c.target().clone(),
        config_name: x.config_name.clone() => c.config_name().clone(),
        config_id: x.config_id.clone() => c.config_id().clone(),
    }
}

#[macros_derive::validated_schema]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Validate)]
pub(crate) enum XConfigAction {
    Upload(#[garde(dive)] XConfigUpload),
    /// Keeps a file the app already uses; copy it from `app_get`.
    InUse(#[garde(dive)] XConfigRef),
    Secret(#[garde(dive)] SecretMount),
}

impl TryFrom<&XConfigAction> for usecase::app::XConfigParams {
    type Error = Error;

    fn try_from(action: &XConfigAction) -> Result<Self> {
        Ok(match action {
            XConfigAction::Upload(upload) => Self::Upload(upload.try_into()?),
            XConfigAction::InUse(existing) => Self::InUse(existing.into()),
            XConfigAction::Secret(secret) => Self::Secret {
                target: secret.target.clone(),
                secret: (&secret.secret).try_into()?,
            },
        })
    }
}

#[macros_derive::validated_schema]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Validate)]
pub(crate) struct SecretMount {
    #[garde(
        length(min = 2, max = 255),
        custom(super::super::super::validate::valid_xconfig_target)
    )]
    pub(crate) target: String,
    #[garde(dive)]
    pub(crate) secret: super::super::super::secret::typ::Reference,
}

#[macros_derive::validated_schema]
#[derive(Debug, Serialize, Deserialize, ToSchema, Validate)]
#[serde(untagged)]
pub(crate) enum CreateXConfig {
    Upload(#[garde(dive)] XConfigUpload),
    Secret(#[garde(dive)] SecretMount),
}

impl TryFrom<&CreateXConfig> for usecase::app::XConfigParams {
    type Error = Error;

    fn try_from(value: &CreateXConfig) -> Result<Self> {
        Ok(match value {
            CreateXConfig::Upload(upload) => Self::Upload(upload.try_into()?),
            CreateXConfig::Secret(secret) => Self::Secret {
                target: secret.target.clone(),
                secret: (&secret.secret).try_into()?,
            },
        })
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(untagged)]
pub(crate) enum XConfig {
    Config(XConfigRef),
    Secret(SecretMount),
}

impl XConfig {
    pub(crate) fn target(&self) -> &str {
        match self {
            Self::Config(config) => &config.target,
            Self::Secret(secret) => &secret.target,
        }
    }
}

impl XConfigAction {
    pub(crate) fn target(&self) -> &str {
        match self {
            Self::Upload(config) => &config.target,
            Self::InUse(config) => &config.target,
            Self::Secret(secret) => &secret.target,
        }
    }
}

impl From<&concept::app::XConfig> for XConfig {
    fn from(value: &concept::app::XConfig) -> Self {
        match value {
            concept::app::XConfig::Config(config) => Self::Config(config.into()),
            concept::app::XConfig::Secret { target, secret } => Self::Secret(SecretMount {
                target: target.clone(),
                secret: secret.into(),
            }),
        }
    }
}

impl From<&concept::app::XConfig> for XConfigAction {
    fn from(value: &concept::app::XConfig) -> Self {
        match XConfig::from(value) {
            XConfig::Config(config) => Self::InUse(config),
            XConfig::Secret(secret) => Self::Secret(secret),
        }
    }
}
