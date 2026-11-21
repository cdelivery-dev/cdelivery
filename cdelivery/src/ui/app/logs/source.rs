use std::fmt;
use std::str::FromStr;

use jiff::Timestamp;
use serde_with::DeserializeFromStr;

use crate::error::{Error, ErrorKind, Result};
use crate::locale::tt;
use crate::{concept, operation};

#[derive(Debug, Clone, PartialEq, Eq, DeserializeFromStr)]
pub(super) enum Source {
    App,
    Access,
    XHealth,
    XJob(concept::name::Name),
    GuardianApp,
    GuardianCheck,
}

impl FromStr for Source {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "app" => Ok(Self::App),
            "access" => Ok(Self::Access),
            "xhealth" => Ok(Self::XHealth),
            "guardian_app" => Ok(Self::GuardianApp),
            "guardian_check" => Ok(Self::GuardianCheck),
            _ => value.strip_prefix("xjob:").map_or_else(
                || Err(ErrorKind::InvalidBody("invalid log source".to_owned()).into()),
                |name| name.parse().map(Self::XJob),
            ),
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::App => "app",
            Self::Access => "access",
            Self::XHealth => "xhealth",
            Self::XJob(name) => return write!(f, "xjob:{name}"),
            Self::GuardianApp => "guardian_app",
            Self::GuardianCheck => "guardian_check",
        })
    }
}

impl From<operation::typ::log::Source> for Source {
    fn from(source: operation::typ::log::Source) -> Self {
        match source {
            operation::typ::log::Source::App => Self::App,
            operation::typ::log::Source::XHealth => Self::XHealth,
            operation::typ::log::Source::XJob { job } => Self::XJob(job),
            operation::typ::log::Source::GuardianApp => Self::GuardianApp,
            operation::typ::log::Source::GuardianCheck => Self::GuardianCheck,
        }
    }
}

impl Source {
    pub(super) fn title(&self) -> String {
        match self {
            Self::App => tt!("ui.logs.source.app"),
            Self::Access => tt!("ui.logs.source.access"),
            Self::XHealth => tt!("ui.logs.source.xhealth"),
            Self::XJob(name) => tt!("ui.logs.source.xjob", name = name.as_str()),
            Self::GuardianApp => tt!("ui.logs.source.guardianApp"),
            Self::GuardianCheck => tt!("ui.logs.source.guardianCheck"),
        }
        .into_owned()
    }

    pub(super) fn target(&self) -> Result<(operation::typ::log::AppTarget, Option<String>)> {
        Ok(match self {
            Self::App => (operation::typ::log::AppTarget::App, None),
            Self::XHealth => (operation::typ::log::AppTarget::XHealth, None),
            Self::XJob(job) => (operation::typ::log::AppTarget::XJob, Some(job.to_string())),
            Self::GuardianApp => (operation::typ::log::AppTarget::GuardianApp, None),
            Self::GuardianCheck => (operation::typ::log::AppTarget::GuardianCheck, None),
            Self::Access => return Err(ErrorKind::InvalidOperationOutput.into()),
        })
    }
}

pub(super) async fn list(
    app: &crate::boot::App,
    access: &concept::permission::Access,
    spec: &operation::app::typ::view::AppSpec,
    now: Timestamp,
) -> (Vec<Source>, Option<String>) {
    let result = operation::execute::<operation::app::LogSources>(
        app,
        access,
        operation::input::Name {
            name: spec.name.clone(),
        },
        now,
    )
    .await;
    let (mut sources, error) = match result {
        Ok(output) => (
            output
                .sources
                .into_iter()
                .map(Source::from)
                .collect::<Vec<_>>(),
            None,
        ),
        Err(error) => {
            let sources = if matches!(
                spec.log_driver,
                operation::app::typ::container::LogDriver::Insight
            ) {
                vec![Source::App]
            } else {
                Vec::new()
            };
            (
                sources,
                Some(super::super::super::read_error(
                    &error,
                    "app log sources query failed",
                )),
            )
        }
    };
    if !spec.domains.is_empty() {
        sources.insert(
            usize::from(sources.first() == Some(&Source::App)),
            Source::Access,
        );
    }
    (sources, error)
}
