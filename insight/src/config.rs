use insight_wire::ObjectStoreConfig;
use jiff::SignedDuration;

use crate::error::{ErrorKind, Result};

pub const FLUSH_RETRY_DELAY: SignedDuration = SignedDuration::from_secs(1);
pub const ACCEPT_RETRY_DELAY: SignedDuration = SignedDuration::from_secs(1);
pub const WRITER_IDLE_TIMEOUT: SignedDuration = SignedDuration::from_mins(5);
pub const LOCAL_MAINTENANCE_DELAY: SignedDuration = if cfg!(feature = "benchmark") {
    SignedDuration::from_millis(100)
} else {
    SignedDuration::from_secs(10)
};
pub(crate) const REMOTE_STARTUP_TIMEOUT: SignedDuration = SignedDuration::from_secs(10);
pub const REMOTE_RECOVERY_RETRY_DELAY: SignedDuration = SignedDuration::from_secs(30);
pub const REMOTE_RECOVERY_RETRY_MAX_DELAY: SignedDuration = SignedDuration::from_mins(10);
pub const REMOTE_RESYNC_DELAY: SignedDuration = SignedDuration::from_mins(1);
pub const REMOTE_SWEEP_DELAY: SignedDuration = SignedDuration::from_hours(1);
pub const QUERY_SPILL_MAX_BYTES: u64 = 5 * 1024 * 1024 * 1024;
pub const QUERY_BATCH_SIZE: usize = 1024;
pub const QUERY_SORT_BUFFER_BYTES: usize = 8 * 1024 * 1024;
pub const METRIC_LOOKBACK_NANOS: i64 =
    2 * insight_wire::metric::SAMPLE_INTERVAL_MAX_SECS as i64 * 1_000_000_000;
pub const METRIC_ELAPSED_MAX_MS: i64 = METRIC_LOOKBACK_NANOS / 1_000_000;
pub(crate) const CPU_TTL: SignedDuration = SignedDuration::from_mins(5);
pub(crate) const CPU_GC_INTERVAL: SignedDuration = SignedDuration::from_mins(1);

#[derive(Debug)]
pub struct Config {
    pub storage: ObjectStoreConfig,
    pub node_id: String,
    pub retention: SignedDuration,
    pub writer_flush_interval: SignedDuration,
    pub row_group_max_bytes: usize,
    pub ingest_memory_budget: usize,
    pub query_memory_budget: usize,
    pub search_partitions: usize,
    pub aggregation_partitions: usize,
    pub single_instance: bool,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let key_path = format!("/run/secrets/{}", insight_wire::KEY_SECRET_NAME);
        let cipher = load_key(&key_path)?;
        let storage = cipher.open(&require_env(insight_wire::STORAGE_ENV)?)?;
        let node_id = valid_node_id(require_env(insight_wire::NODE_ID_ENV)?)?;
        let retention = retention_days(&require_env(insight_wire::DATA_RETENTION_DAYS_ENV)?)?;
        let writer_flush_interval = writer_flush_interval_from_env()?;
        let single_instance = single_instance_from_env()?;
        let memory = memory_available();
        let cpu = cpu_available();
        let ingest_memory_budget = ingest_memory_budget(memory);
        let query_memory_budget = query_memory_budget(memory);
        let search_partitions = search_partitions(cpu, memory);
        let aggregation_partitions = aggregation_partitions(cpu, memory);
        tracing::info!(
            "detected {cpu} cpus and {memory} bytes of memory: ingest budget {ingest_memory_budget}, query budget {query_memory_budget}, search partitions {search_partitions}, aggregation partitions {aggregation_partitions}"
        );
        Ok(Self {
            storage,
            node_id,
            retention,
            writer_flush_interval,
            row_group_max_bytes: row_group_tier(memory),
            ingest_memory_budget,
            query_memory_budget,
            search_partitions,
            aggregation_partitions,
            single_instance,
        })
    }
}

fn load_key(path: &str) -> Result<cipher::Cipher> {
    let bytes = std::fs::read(path)?;
    let key: [u8; 32] = bytes
        .try_into()
        .map_err(|_| ErrorKind::Config(format!("key at {path} must be exactly 32 bytes")))?;
    Ok(cipher::Cipher::from_key(&key))
}

