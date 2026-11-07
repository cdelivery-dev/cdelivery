use serde_with::{DeserializeFromStr, SerializeDisplay};

use crate::error::{Error, ErrorKind, Result};

pub(crate) const MAX: usize = cdelivery_wire::name::MAX;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, SerializeDisplay, DeserializeFromStr)]
pub(crate) struct Name(String);

impl std::str::FromStr for Name {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        if !cdelivery_wire::name::shape_ok(s) {
            return Err(ErrorKind::InvalidName(s.to_owned()).into());
        }
        if cdelivery_wire::name::platform(s) {
            return Err(ErrorKind::ReservedName(s.to_owned()).into());
        }
        Ok(Self(s.to_owned()))
    }
}

impl Name {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
