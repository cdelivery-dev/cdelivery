use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::concept;

#[macros_derive::validated_schema]
#[derive(Debug, Serialize, Deserialize, ToSchema, Validate)]
pub(crate) struct Domain {
    #[garde(custom(super::super::validate::valid_domain))]
    pub(crate) domain: String,
    #[garde(skip)]
    #[serde(default)]
    pub(crate) https: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct Instance {
    pub(crate) image: String,
    pub(crate) version: String,
    pub(crate) domain: Option<Domain>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct Versions(pub(crate) std::collections::BTreeMap<String, Release>);

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct Release {
    pub(crate) image: String,
    pub(crate) version: String,
    changelog: String,
    important: bool,
}

super::super::typ::mirror_struct! {
    Release from release: &concept::release::Release {
        image: release.image().to_string(),
        version: release.version().to_string(),
        changelog: release.changelog().clone(),
        important: release.important(),
    }
}

#[macros_derive::validated_schema(patch)]
#[derive(Debug, Deserialize, ToSchema, Validate)]
pub(crate) struct UpdateInstanceRequest {
    /// Must be the latest `cdelivery` version from `version_list`.
    #[garde(inner(length(min = 1, max = 255)))]
    pub(crate) version: Option<String>,
    #[garde(dive)]
    pub(crate) domain: Option<Option<Domain>>,
}
