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
}

impl TryFrom<&XConfigAction> for usecase::app::XConfigParams {
    type Error = Error;

    fn try_from(action: &XConfigAction) -> Result<Self> {
        Ok(match action {
            XConfigAction::Upload(upload) => Self::Upload(upload.try_into()?),
            XConfigAction::InUse(existing) => Self::InUse(existing.into()),
        })
    }
}
