use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::concept;

macros::mirror_enum! {
    #[macros_derive::validated_schema]
    #[derive(Debug, Serialize, Deserialize, ToSchema, Validate)]
    StorageConfig both insight_wire::ObjectStoreConfig {
        Local,
        S3 {
            #[garde(length(min = 1))]
            region: String,
            #[garde(length(min = 1))]
            bucket_name: String,
            #[garde(length(min = 1))]
            access_key_id: String,
            /// Omit to keep the stored key if `access_key_id` is unchanged.
            #[garde(skip)]
            #[serde(default, skip_serializing_if = "super::super::typ::write_only")]
            #[schema(write_only, value_type = String)]
            secret_access_key: cipher::Secret,
        },
        S3Compatible {
            #[garde(length(min = 1))]
            endpoint: String,
            #[garde(length(min = 1))]
            region: String,
            #[garde(length(min = 1))]
            bucket_name: String,
            #[garde(length(min = 1))]
            access_key_id: String,
            /// Omit to keep the stored key if `access_key_id` is unchanged.
            #[garde(skip)]
            #[serde(default, skip_serializing_if = "super::super::typ::write_only")]
            #[schema(write_only, value_type = String)]
            secret_access_key: cipher::Secret,
        },
        Gcs {
            #[garde(length(min = 1))]
            bucket_name: String,
            /// Service account key JSON; omit to keep the stored one.
            #[garde(skip)]
            #[serde(default, skip_serializing_if = "super::super::typ::write_only")]
            #[schema(write_only, value_type = String)]
            credentials: cipher::Secret,
        },
        Azure {
            #[garde(length(min = 1))]
            container_name: String,
            #[garde(length(min = 1))]
            account_name: String,
            /// Omit to keep the stored key.
            #[garde(skip)]
            #[serde(default, skip_serializing_if = "super::super::typ::write_only")]
            #[schema(write_only, value_type = String)]
            account_key: cipher::Secret,
        },
    }
}

const _: () = assert!(
    insight_wire::metric::SAMPLE_INTERVAL_MIN_SECS == 1
        && insight_wire::metric::SAMPLE_INTERVAL_MAX_SECS == 60
);

#[macros_derive::validated_schema(patch)]
#[derive(Debug, Deserialize, ToSchema, Validate)]
pub(crate) struct EnableInsightRequest {
    /// After the first enable, only `region` and credentials can change.
    #[garde(dive)]
    pub(crate) storage: Option<StorageConfig>,
    #[garde(inner(range(min = 1, max = 3650)))]
    pub(crate) retention_days: Option<u16>,
    /// Fixed after the first enable when storage is `Local`.
    #[garde(dive)]
    pub(crate) placement: Option<super::super::node::typ::NodeSelector>,
    /// Must be the latest `insight` version from `version_list`.
    #[garde(inner(length(min = 1, max = 255)))]
    pub(crate) server_version: Option<String>,
    /// Must be the latest `insight_agent` version from `version_list`.
    #[garde(inner(length(min = 1, max = 255)))]
    pub(crate) agent_version: Option<String>,
    #[garde(inner(range(min = 1, max = 60)))]
    pub(crate) sample_interval_secs: Option<u16>,
    #[garde(dive)]
    pub(crate) flush_interval: Option<super::super::typ::Duration>,
}

macros::mirror_enum! {
    #[derive(Debug, Serialize, ToSchema)]
    InsightStatus from concept::insight::Status { Enabled, Disabled, Degraded }
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct InsightServer {
    pub(crate) version: String,
    pub(crate) image: String,
    pub(crate) storage: StorageConfig,
    pub(crate) retention_days: u16,
    pub(crate) flush_interval: super::super::typ::Duration,
    pub(crate) placement: super::super::node::typ::NodeSelector,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct InsightAgent {
    pub(crate) version: String,
    pub(crate) image: String,
    pub(crate) sample_interval_secs: u16,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct Insight {
    pub(crate) status: InsightStatus,
    pub(crate) server: Option<InsightServer>,
    pub(crate) agent: Option<InsightAgent>,
}

impl TryFrom<concept::insight::Deployment> for Insight {
    type Error = crate::error::Error;

    fn try_from(deployment: concept::insight::Deployment) -> crate::error::Result<Self> {
        Ok(Self {
            status: deployment.status().into(),
            server: deployment
                .server()
                .as_ref()
                .map(|insight| -> crate::error::Result<_> {
                    Ok(InsightServer {
                        version: insight.image().tag().clone(),
                        image: insight.image().to_string(),
                        storage: insight.storage().clone().into(),
                        retention_days: insight.retention_days(),
                        flush_interval: insight.flush_interval().try_into()?,
                        placement: insight.placement().clone().into(),
                    })
                })
                .transpose()?,
            agent: deployment
                .agent()
                .as_ref()
                .map(|(image, secs)| InsightAgent {
                    version: image.tag().clone(),
                    image: image.to_string(),
                    sample_interval_secs: *secs,
                }),
        })
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, ToSchema, strum::IntoStaticStr)]
pub(crate) enum Component {
    #[default]
    Server,
    Agent,
}
