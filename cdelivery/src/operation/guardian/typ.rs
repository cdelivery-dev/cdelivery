use jiff::Timestamp;
use serde::Serialize;
use utoipa::ToSchema;

use crate::concept;
use crate::error::{Error, Result};

#[derive(Debug, Serialize, ToSchema)]
pub(crate) enum GuardianPhase {
    Deploying,
    Validating,
    Failed,
    Promoting,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct Guardian {
    pub(crate) phase: GuardianPhase,
    pub(crate) error: Option<String>,
    at: Option<Timestamp>,
    pub(crate) actionable: bool,
    pub(crate) cancellable: bool,
    initiator: super::super::app::typ::view::Operator,
    pub(crate) submitted_at: Timestamp,
    changes: super::super::app::typ::mutation::Changes,
    check: Option<super::super::typ::RunState>,
}

impl TryFrom<concept::guardian::Status> for Guardian {
    type Error = Error;

    fn try_from(status: concept::guardian::Status) -> Result<Self> {
        let (phase, error, at) = match status.phase() {
            concept::guardian::Phase::Deploying => (GuardianPhase::Deploying, None, None),
            concept::guardian::Phase::Validating => (GuardianPhase::Validating, None, None),
            concept::guardian::Phase::Failed { error, at } => {
                (GuardianPhase::Failed, Some(error.clone()), Some(*at))
            }
            concept::guardian::Phase::Promoting => (GuardianPhase::Promoting, None, None),
        };
        let guardian = status.guardian();
        Ok(Self {
            phase,
            error,
            at,
            actionable: status.actionable(),
            cancellable: status.cancellable(),
            initiator: guardian.initiator().into(),
            submitted_at: guardian.submitted_at(),
            changes: guardian.changes().try_into()?,
            check: status.check().clone().map(Into::into),
        })
    }
}
