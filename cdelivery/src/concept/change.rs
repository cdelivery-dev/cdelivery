use getset::{CopyGetters, Getters};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{ErrorKind, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Target {
    App {
        name: super::name::Name,
        changes: Box<super::app::Changes>,
    },
    PostgreSQL {
        dependency: super::secret::Reference,
        sql: cipher::Secret,
        size: Option<String>,
    },
}

impl Target {
    pub(crate) fn reference(&self) -> super::dependency::Reference {
        match self {
            Self::App { name, .. } => super::dependency::Reference::App(name.clone()),
            Self::PostgreSQL { dependency, .. } => {
                super::dependency::Reference::PostgreSQL(dependency.clone())
            }
        }
    }

    pub(crate) fn name(&self) -> &super::name::Name {
        match self {
            Self::App { name, .. } => name,
            Self::PostgreSQL { dependency, .. } => dependency.name(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Stage {
    Prepare,
    Check,
    Clean,
    Apply,
    Finish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum State {
    Open,
    Completed,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    Apply,
    Complete,
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum CheckResult {
    Passed,
    Failed { error: String },
}

#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters, macros_derive::Ctor,
)]
pub(crate) struct App {
    #[getset(get = "pub(crate)")]
    name: super::name::Name,
    #[getset(get_copy = "pub(crate)")]
    check: bool,
    #[getset(get = "pub(crate)")]
    result: Option<CheckResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters)]
pub(crate) struct Change {
    #[getset(get_copy = "pub(crate)")]
    id: Uuid,
    #[getset(get = "pub(crate)")]
    target: Target,
    #[getset(get = "pub(crate)")]
    apps: Vec<App>,
    #[getset(get_copy = "pub(crate)")]
    stage: Stage,
    #[getset(get_copy = "pub(crate)")]
    state: State,
    #[getset(get = "pub(crate)")]
    error: Option<String>,
    #[getset(get_copy = "pub(crate)")]
    initiator: super::app::Operator,
    #[getset(get_copy = "pub(crate)")]
    created_at: Timestamp,
}

impl super::versioned::Aggregate for Change {}

impl Change {
    pub(crate) fn new(
        target: Target,
        apps: Vec<App>,
        initiator: super::app::Operator,
        created_at: Timestamp,
    ) -> Self {
        let mut participants = std::collections::BTreeMap::new();
        for app in apps {
            participants
                .entry(app.name)
                .and_modify(|check| *check |= app.check)
                .or_insert(app.check);
        }
        Self {
            id: Uuid::new_v4(),
            target,
            apps: participants
                .into_iter()
                .map(|(name, check)| App::new(name, check, None))
                .collect(),
            stage: Stage::Prepare,
            state: State::Open,
            error: None,
            initiator,
            created_at,
        }
    }

    pub(crate) fn active(&self) -> bool {
        self.state == State::Open
    }

    pub(crate) fn has_checks(&self) -> bool {
        self.apps.iter().any(App::check)
    }

    pub(crate) fn finish_checks(
        &mut self,
        mut results: std::collections::BTreeMap<super::name::Name, CheckResult>,
    ) {
        for app in self.apps.iter_mut().filter(|app| app.check) {
            app.result = results.remove(&app.name);
        }
        if self
            .apps
            .iter()
            .filter(|app| app.check)
            .all(|app| matches!(app.result, Some(CheckResult::Passed)))
        {
            self.advance(Stage::Clean);
        }
    }

    pub(crate) fn occupies(&self, name: &super::name::Name) -> bool {
        self.apps.iter().any(|app| app.name() == name)
    }

    pub(crate) fn advance(&mut self, stage: Stage) {
        self.stage = stage;
        self.error = None;
    }

    pub(crate) fn fail(&mut self, error: String) {
        self.error = Some(error);
    }

    pub(crate) fn complete(&mut self) {
        self.state = State::Completed;
        self.error = None;
    }

    pub(crate) fn close(&mut self) {
        self.state = State::Closed;
    }

    pub(crate) fn validate_action(&self, action: Action) -> Result<()> {
        if self.actions().contains(&action) {
            Ok(())
        } else {
            Err(ErrorKind::ChangeActionInvalid.into())
        }
    }

    pub(crate) fn actions(&self) -> &'static [Action] {
        if !self.active() {
            return &[];
        }
        match self.stage {
            Stage::Prepare | Stage::Check | Stage::Clean => &[Action::Apply, Action::Close],
            Stage::Apply => &[Action::Apply, Action::Complete, Action::Close],
            Stage::Finish => &[Action::Complete],
        }
    }

    pub(crate) fn resource_name(&self, kind: &str, object: &str) -> String {
        format!(
            "cdelivery-{kind}-{}",
            Uuid::new_v5(&self.id, object.as_bytes()).simple()
        )
    }

    pub(crate) fn branch_name(&self) -> String {
        format!("cdelivery-{}", self.id.simple())
    }
}
