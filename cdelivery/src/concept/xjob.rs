use getset::{CopyGetters, Getters};
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters, macros_derive::Ctor,
)]
#[serde(deny_unknown_fields)]
pub(crate) struct Spec {
    #[getset(get_copy = "pub(crate)")]
    interval: SignedDuration,
    #[getset(get_copy = "pub(crate)")]
    timeout: SignedDuration,
    #[getset(get = "pub(crate)")]
    command: Vec<String>,
}

impl Spec {
    pub(crate) fn valid(&self) -> bool {
        [self.interval, self.timeout].into_iter().all(|duration| {
            (SignedDuration::from_mins(1)..=crate::config::XJOB_MAX_DURATION).contains(&duration)
                && duration.as_nanos() % 60_000_000_000 == 0
        }) && (1..=32).contains(&self.command.len())
            && self.command.iter().all(|arg| Self::valid_argument(arg))
    }

    pub(crate) fn valid_argument(arg: &str) -> bool {
        (1..=256).contains(&arg.len()) && !arg.contains('\0')
    }

    pub(crate) fn update(
        &mut self,
        interval: Option<SignedDuration>,
        timeout: Option<SignedDuration>,
        command: Option<Vec<String>>,
    ) {
        if let Some(interval) = interval {
            self.interval = interval;
        }
        if let Some(timeout) = timeout {
            self.timeout = timeout;
        }
        if let Some(command) = command {
            self.command = command;
        }
    }
}

impl super::versioned::Aggregate for Spec {}

#[derive(Debug, Getters, CopyGetters, macros_derive::Ctor)]
pub(crate) struct XJob {
    #[getset(get = "pub(crate)")]
    name: super::name::Name,
    #[getset(get = "pub(crate)")]
    spec: Spec,
    #[getset(get = "pub(crate)")]
    state: super::run::State,
    #[getset(get_copy = "pub(crate)")]
    last_run: Option<Timestamp>,
    fallback_at: Timestamp,
}

impl XJob {
    pub(crate) fn next_run(&self) -> Option<Timestamp> {
        let anchor = match self.state {
            super::run::State::Passed { finished_at }
            | super::run::State::Failed { finished_at, .. } => finished_at,
            super::run::State::Settled => self.fallback_at,
            super::run::State::Pending | super::run::State::Running { .. } => return None,
        };
        Some(crate::config::expiry(anchor, self.spec.interval()))
    }

    pub(crate) fn is_active(&self) -> bool {
        !self.state.finished()
    }

    pub(crate) fn due(&self, now: Timestamp) -> bool {
        self.next_run().is_some_and(|time| time <= now)
    }
}
