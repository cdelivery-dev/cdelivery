use crate::error::{ErrorKind, Result};

pub(crate) const HOST_PROC_DIR: &str = "/host/proc";
pub(crate) const HOST_MOUNTINFO_PATH: &str = "/host/proc/self/mountinfo";
pub(crate) const HOST_PID1_MOUNTINFO_PATH: &str = "/host/proc/1/mountinfo";
pub(crate) const HOST_SYS_DEV_BLOCK_DIR: &str = "/host/sys/dev/block";

pub(crate) fn node_id_from_env() -> Result<String> {
    let node_id = require_env(insight_wire::NODE_ID_ENV)?;
    if !insight_wire::valid_node_id(&node_id) {
        return Err(ErrorKind::Config(format!(
            "NODE_ID({node_id}) must be a resolved Swarm node id"
        ))
        .into());
    }
    Ok(node_id)
}

pub(crate) fn sample_interval_from_env() -> Result<u16> {
    let name = insight_wire::SAMPLE_INTERVAL_SECS_ENV;
    let raw = require_env(name)?;
    let range = insight_wire::metric::SAMPLE_INTERVAL_MIN_SECS
        ..=insight_wire::metric::SAMPLE_INTERVAL_MAX_SECS;
    raw.parse::<u16>()
        .ok()
        .filter(|secs| range.contains(secs))
        .ok_or_else(|| {
            ErrorKind::Config(format!("{name}({raw}) must be an integer within {range:?}")).into()
        })
}

fn require_env(name: &str) -> Result<String> {
    std::env::var(name)
        .map_err(|_| ErrorKind::Config(format!("{name} environment variable must be set")).into())
}
