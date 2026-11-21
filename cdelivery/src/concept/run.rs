use jiff::{SignedDuration, Timestamp};

#[derive(Debug, Clone)]
pub(crate) enum State {
    Pending,
    Running {
        started_at: Timestamp,
    },
    Passed {
        finished_at: Timestamp,
    },
    Failed {
        error: String,
        finished_at: Timestamp,
    },
    Settled,
}

impl State {
    pub(crate) fn finished(&self) -> bool {
        matches!(
            self,
            Self::Passed { .. } | Self::Failed { .. } | Self::Settled
        )
    }

    pub(crate) fn expired(&self, max_run: SignedDuration, now: Timestamp) -> bool {
        matches!(self, Self::Running { started_at } if now.duration_since(*started_at) > max_run)
    }
}
