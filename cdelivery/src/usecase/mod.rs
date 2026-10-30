pub(crate) mod access;
pub(crate) mod acme;
pub(crate) mod app;
pub(crate) mod auth;
pub(crate) mod chat;
pub(crate) mod event;
pub(crate) mod group;
pub(crate) mod guardian;
pub(crate) mod insight;
pub(crate) mod instance;
pub(crate) mod node;
pub(crate) mod overview;
pub(crate) mod pingress;
pub(crate) mod release;
pub(crate) mod scaling;
pub(crate) mod setting;
pub(crate) mod setup;
pub(crate) mod totp;
pub(crate) mod user;
pub(crate) mod volume;
pub(crate) mod xhealth;
pub(crate) mod xjob;

use insight_wire::event::Kind;
use jiff::SignedDuration;

use crate::concept;
use crate::error::{Error, ErrorKind, Result};
use crate::infra;
use crate::locale::Locale;

pub(super) async fn found<T: concept::versioned::Aggregate>(
    loaded: impl Future<Output = Result<Option<concept::versioned::Versioned<T>>>>,
    missing: ErrorKind,
) -> Result<T> {
    Ok(require(loaded, missing).await?.into_inner())
}

pub(super) async fn require<T>(
    loaded: impl Future<Output = Result<Option<T>>>,
    missing: ErrorKind,
) -> Result<T> {
    loaded.await?.ok_or_else(|| missing.into())
}

#[expect(clippy::type_complexity)]
pub(super) async fn sweep_units<T, Fut: Future<Output = Result<bool>>>(
    observed: Result<(Vec<T>, Vec<(String, Error)>)>,
    label: &str,
    kind: Kind,
    subject: impl Fn(&T) -> String,
    status: impl Fn(&T) -> Option<concept::run::State>,
    tick: impl Fn(T) -> Fut,
) -> (usize, Vec<(String, String)>) {
    let (units, errors) = match observed {
        Ok(observed) => observed,
        Err(error) => {
            tracing::warn!(error = ?error, "{label} sweep cannot observe runner services");
            return (0, vec![(format!("/{label}/list"), error.chain(Locale::En))]);
        }
    };
    let complete = errors.is_empty();
    let mut diagnostics: Vec<_> = errors
        .into_iter()
        .map(|(service, error)| (format!("/{label}/read/{service}"), error.chain(Locale::En)))
        .collect();
    let mut triggered = 0;
    let mut seen = std::collections::HashSet::new();
    for unit in units {
        let subject = subject(&unit);
        match status(&unit) {
            Some(concept::run::State::Failed { error, .. }) => {
                infra::event::HUB.raise(kind, &subject, &error, Some(SignedDuration::ZERO));
            }
            Some(concept::run::State::Passed { .. }) => {
                infra::event::HUB.clear(kind, &subject);
            }
            _ => {}
        }
        match tick(unit).await {
            Ok(true) => triggered += 1,
            Ok(false) => {}
            Err(error) if error.is_conflict() => {}
            Err(error) => {
                tracing::warn!(error = ?error, "{label} tick failed for {subject}");
                diagnostics.push((format!("{subject}/{label}/tick"), error.chain(Locale::En)));
            }
        }
        seen.insert(subject);
    }
    if complete {
        infra::event::HUB.retain(kind, &seen);
    }
    (triggered, diagnostics)
}
