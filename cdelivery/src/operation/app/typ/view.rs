use jiff::Timestamp;
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::concept;
use crate::error::{Error, Result};

macros::mirror_enum! {
    #[derive(Debug, Clone, Copy, Serialize, ToSchema)]
    Operator from concept::app::Operator {
        User { id: Uuid },
        ApiKey { user_id: Uuid },
        Agent { user_id: Uuid },
        Duty,
    }
}

impl Operator {
    pub(crate) fn user_id(self) -> Option<Uuid> {
        match self {
            Self::User { id } => Some(id),
            Self::ApiKey { user_id } | Self::Agent { user_id } => Some(user_id),
            Self::Duty => None,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct Change {
    pub(crate) changes: super::mutation::Changes,
    pub(crate) operator: Operator,
    pub(crate) operator_name: Option<String>,
    pub(crate) changed_at: Option<Timestamp>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) enum Rollback {
    Available(Change),
    InProgress(Option<Change>),
    Paused(Option<Change>),
    Completed(Option<Change>),
}

impl Rollback {
    pub(crate) fn change(&self) -> Option<&Change> {
        match self {
            Self::Available(change) => Some(change),
            Self::InProgress(change) | Self::Paused(change) | Self::Completed(change) => {
                change.as_ref()
            }
        }
    }

    pub(crate) fn change_mut(&mut self) -> Option<&mut Change> {
        match self {
            Self::Available(change) => Some(change),
            Self::InProgress(change) | Self::Paused(change) | Self::Completed(change) => {
                change.as_mut()
            }
        }
    }
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct AppSpec {
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    pub(crate) group: String,
    pub(crate) image: String,
    pub(crate) image_auth: Option<super::image::ImageAuth>,
    pub(crate) replicas: super::replicas::Replicas,
    /// The current target instance count; dynamic scaling changes it without changing the replica policy.
    #[schema(read_only)]
    pub(crate) desired_replicas: u64,
    pub(crate) ports: Vec<super::network::Port>,
    pub(crate) domains: Vec<super::mutation::DomainSpec>,
    pub(crate) xconfigs: Vec<super::xconfig::XConfigRef>,
    pub(crate) mounts: Option<super::container::NodeMounts>,
    pub(crate) container_options: Option<super::container::ContainerOptions>,
    pub(crate) resources: Option<super::deployment::Resources>,
    pub(crate) placement: Option<super::deployment::Placement>,
    pub(crate) log_driver: super::container::LogDriver,
    pub(crate) restart_policy: super::deployment::RestartPolicy,
    pub(crate) update_config: super::deployment::UpdateConfig,
    created_at: Timestamp,
    pub(crate) rollback: Option<Rollback>,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct AppSummary {
    pub(crate) name: String,
    pub(crate) group: String,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct Apps {
    pub(crate) apps: Vec<AppSummary>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct AppStatus {
    /// Time when the sequential runtime reads completed; this is not an atomic snapshot.
    pub(crate) observed_at: Timestamp,
    /// Ready means desired > 0 and running >= desired; it does not indicate business health or `XHealth` results.
    pub(crate) instances: super::super::super::overview::typ::ServiceInstances,
    /// Current task and update diagnostics; recent recorded failures may remain in `event_conditions`.
    pub(crate) diagnostics: Vec<String>,
}

impl TryFrom<concept::app::App> for AppSpec {
    type Error = Error;

    fn try_from(app: concept::app::App) -> Result<Self> {
        let change = |change: &concept::app::Change| {
            Ok::<_, Error>(Change {
                changes: change.changes().try_into()?,
                operator: (*change.operator()).into(),
                operator_name: None,
                changed_at: *change.changed_at(),
            })
        };
        Ok(Self {
            name: app.name().to_string(),
            description: app.description().clone(),
            group: app.group().to_string(),
            image: app.image().to_string(),
            image_auth: app.image_auth().as_ref().map(Into::into),
            replicas: app.replica_policy().into(),
            desired_replicas: app.replicas(),
            ports: app.ports().iter().copied().map(Into::into).collect(),
            domains: app.domains().iter().map(Into::into).collect(),
            xconfigs: app.xconfigs().iter().map(Into::into).collect(),
            mounts: app.mounts().as_ref().map(Into::into),
            container_options: app.container_options().as_ref().map(Into::into),
            resources: app
                .resources()
                .as_ref()
                .map(super::deployment::Resources::try_from)
                .transpose()?,
            placement: app.placement().as_ref().map(Into::into),
            log_driver: app.log_driver().try_into()?,
            restart_policy: app.restart_policy().try_into()?,
            update_config: app.update_config().try_into()?,
            created_at: app.created_at(),
            rollback: app
                .rollback()
                .as_ref()
                .map(|rollback| {
                    Ok::<_, Error>(match rollback {
                        concept::app::Rollback::Available(value) => {
                            Rollback::Available(change(value)?)
                        }
                        concept::app::Rollback::InProgress(value) => {
                            Rollback::InProgress(value.as_ref().map(change).transpose()?)
                        }
                        concept::app::Rollback::Paused(value) => {
                            Rollback::Paused(value.as_ref().map(change).transpose()?)
                        }
                        concept::app::Rollback::Completed(value) => {
                            Rollback::Completed(value.as_ref().map(change).transpose()?)
                        }
                    })
                })
                .transpose()?,
        })
    }
}
