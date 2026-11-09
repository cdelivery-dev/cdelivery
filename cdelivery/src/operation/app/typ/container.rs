use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::concept;
use crate::error::{Error, Result};

#[macros_derive::validated_schema]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Validate)]
pub(crate) struct Mount {
    #[garde(
        length(min = 2, max = 255),
        custom(super::super::super::validate::valid_absolute_path)
    )]
    pub(crate) target: String,
    #[garde(custom(super::super::super::validate::valid_name))]
    #[garde(custom(super::super::super::validate::available_volume_name))]
    pub(crate) volume: String,
    #[garde(skip)]
    #[serde(default)]
    pub(crate) read_only: bool,
}

impl From<&Mount> for concept::app::Mount {
    fn from(mount: &Mount) -> Self {
        Self::new(
            mount.target.clone(),
            mount.volume.clone(),
            mount.read_only,
            cdelivery_wire::Backend::Local,
        )
    }
}

impl From<&concept::app::Mount> for Mount {
    fn from(mount: &concept::app::Mount) -> Self {
        Self {
            target: mount.target().clone(),
            volume: mount.volume().clone(),
            read_only: mount.read_only(),
        }
    }
}

#[macros_derive::validated_schema]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Validate)]
pub(crate) struct NodeMounts {
    #[garde(custom(super::super::super::validate::valid_node_id))]
    pub(crate) node_id: String,
    #[garde(length(min = 1), dive)]
    pub(crate) mounts: Vec<Mount>,
}

super::super::super::typ::mirror_struct! {
    NodeMounts(m: &NodeMounts) <-> concept::app::NodeMounts(c: &concept::app::NodeMounts) {
        node_id: m.node_id.clone() => c.node_id().clone(),
        mounts: m.mounts.iter().map(Into::into).collect()
            => c.mounts().iter().map(Into::into).collect(),
    }
}

#[macros_derive::validated_schema]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Validate)]
pub(crate) struct EnvVar {
    #[garde(length(min = 1, max = 128), pattern(r"^[^=]+$"))]
    pub(crate) name: String,
    #[garde(dive)]
    pub(crate) value: EnvValue,
}

#[macros_derive::validated_schema]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Validate)]
#[serde(untagged)]
pub(crate) enum EnvValue {
    Plain(#[garde(length(max = 4096))] String),
    Secret(#[garde(dive)] super::super::super::secret::typ::Reference),
}

#[macros_derive::validated_schema]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Validate)]
pub(crate) struct ContainerOptions {
    #[garde(inner(length(max = 4096)))]
    #[serde(default)]
    pub(crate) command: Vec<String>,
    #[garde(dive)]
    #[serde(default)]
    pub(crate) env: Vec<EnvVar>,
}

impl ContainerOptions {
    pub(crate) fn set_command(&mut self, command: Vec<String>) {
        self.command = command;
    }

    pub(super) fn from_command(command: Vec<String>) -> Self {
        Self {
            command,
            env: Vec::new(),
        }
    }
}

impl TryFrom<&ContainerOptions> for concept::app::ContainerOptions {
    type Error = Error;

    fn try_from(options: &ContainerOptions) -> Result<Self> {
        Ok(Self::new(
            options.command.clone(),
            options
                .env
                .iter()
                .map(|env| {
                    Ok((
                        env.name.clone(),
                        match &env.value {
                            EnvValue::Plain(value) => concept::app::EnvValue::Plain(value.clone()),
                            EnvValue::Secret(reference) => {
                                concept::app::EnvValue::Secret(reference.try_into()?)
                            }
                        },
                    ))
                })
                .collect::<Result<_>>()?,
        ))
    }
}

impl From<&concept::app::ContainerOptions> for ContainerOptions {
    fn from(options: &concept::app::ContainerOptions) -> Self {
        Self {
            command: options.command().clone(),
            env: options
                .env()
                .iter()
                .map(|(name, value)| EnvVar {
                    name: name.clone(),
                    value: match value {
                        concept::app::EnvValue::Plain(value) => EnvValue::Plain(value.clone()),
                        concept::app::EnvValue::Secret(reference) => {
                            EnvValue::Secret(reference.into())
                        }
                    },
                })
                .collect(),
        }
    }
}

#[macros_derive::validated_schema]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Validate)]
#[serde(tag = "driver")]
pub(crate) enum LogDriver {
    FollowHost,
    JsonFile {
        /// From 1 MiB to 1 GiB.
        #[garde(dive, custom(super::super::super::validate::log_max_size))]
        max_size: super::super::super::typ::ByteSize,
        #[garde(range(min = 1, max = 100))]
        max_file: u64,
    },
    Insight,
}

impl From<&LogDriver> for concept::app::LogDriver {
    fn from(driver: &LogDriver) -> Self {
        match driver {
            LogDriver::FollowHost => Self::FollowHost,
            LogDriver::JsonFile { max_size, max_file } => Self::JsonFile {
                max_size: u64::from(*max_size),
                max_file: *max_file,
            },
            LogDriver::Insight => Self::Insight,
        }
    }
}

impl TryFrom<&concept::app::LogDriver> for LogDriver {
    type Error = Error;

    fn try_from(driver: &concept::app::LogDriver) -> Result<Self> {
        Ok(match driver {
            concept::app::LogDriver::FollowHost => Self::FollowHost,
            concept::app::LogDriver::JsonFile { max_size, max_file } => Self::JsonFile {
                max_size: super::super::super::typ::ByteSize::try_from(*max_size)?,
                max_file: *max_file,
            },
            concept::app::LogDriver::Insight => Self::Insight,
        })
    }
}
