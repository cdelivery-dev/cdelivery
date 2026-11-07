use http::StatusCode;

use super::Client;
use crate::error::{Error, ErrorKind, Result};
use crate::types::{DockerVersion, InspectSwarm, SystemInfo};

#[derive(Debug)]
pub struct System<'a>(pub(crate) &'a Client);

impl System<'_> {
    pub async fn version(&self) -> Result<DockerVersion> {
        self.0
            .get_opt(super::VERSION_PATH, &[])
            .await?
            .ok_or_else(|| missing("version"))
    }

    pub async fn info(&self) -> Result<SystemInfo> {
        self.0
            .get_opt("/info", &[])
            .await?
            .ok_or_else(|| missing("info"))
    }

    pub async fn swarm(&self) -> Result<InspectSwarm> {
        self.0
            .get_opt("/swarm", &[])
            .await?
            .ok_or_else(|| missing("swarm"))
    }
}

fn missing(what: &str) -> Error {
    Error::new(ErrorKind::Docker {
        code: StatusCode::NOT_FOUND,
        msg: format!("{what} endpoint returned no body"),
    })
}
