use getset::{CopyGetters, Getters};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, EnumString};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, AsRefStr, EnumString)]
pub(crate) enum Kind {
    Text,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, macros_derive::Ctor)]
#[getset(get = "pub(crate)")]
pub(crate) struct Reference {
    name: super::name::Name,
    id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Getters, CopyGetters, macros_derive::Ctor)]
pub(crate) struct Secret {
    #[getset(get = "pub(crate)")]
    reference: Reference,
    #[getset(get_copy = "pub(crate)")]
    kind: Kind,
    #[getset(get = "pub(crate)")]
    description: Option<String>,
    #[getset(get_copy = "pub(crate)")]
    created_at: Timestamp,
}

impl super::versioned::Aggregate for Secret {}
