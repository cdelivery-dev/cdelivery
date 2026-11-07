pub(crate) mod agent;
pub(crate) mod query;
pub(crate) mod server;

use std::collections::HashMap;

use docker::types::{LogDriver, NanoDuration, NetworkAttachment};

use crate::config;
use crate::error::{Result, SpecProblem};

const FLUENTD: &str = "fluentd";
const STOP_GRACE_PERIOD: NanoDuration = NanoDuration::from_secs(30);

pub(crate) fn is_fluentd(driver: Option<&LogDriver>) -> bool {
    driver.is_some_and(|driver| driver.name == FLUENTD)
}

pub(crate) async fn collection_ready(docker: &super::docker::Docker) -> crate::error::Result<bool> {
    Ok(server::exists(docker).await? && agent::observe(docker).await?.is_some())
}

fn env_value<'a>(spec: &'a docker::types::ServiceSpec, name: &str) -> Result<&'a str> {
    let mut value = None;
    for entry in &spec.task_template.container_spec.env {
        let Some((key, current)) = entry.split_once('=') else {
            if entry == name {
                return Err(bad_env(spec, name).into());
            }
            continue;
        };
        if key == name && value.replace(current).is_some() {
            return Err(bad_env(spec, name).into());
        }
    }
    value.ok_or_else(|| {
        SpecProblem::MissingEnv {
            service: spec.name.clone(),
            env: name.to_owned(),
        }
        .into()
    })
}

fn bad_env(spec: &docker::types::ServiceSpec, name: &str) -> SpecProblem {
    SpecProblem::BadEnv {
        service: spec.name.clone(),
        env: name.to_owned(),
    }
}

pub(crate) fn fluentd_log_driver() -> LogDriver {
    LogDriver {
        name: FLUENTD.to_owned(),
        options: Some(HashMap::from([
            (
                "fluentd-address".to_owned(),
                format!("unix://{}", insight_wire::FLUENTD_SOCKET_PATH),
            ),
            ("fluentd-async".to_owned(), "true".to_owned()),
            ("fluentd-buffer-limit".to_owned(), "65536".to_owned()),
            ("fluentd-sub-second-precision".to_owned(), "true".to_owned()),
            (
                "labels".to_owned(),
                "com.docker.swarm.node.id,com.docker.swarm.service.id,com.docker.swarm.service.name"
                    .to_owned(),
            ),
        ])),
    }
}

fn platform_network() -> Vec<NetworkAttachment> {
    vec![NetworkAttachment {
        target: config::NETWORK_NAME.to_owned(),
        aliases: None,
        driver_opts: None,
    }]
}

fn node_id_env() -> String {
    format!("{}={{{{.Node.ID}}}}", insight_wire::NODE_ID_ENV)
}
