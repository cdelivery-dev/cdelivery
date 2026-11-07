use getset::{CopyGetters, Getters};
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};

use crate::error::{ErrorKind, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Verdict {
    Passed { at: Timestamp },
    Skipped { at: Timestamp },
    Failed { error: String, at: Timestamp },
}

impl Verdict {
    pub(crate) fn resolved(&self) -> bool {
        matches!(self, Self::Passed { .. } | Self::Skipped { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Getters, CopyGetters, macros_derive::Ctor)]
#[ctor(hydrate)]
pub(crate) struct Guardian {
    #[getset(get = "pub(crate)")]
    name: super::name::Name,
    #[getset(get = "pub(crate)")]
    changes: super::app::Changes,
    #[getset(get_copy = "pub(crate)")]
    initiator: super::app::Operator,
    #[getset(get = "pub(crate)")]
    verdict: Option<Verdict>,
    #[getset(get_copy = "pub(crate)")]
    submitted_at: Timestamp,
}

impl Guardian {
    pub(crate) fn conclude(&mut self, verdict: Verdict) -> Result<()> {
        if self.verdict.is_some() {
            return Err(ErrorKind::GuardianResolved.into());
        }
        self.verdict = Some(verdict);
        Ok(())
    }

    pub(crate) fn skip(&mut self, now: Timestamp) -> Result<()> {
        if self.verdict.as_ref().is_some_and(Verdict::resolved) {
            return Err(ErrorKind::GuardianResolved.into());
        }
        self.verdict = Some(Verdict::Skipped { at: now });
        Ok(())
    }

    pub(crate) fn reopen(&mut self) -> Result<()> {
        if self.verdict.as_ref().is_some_and(Verdict::resolved) {
            return Err(ErrorKind::GuardianResolved.into());
        }
        self.verdict = None;
        Ok(())
    }

    pub(crate) fn cancel(&self, now: Timestamp) -> Result<()> {
        match &self.verdict {
            Some(Verdict::Passed { at } | Verdict::Skipped { at })
                if now.duration_since(*at) < crate::config::GUARDIAN_PROMOTE_SETTLE =>
            {
                Err(ErrorKind::GuardianResolved.into())
            }
            _ => Ok(()),
        }
    }
}

impl super::versioned::Aggregate for Guardian {}

#[derive(Debug, macros_derive::Ctor)]
pub(crate) struct Shadow {
    version: u64,
    updated_at: Timestamp,
    ready: bool,
    latest_task: Option<super::run::State>,
}

#[derive(Debug, macros_derive::Ctor)]
pub(crate) struct Check {
    version: u64,
    for_shadow: u64,
    status: super::run::State,
}

#[derive(Debug, macros_derive::Ctor)]
pub(crate) struct Snapshot {
    shadow: Shadow,
    check: Option<Check>,
}

#[derive(Debug)]
pub(crate) enum Action {
    Wait(Phase),
    RunCheck {
        shadow_version: u64,
        check_version: Option<u64>,
    },
    Stop {
        error: String,
        at: Timestamp,
        check_version: Option<u64>,
    },
    RecordVerdict(Verdict),
    Promote,
}

#[derive(Debug)]
pub(crate) enum Phase {
    Deploying,
    Validating,
    Failed { error: String, at: Timestamp },
    Promoting,
}

#[derive(Debug, Getters, CopyGetters)]
pub(crate) struct Status {
    #[getset(get = "pub(crate)")]
    guardian: Guardian,
    #[getset(get = "pub(crate)")]
    phase: Phase,
    #[getset(get = "pub(crate)")]
    check: Option<super::run::State>,
    #[getset(get_copy = "pub(crate)")]
    actionable: bool,
    #[getset(get_copy = "pub(crate)")]
    cancellable: bool,
}

impl Status {
    pub(crate) fn new(
        guardian: Guardian,
        snapshot: Snapshot,
        deadline: Option<SignedDuration>,
        now: Timestamp,
    ) -> Self {
        let action = next_action(guardian.verdict.as_ref(), &snapshot, deadline, now);
        let check = snapshot.check.map(|check| check.status);
        let actionable = !guardian.verdict.as_ref().is_some_and(Verdict::resolved);
        let cancellable = guardian.cancel(now).is_ok();
        Self {
            guardian,
            phase: action.phase(),
            check,
            actionable,
            cancellable,
        }
    }
}

impl Action {
    pub(crate) fn phase(self) -> Phase {
        match self {
            Self::Wait(phase) => phase,
            Self::RunCheck { .. } => Phase::Validating,
            Self::Stop { error, at, .. } | Self::RecordVerdict(Verdict::Failed { error, at }) => {
                Phase::Failed { error, at }
            }
            Self::RecordVerdict(_) | Self::Promote => Phase::Promoting,
        }
    }
}

pub(crate) fn next_action(
    verdict: Option<&Verdict>,
    snap: &Snapshot,
    deadline: Option<SignedDuration>,
    now: Timestamp,
) -> Action {
    match verdict {
        Some(Verdict::Passed { .. } | Verdict::Skipped { .. }) => return Action::Promote,
        Some(Verdict::Failed { error, at }) => {
            return Action::Stop {
                error: error.clone(),
                at: *at,
                check_version: snap.check.as_ref().map(|check| check.version),
            };
        }
        None => {}
    }
    let Some(deadline) = deadline else {
        return Action::RecordVerdict(Verdict::Failed {
            error: "health check configuration missing".to_owned(),
            at: snap.shadow.updated_at,
        });
    };
    if !snap.shadow.ready {
        return deploy_action(snap, now);
    }
    let Some(check) = &snap.check else {
        return Action::RunCheck {
            shadow_version: snap.shadow.version,
            check_version: None,
        };
    };
    if check.for_shadow < snap.shadow.version {
        return Action::RunCheck {
            shadow_version: snap.shadow.version,
            check_version: Some(check.version),
        };
    }
    match &check.status {
        super::run::State::Running { .. } if check.status.expired(deadline, now) => {
            Action::RecordVerdict(Verdict::Failed {
                error: "check timed out".to_owned(),
                at: now,
            })
        }
        super::run::State::Pending | super::run::State::Running { .. } => {
            Action::Wait(Phase::Validating)
        }
        super::run::State::Settled => Action::RecordVerdict(Verdict::Failed {
            error: "check produced no result".to_owned(),
            at: now,
        }),
        super::run::State::Passed { finished_at } => {
            Action::RecordVerdict(Verdict::Passed { at: *finished_at })
        }
        super::run::State::Failed { error, finished_at } => {
            Action::RecordVerdict(Verdict::Failed {
                error: error.clone(),
                at: *finished_at,
            })
        }
    }
}

fn deploy_action(snap: &Snapshot, now: Timestamp) -> Action {
    let Some(task) = &snap.shadow.latest_task else {
        if now.duration_since(snap.shadow.updated_at) <= crate::config::TASK_SETTLE {
            return Action::Wait(Phase::Deploying);
        }
        return Action::RecordVerdict(Verdict::Failed {
            error: "deploy produced no task".to_owned(),
            at: crate::config::expiry(snap.shadow.updated_at, crate::config::TASK_SETTLE),
        });
    };
    match task {
        super::run::State::Running { .. }
            if task.expired(crate::config::GUARDIAN_DEPLOY_TIMEOUT, now) =>
        {
            Action::RecordVerdict(Verdict::Failed {
                error: "deploy timed out".to_owned(),
                at: now,
            })
        }
        super::run::State::Pending
        | super::run::State::Running { .. }
        | super::run::State::Settled => Action::Wait(Phase::Deploying),
        super::run::State::Passed { finished_at } => Action::RecordVerdict(Verdict::Failed {
            error: "service task exited".to_owned(),
            at: *finished_at,
        }),
        super::run::State::Failed { error, finished_at } => {
            Action::RecordVerdict(Verdict::Failed {
                error: error.clone(),
                at: *finished_at,
            })
        }
    }
}