fn writer_flush_interval_from_env() -> Result<SignedDuration> {
    let name = insight_wire::WRITER_FLUSH_INTERVAL_SECS_ENV;
    let raw = require_env(name)?;
    raw
        .parse::<i64>()
        .ok()
        .and_then(insight_wire::writer_flush_interval_from_secs)
        .ok_or_else(|| {
            ErrorKind::Config(format!(
                "{name}({raw}) must be a whole-second duration representable as 1..=1000000 seconds, minutes, or hours"
            ))
            .into()
        })
}

fn single_instance_from_env() -> Result<bool> {
    let name = insight_wire::SINGLE_INSTANCE_ENV;
    let raw = require_env(name)?;
    raw.parse::<bool>()
        .map_err(|_| ErrorKind::Config(format!("{name}({raw}) must be true or false")).into())
}

fn require_env(name: &str) -> Result<String> {
    std::env::var(name)
        .map_err(|_| ErrorKind::Config(format!("{name} environment variable must be set")).into())
}

fn valid_node_id(node_id: String) -> Result<String> {
    if !insight_wire::valid_node_id(&node_id) {
        return Err(ErrorKind::Config(format!(
            "NODE_ID({node_id}) must be a resolved Swarm node id"
        ))
        .into());
    }
    Ok(node_id)
}

fn retention_days(raw: &str) -> Result<SignedDuration> {
    let days: i64 = raw.parse().map_err(|_| {
        ErrorKind::Config(format!(
            "DATA_RETENTION_DAYS({raw}) must be an integer day count"
        ))
    })?;
    if !(1..=3650).contains(&days) {
        return Err(ErrorKind::Config(format!(
            "DATA_RETENTION_DAYS({days}) must be within 1..=3650"
        ))
        .into());
    }
    Ok(SignedDuration::from_hours(days * 24))
}

fn row_group_tier(available: u64) -> usize {
    match available {
        a if a >= 1024 * 1024 * 1024 => 8 * 1024 * 1024,
        a if a >= 512 * 1024 * 1024 => 4 * 1024 * 1024,
        _ => 2 * 1024 * 1024,
    }
}

fn memory_available() -> u64 {
    cgroup_v2_limit()
        .or_else(meminfo_total)
        .or_else(sysctl_memsize)
        .unwrap_or_else(|| {
            tracing::warn!("no memory source available, assuming 512 MiB");
            512 * 1024 * 1024
        })
}

fn cgroup_v2_limit() -> Option<u64> {
    std::fs::read_to_string("/sys/fs/cgroup/memory.max")
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn meminfo_total() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb: u64 = meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    Some(kb.saturating_mul(1024))
}

#[cfg(target_os = "macos")]
fn sysctl_memsize() -> Option<u64> {
    let output = std::process::Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()?;
    std::str::from_utf8(&output.stdout)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
}

#[cfg(not(target_os = "macos"))]
fn sysctl_memsize() -> Option<u64> {
    None
}

fn cpu_available() -> usize {
    std::thread::available_parallelism().map_or_else(
        |error| {
            tracing::warn!(error = ?error, "no cpu parallelism source available, assuming 1");
            1
        },
        std::num::NonZeroUsize::get,
    )
}

fn ingest_memory_budget(available: u64) -> usize {
    usize::try_from(available / 5 * 3).unwrap_or(usize::MAX)
}

fn query_memory_budget(available: u64) -> usize {
    usize::try_from((available / 4).max(64 * 1024 * 1024)).unwrap_or(64 * 1024 * 1024)
}

fn search_partitions(cpu: usize, memory: u64) -> usize {
    cpu.min(if memory >= 4 * 1024 * 1024 * 1024 {
        2
    } else {
        1
    })
}

fn aggregation_partitions(cpu: usize, memory: u64) -> usize {
    cpu.min(usize::try_from((memory / (4 * 1024 * 1024 * 1024)).clamp(1, 8)).unwrap_or(1))
}
