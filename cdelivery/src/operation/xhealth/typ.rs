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
        /// Hurl file content, not a path.
        #[garde(inner(length(min = 1, max = 1_023_999)))]
        file: Option<String>,
        /// Must be the latest `hurl` version from `version_list`.
        #[garde(inner(length(min = 1, max = 255)))]
        version: Option<String>,
    },
    Custom {
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
    Domain(#[garde(custom(super::super::validate::valid_domain))] String),
    PublishedPort {
        #[garde(range(min = 1))]
        published: u16,
    },
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
    #[garde(skip)]
    pub(crate) guardian: Option<bool>,
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
    #[garde(skip)]
    #[serde(default)]
    pub(crate) guardian: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct XHealth {
    pub(crate) method: XHealthMethod,
    pub(crate) interval: super::super::typ::Duration,
    pub(crate) target: Target,
    pub(crate) guardian: bool,
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
            guardian: spec.guardian(),
            status: xhealth.state().clone().into(),
        })
    }
}
