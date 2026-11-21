use getset::{CopyGetters, Getters};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, EnumString};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, AsRefStr, EnumString)]
pub(crate) enum Provider {
    Neon,
    PlanetScale,
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Reference {
    App(super::name::Name),
    PostgreSQL(super::secret::Reference),
}

impl Reference {
    pub(crate) fn index(&self) -> String {
        match self {
            Self::App(name) => format!("app.{name}"),
            Self::PostgreSQL(reference) => format!("postgres.{}", reference.id()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, macros_derive::Ctor)]
#[getset(get = "pub(crate)")]
pub(crate) struct Binding {
    dependency: Reference,
    field: String,
}

impl Binding {
    pub(crate) fn validate(&self) -> crate::error::Result<()> {
        let valid = match self.dependency {
            Reference::App(_) => self.field == "host",
            Reference::PostgreSQL(_) => matches!(
                self.field.as_str(),
                "dsn" | "host" | "port" | "database" | "username" | "password"
            ),
        };
        if valid {
            Ok(())
        } else {
            Err(crate::error::ErrorKind::SecretBindingInvalid.into())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Getters, CopyGetters, macros_derive::Ctor)]
pub(crate) struct Dependency {
    #[getset(get = "pub(crate)")]
    reference: super::secret::Reference,
    #[getset(get = "pub(crate)")]
    group: Option<super::name::Name>,
    #[getset(get_copy = "pub(crate)")]
    provider: Provider,
    #[getset(get = "pub(crate)")]
    local_app: Option<super::name::Name>,
    #[getset(get = "pub(crate)")]
    cloud_provider: Option<String>,
    #[getset(get = "pub(crate)")]
    architecture: Option<String>,
    #[getset(get = "pub(crate)")]
    description: Option<String>,
    #[getset(get_copy = "pub(crate)")]
    created_at: Timestamp,
}

impl super::versioned::Aggregate for Dependency {}
