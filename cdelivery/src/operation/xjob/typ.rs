use garde::Validate;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::concept;

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct XJob {
    pub(crate) name: String,
    pub(crate) interval: super::super::typ::Duration,
    pub(crate) timeout: super::super::typ::Duration,
    pub(crate) command: Vec<String>,
    pub(crate) status: super::super::typ::RunState,
    /// Latest observed task creation time, not its completion time; absent when no task records remain.
    pub(crate) last_run: Option<Timestamp>,
    /// Unavailable during an active run; missing run records use the service update time as an approximation.
    pub(crate) next_run: Option<Timestamp>,
}

impl XJob {
    pub(crate) fn is_active(&self) -> bool {
        matches!(
            self.status,
            super::super::typ::RunState::Pending | super::super::typ::RunState::Running { .. }
        )
    }
}

impl TryFrom<concept::xjob::XJob> for XJob {
    type Error = crate::error::Error;

    fn try_from(xjob: concept::xjob::XJob) -> crate::error::Result<Self> {
        Ok(Self {
            name: xjob.name().to_string(),
            interval: xjob.spec().interval().try_into()?,
            timeout: xjob.spec().timeout().try_into()?,
            command: xjob.spec().command().clone(),
            status: xjob.state().clone().into(),
            last_run: xjob.last_run(),
            next_run: xjob.next_run(),
        })
    }
}

#[derive(Serialize, ToSchema)]
pub(crate) struct XJobs {
    pub(crate) jobs: Vec<XJob>,
}

#[macros_derive::validated_schema]
#[derive(Debug, Deserialize, ToSchema, Validate)]
pub(crate) struct CreateXJobRequest {
    /// Wait after a run ends; the first run waits from creation. Use integer minutes or hours, from 1 minute to 24 hours.
    #[garde(custom(super::super::validate::xjob_duration))]
    pub(crate) interval: super::super::typ::Duration,
    /// Maximum time from task creation, including preparation; enforced by polling. Use integer minutes or hours, from 1 minute to 24 hours.
    #[garde(custom(super::super::validate::xjob_duration))]
    pub(crate) timeout: super::super::typ::Duration,
    #[garde(
        length(min = 1, max = 32),
        inner(custom(super::super::validate::xjob_argument))
    )]
    pub(crate) command: Vec<String>,
}

#[macros_derive::validated_schema(patch)]
#[derive(Debug, Deserialize, ToSchema, Validate)]
pub(crate) struct UpdateXJobRequest {
    /// Recalculates the wait using the existing end time. Use integer minutes or hours, from 1 minute to 24 hours.
    #[garde(inner(custom(super::super::validate::xjob_duration)))]
    pub(crate) interval: Option<super::super::typ::Duration>,
    /// Applies to the current run without resetting its start; shortening can request a stop on the next poll. Use integer minutes or hours, from 1 minute to 24 hours.
    #[garde(inner(custom(super::super::validate::xjob_duration)))]
    pub(crate) timeout: Option<super::super::typ::Duration>,
    #[garde(inner(
        length(min = 1, max = 32),
        inner(custom(super::super::validate::xjob_argument))
    ))]
    pub(crate) command: Option<Vec<String>>,
}
