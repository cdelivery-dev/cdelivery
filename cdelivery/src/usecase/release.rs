use std::sync::Arc;

use crate::error::{self, ErrorKind, Result};
use crate::{concept, infra};

pub(crate) fn list(releases: &infra::release::Catalog) -> Result<Arc<concept::release::Snapshot>> {
    releases
        .list()
        .ok_or_else(|| ErrorKind::ReleaseUnavailable.into())
}

pub(super) fn resolve(
    releases: &infra::release::Catalog,
    component: concept::release::Component,
    version: &str,
    field: &str,
) -> Result<concept::image::Reference> {
    let snapshot = list(releases)?;
    let release = snapshot
        .get(&component)
        .ok_or(ErrorKind::ReleaseUnavailable)?;
    concept::release::parse_version(version)
        .ok()
        .and_then(|wanted| release.resolve(&wanted))
        .ok_or_else(|| {
            ErrorKind::VersionNotLatest {
                field: field.to_owned(),
                latest: release.version().to_string(),
            }
            .into()
        })
}

pub(super) fn select(
    releases: &infra::release::Catalog,
    component: concept::release::Component,
    version: Option<&str>,
    current: Option<&concept::image::Reference>,
    field: &str,
) -> Result<concept::image::Reference> {
    match version {
        Some(version) => resolve(releases, component, version, field),
        None => error::required(current.cloned(), field),
    }
}
