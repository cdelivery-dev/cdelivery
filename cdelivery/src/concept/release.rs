use getset::{CopyGetters, Getters};
use strum::{AsRefStr, EnumString};

use crate::error::{ErrorKind, Result};

pub(crate) fn parse_version(version: &str) -> Result<semver::Version> {
    let trimmed = version.trim();
    let normalized = trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('V'))
        .unwrap_or(trimmed);
    semver::Version::parse(normalized)
        .map_err(|_| ErrorKind::TagNotSemver(version.to_owned()).into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, EnumString, AsRefStr)]
#[strum(serialize_all = "snake_case")]
pub(crate) enum Component {
    Cdelivery,
    CdeliveryAgent,
    Pingress,
    Insight,
    InsightAgent,
    Hurl,
}

#[derive(Debug, Getters, CopyGetters)]
pub(crate) struct Release {
    #[getset(get = "pub(crate)")]
    image: super::image::Reference,
    #[getset(get = "pub(crate)")]
    version: semver::Version,
    #[getset(get = "pub(crate)")]
    changelog: String,
    #[getset(get_copy = "pub(crate)")]
    important: bool,
}

impl Release {
    pub(crate) fn try_new(image: &str, changelog: String, important: bool) -> Result<Self> {
        let reference: super::image::Reference = image.parse()?;
        if image.contains('@') {
            return Err(
                ErrorKind::InvalidBody("release image must not contain a digest".into()).into(),
            );
        }
        let version = parse_version(reference.tag())?;
        Ok(Self {
            image: reference,
            version,
            changelog,
            important,
        })
    }

    pub(crate) fn resolve(&self, version: &semver::Version) -> Option<super::image::Reference> {
        (self.version == *version).then(|| self.image.clone())
    }
}

pub(crate) type Snapshot = std::collections::BTreeMap<Component, Release>;
