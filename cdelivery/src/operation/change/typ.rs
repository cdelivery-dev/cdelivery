use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{concept, usecase};

macros::mirror_enum! {
    #[derive(Debug, Clone, Copy, Serialize, ToSchema)]
    Stage from concept::change::Stage { Prepare, Check, Clean, Apply, Finish }
}

macros::mirror_enum! {
    #[derive(Debug, Clone, Copy, Serialize, ToSchema)]
    State from concept::change::State { Open, Completed, Closed }
}

macros::mirror_enum! {
    #[derive(Debug, Serialize, ToSchema)]
    Action from concept::change::Action { Apply, Complete, Close }
}

macros::mirror_enum! {
    #[derive(Debug, Serialize, ToSchema)]
    CheckResult from concept::change::CheckResult {
        Passed,
        Failed { error: String },
    }
}

#[macros_derive::validated_schema]
#[derive(Deserialize, ToSchema, Validate)]
#[schema(as = ChangeCreateRequest)]
pub(crate) struct CreateRequest {
    #[garde(custom(super::super::validate::valid_name))]
    pub(crate) dependency: String,
    #[garde(length(min = 1, max = 262_144))]
    pub(crate) sql: String,
    /// Rehearsal size: Neon CU or a name from `dependency_planetscale_sizes`; forbidden for Local. Omission uses the source minimum CU for Neon or source size for `PlanetScale`, except Metal defaults to PS-5.
    #[garde(inner(length(min = 1, max = 128)))]
    #[serde(default)]
    pub(crate) size: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[schema(as = ChangeTarget)]
pub(crate) enum Target {
    App {
        name: String,
        changes: Box<super::super::app::typ::mutation::Changes>,
    },
    PostgreSQL {
        dependency: super::super::secret::typ::Reference,
        sql: String,
        size: Option<String>,
    },
}

#[derive(Serialize, ToSchema)]
pub(crate) struct App {
    pub(crate) name: String,
    pub(crate) check: bool,
    /// Null means no result was saved, not that the check did not execute.
    pub(crate) result: Option<CheckResult>,
}

#[derive(Serialize, ToSchema)]
#[schema(as = ReleaseChange)]
pub(crate) struct Change {
    pub(crate) id: uuid::Uuid,
    pub(crate) target: Target,
    pub(crate) apps: Vec<App>,
    pub(crate) stage: Stage,
    pub(crate) state: State,
    pub(crate) error: Option<String>,
    pub(crate) busy: bool,
    pub(crate) actions: Vec<Action>,
    pub(crate) initiator: super::super::app::typ::view::Operator,
    pub(crate) created_at: jiff::Timestamp,
}

impl TryFrom<concept::change::Change> for Change {
    type Error = crate::error::Error;

    fn try_from(change: concept::change::Change) -> crate::error::Result<Self> {
        let target = match change.target() {
            concept::change::Target::App { name, changes } => Target::App {
                name: name.to_string(),
                changes: Box::new(changes.as_ref().try_into()?),
            },
            concept::change::Target::PostgreSQL {
                dependency,
                sql,
                size,
            } => Target::PostgreSQL {
                dependency: dependency.into(),
                sql: sql.reveal().to_owned(),
                size: size.clone(),
            },
        };
        let busy = usecase::change::busy(change.id());
        Ok(Self {
            id: change.id(),
            target,
            apps: change
                .apps()
                .iter()
                .map(|app| App {
                    name: app.name().to_string(),
                    check: app.check(),
                    result: app.result().clone().map(Into::into),
                })
                .collect(),
            stage: change.stage().into(),
            state: change.state().into(),
            error: change.error().clone(),
            busy,
            actions: if busy {
                Vec::new()
            } else {
                change.actions().iter().copied().map(Into::into).collect()
            },
            initiator: change.initiator().into(),
            created_at: change.created_at(),
        })
    }
}

impl Change {
    pub(crate) fn active(&self) -> bool {
        matches!(self.state, State::Open)
    }

    pub(crate) fn has_checks(&self) -> bool {
        self.apps.iter().any(|app| app.check)
    }

    pub(crate) fn checks_failed(&self) -> bool {
        self.apps
            .iter()
            .any(|app| matches!(app.result, Some(CheckResult::Failed { .. })))
    }
}

#[derive(Serialize, ToSchema)]
#[schema(as = ReleaseChanges)]
pub(crate) struct Changes {
    pub(crate) changes: Vec<Change>,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct Submitted {
    pub(crate) change: uuid::Uuid,
}
