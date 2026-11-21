use std::num::NonZeroU16;

use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::{Error, ErrorKind, Result};
use crate::{concept, usecase};

const _: () = assert!(crate::config::XCONFIG_DATA_MAX == 1_023_999);

#[macros_derive::validated_schema]
#[derive(Debug, Deserialize, ToSchema, Validate)]
pub(crate) enum XHealthMethodInput {
    Hurl {
        /// Hurl file content, not a path; reference `{{TARGET_URL}}`. Injected `HURL_VARIABLE_TARGET_URL` overrides plain values and rejects Secret or Dependency bindings.
        #[garde(inner(length(min = 1, max = 1_023_999)))]
        file: Option<String>,
        /// Must be the latest `hurl` version from `version_list.components`.
        #[garde(inner(length(min = 1, max = 255)))]
        version: Option<String>,
    },
    Custom {
        /// Argv in a separate container using the app image; localhost refers to that container. Use an explicit shell to expand variables. Injected `TARGET_HOST`/`TARGET_PORT` and, for Domain, `TARGET_SCHEME` override plain values and reject Secret or Dependency bindings; port targets preserve app-defined `TARGET_SCHEME`.
        #[garde(length(min = 1, max = 32), inner(length(min = 1, max = 256)))]
        command: Vec<String>,
    },
}

impl From<XHealthMethodInput> for usecase::xhealth::MethodParams {
    fn from(method: XHealthMethodInput) -> Self {
        match method {
            XHealthMethodInput::Hurl { file, version } => Self::Hurl {
                file: file.map(String::into_bytes),
                version,
            },
            XHealthMethodInput::Custom { command } => Self::Custom { command },
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) enum XHealthMethod {
    Hurl {
        #[schema(value_type = String)]
        image: concept::image::Reference,
    },
    Custom {
        command: Vec<String>,
    },
}

impl From<&concept::xhealth::Method> for XHealthMethod {
    fn from(method: &concept::xhealth::Method) -> Self {
        match method {
            concept::xhealth::Method::Hurl { image, .. } => Self::Hurl {
                image: image.clone(),
            },
            concept::xhealth::Method::Custom { command } => Self::Custom {
                command: command.clone(),
            },
        }
    }
}

#[macros_derive::validated_schema]
#[derive(Debug, Serialize, Deserialize, ToSchema, Validate)]
pub(crate) enum Target {
    /// Periodic checks use normal DNS and HTTPS on port 443 if any app route for this domain enables it, otherwise HTTP on port 80. Must belong to the app and map to one internal port, even when release checks are disabled.
    Domain(#[garde(custom(super::super::validate::valid_domain))] String),
    /// Periodic checks use a node IP and this published port; Hurl uses HTTP. Must map to one internal port in the app, even when release checks are disabled; Hurl considers TCP mappings only.
    PublishedPort {
        #[garde(range(min = 1))]
        published: u16,
    },
    /// Periodic checks use the app service and this internal port; Hurl uses HTTP.
    InternalPort {
        #[garde(range(min = 1))]
        port: u16,
    },
}

fn nonzero_port(port: u16) -> Result<NonZeroU16> {
    NonZeroU16::new(port)
        .ok_or_else(|| ErrorKind::InvalidBody("port must be non-zero".to_owned()).into())
}

impl TryFrom<Target> for concept::xhealth::Target {
    type Error = Error;

    fn try_from(target: Target) -> Result<Self> {
        match target {
            Target::Domain(domain) => Ok(Self::Domain(domain.parse()?)),
            Target::PublishedPort { published } => Ok(Self::PublishedPort {
                published: nonzero_port(published)?,
            }),
            Target::InternalPort { port } => Ok(Self::InternalPort(nonzero_port(port)?)),
        }
    }
}

impl From<&concept::xhealth::Target> for Target {
    fn from(target: &concept::xhealth::Target) -> Self {
        match target {
            concept::xhealth::Target::Domain(domain) => Self::Domain(domain.to_string()),
            concept::xhealth::Target::PublishedPort { published } => Self::PublishedPort {
                published: published.get(),
            },
            concept::xhealth::Target::InternalPort(port) => Self::InternalPort { port: port.get() },
        }
    }
}

#[macros_derive::validated_schema(patch)]
#[derive(Debug, Deserialize, ToSchema, Validate)]
pub(crate) struct UpdateXHealthRequest {
    #[garde(dive)]
    pub(crate) method: Option<XHealthMethodInput>,
    /// At least 1 minute.
    #[garde(dive)]
    pub(crate) interval: Option<super::super::typ::Duration>,
    #[garde(dive)]
    pub(crate) target: Option<Target>,
    /// Check this app's preview before updates to it or its direct dependencies, separately from periodic checks; omitted means unchanged. Uses the preview's internal port; Hurl uses HTTP and Custom Domain receives `TARGET_SCHEME=http`.
    #[garde(skip)]
    pub(crate) release_check: Option<bool>,
}

#[macros_derive::validated_schema]
#[derive(Debug, Deserialize, ToSchema, Validate)]
pub(crate) struct CreateXHealthRequest {
    #[garde(dive)]
    pub(crate) method: XHealthMethodInput,
    /// At least 1 minute.
    #[garde(dive)]
    pub(crate) interval: super::super::typ::Duration,
    #[garde(dive)]
    pub(crate) target: Target,
    /// Check this app's preview before updates to it or its direct dependencies, separately from periodic checks; defaults to false. Uses the preview's internal port; Hurl uses HTTP and Custom Domain receives `TARGET_SCHEME=http`.
    #[garde(skip)]
    #[serde(default)]
    pub(crate) release_check: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct XHealth {
    pub(crate) method: XHealthMethod,
    pub(crate) interval: super::super::typ::Duration,
    pub(crate) target: Target,
    pub(crate) release_check: bool,
    pub(crate) status: super::super::typ::RunState,
}

impl TryFrom<concept::xhealth::XHealth> for XHealth {
    type Error = Error;

    fn try_from(xhealth: concept::xhealth::XHealth) -> Result<Self> {
        let spec = xhealth.spec();
        Ok(Self {
            method: spec.method().into(),
            interval: spec.interval().try_into()?,
            target: spec.target().into(),
            release_check: spec.guardian(),
            status: xhealth.state().clone().into(),
        })
    }
}
