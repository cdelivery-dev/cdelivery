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

super::super::super::typ::mirror_struct! {
    Mount(m: &Mount) <-> concept::app::Mount(c: &concept::app::Mount) {
        target: m.target.clone() => c.target().clone(),
        volume: m.volume.clone() => c.volume().clone(),
        read_only: m.read_only => c.read_only(),
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
    #[garde(length(max = 4096))]
    pub(crate) value: String,
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

super::super::super::typ::mirror_struct! {
    ContainerOptions(o: &ContainerOptions) <-> concept::app::ContainerOptions(c: &concept::app::ContainerOptions) {
        command: o.command.clone() => c.command().clone(),
        env: o.env.iter().map(|e| (e.name.clone(), e.value.clone())).collect()
            => c.env().iter().map(|(name, value)| EnvVar { name: name.clone(), value: value.clone() }).collect(),
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
