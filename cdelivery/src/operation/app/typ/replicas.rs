use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::concept;
use crate::error::{Error, Result};

#[macros_derive::validated_schema]
#[derive(Debug, Serialize, Deserialize, ToSchema, Validate)]
pub(crate) enum Replicas {
    Fixed(#[garde(range(min = 1, max = 65536))] u64),
    Dynamic {
        /// Must not exceed max when max is set.
        #[garde(range(min = 1, max = 65536))]
        min: u64,
        /// Omit to use the platform ceiling; per-node scheduling limits still apply.
        #[garde(inner(range(min = 1, max = 65536)))]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<u64>,
    },
}

impl TryFrom<&Replicas> for concept::app::Replicas {
    type Error = Error;

    fn try_from(replicas: &Replicas) -> Result<Self> {
        match replicas {
            Replicas::Fixed(count) => Ok(Self::Fixed(*count)),
            Replicas::Dynamic { min, max } => {
                Ok(Self::Dynamic(concept::app::Dynamic::new(*min, *max)?))
            }
        }
    }
}

impl From<concept::app::Replicas> for Replicas {
    fn from(replicas: concept::app::Replicas) -> Self {
        match replicas {
            concept::app::Replicas::Fixed(count) => Self::Fixed(count),
            concept::app::Replicas::Dynamic(scaling) => Self::Dynamic {
                min: scaling.min(),
                max: scaling.max(),
            },
        }
    }
}
