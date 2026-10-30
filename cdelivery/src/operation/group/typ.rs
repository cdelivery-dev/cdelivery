use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::concept;

#[macros_derive::validated_schema]
#[derive(Deserialize, ToSchema, Validate)]
pub(crate) struct CreateGroupRequest {
    #[garde(custom(super::super::validate::valid_name))]
    pub(crate) name: String,
    #[garde(length(max = 256))]
    #[serde(default)]
    pub(crate) description: Option<String>,
}

#[macros_derive::validated_schema(patch)]
#[derive(Deserialize, ToSchema, Validate)]
pub(crate) struct UpdateGroupRequest {
    #[garde(inner(length(max = 256)))]
    pub(crate) description: Option<Option<String>>,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct Group {
    pub(crate) name: String,
    pub(crate) description: Option<String>,
}

impl From<concept::group::Group> for Group {
    fn from(group: concept::group::Group) -> Self {
        Self {
            name: group.name().as_str().to_owned(),
            description: group.description().clone(),
        }
    }
}

#[derive(Serialize, ToSchema)]
pub(crate) struct Groups {
    pub(crate) groups: Vec<Group>,
}
