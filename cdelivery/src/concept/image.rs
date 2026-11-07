use getset::Getters;
use serde::{Deserialize, Serialize};

use crate::error::{Error, ErrorKind, Result};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Getters, macros_derive::Ctor)]
#[getset(get = "pub(crate)")]
pub(crate) struct ImageAuth {
    username: String,
    token: String,
}

impl ImageAuth {
    pub(crate) fn inherit(&self, current: Option<Self>) -> Option<Self> {
        if self.token.is_empty() {
            current.filter(|current| current.username == self.username)
        } else {
            Some(self.clone())
        }
    }
}

impl std::fmt::Debug for ImageAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageAuth")
            .field("username", &self.username)
            .field("token", &"<redacted>")
            .finish()
    }
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Getters,
    serde_with::SerializeDisplay,
    serde_with::DeserializeFromStr,
)]
pub(crate) struct Reference {
    #[getset(get = "pub(crate)")]
    registry: String,
    repository: String,
    #[getset(get = "pub(crate)")]
    tag: String,
}

impl std::fmt::Display for Reference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}:{}", self.registry, self.repository, self.tag)
    }
}

impl Reference {
    pub(crate) fn prefix(&self) -> String {
        match self.repository.rsplit_once('/') {
            Some((parent, _)) => format!("{}/{parent}", self.registry),
            None => self.registry.clone(),
        }
    }

    pub(crate) fn replacement_for(&self, current: &str) -> Result<Option<String>> {
        Ok((self != &current.parse::<Self>()?).then(|| self.to_string()))
    }

    pub(crate) fn same_registry(&self, other: &Self) -> bool {
        self.registry().eq_ignore_ascii_case(other.registry())
    }
}

impl std::str::FromStr for Reference {
    type Err = Error;

    fn from_str(reference: &str) -> Result<Self> {
        let image: oci_spec::distribution::Reference = reference.parse()?;
        if image
            .repository()
            .bytes()
            .any(|byte| byte.is_ascii_uppercase())
        {
            return Err(oci_spec::distribution::ParseError::NameContainsUppercase.into());
        }
        let tag = image
            .tag()
            .ok_or_else(|| ErrorKind::InvalidBody("image must have a tag".into()))?;
        if !tag.is_ascii() {
            return Err(oci_spec::distribution::ParseError::TagInvalidFormat.into());
        }
        Ok(Self {
            registry: image.registry().to_owned(),
            repository: image.repository().to_owned(),
            tag: tag.to_owned(),
        })
    }
}
