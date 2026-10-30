use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use insight_schema::sample::{ContainerSample, HostSample};

use crate::error::Result;

const CONTAINER_SERVICE_ID_LABEL: &str = "com.docker.swarm.service.id";
const CONTAINER_SERVICE_NAME_LABEL: &str = "com.docker.swarm.service.name";
const MAPPING_RECONCILE_INTERVAL: Duration = Duration::from_mins(1);

#[derive(Debug)]
struct ContainerInfo {
    service_id: String,
    service_name: String,
    managed: bool,
    container_id: String,
    container_started_at: jiff::Timestamp,
    host_pid: u32,
    host_pid_start_ticks: Option<u64>,
}

#[derive(Debug)]
struct IoStats {
    device_set: String,
    read_bytes: u64,
    write_bytes: u64,
    read_ops: u64,
    write_ops: u64,
}

#[derive(Debug)]
pub(crate) struct Cgroup2 {
    docker: docker::Client,
    node_id: String,
    net_collector: crate::netstat::NetStatCollector,
    docker_cgroup_dir: String,
    directories: Vec<PathBuf>,
    dir_to_container: HashMap<PathBuf, Option<ContainerInfo>>,
    last_reconciled: Option<Instant>,
    mapping_complete: bool,
}

impl Cgroup2 {
    pub(crate) fn new(
        docker: docker::Client,
        node_id: String,
        host_mountinfo_path: &str,
        host_volume_prefix: String,
    ) -> std::result::Result<Self, String> {
        let mount_point = get_mount_point(host_mountinfo_path, &host_volume_prefix)
            .ok_or_else(|| "failed to find cgroup2 mount point".to_owned())?;
        let own_cgroup = find_own_cgroup(mount_point.as_ref())
            .ok_or_else(|| "failed to locate the agent's own cgroup".to_owned())?;
        let docker_cgroup_dir = own_cgroup
            .parent()
            .map(|path| path.display().to_string())
            .ok_or_else(|| "failed to derive docker cgroup base dir".to_owned())?;
        Ok(Self {
            docker,
            node_id,
            net_collector: crate::netstat::NetStatCollector::new(host_volume_prefix),
            docker_cgroup_dir,
            directories: Vec::new(),
            dir_to_container: HashMap::new(),
            last_reconciled: None,
            mapping_complete: false,
        })
    }

    async fn update_dir_to_container(&mut self, dirs: &[PathBuf]) -> Result<bool> {
        let containers = self
            .docker
            .containers()
            .list(false, true, &docker::types::Filter::new())
            .await?;
        let mut mapped = dirs
            .iter()
            .cloned()
            .map(|directory| (directory, None))
            .collect::<HashMap<_, _>>();
        let mut complete = true;
        for container in containers {
            let labels = &container.labels;
            let Some(service_id) = labels.get(CONTAINER_SERVICE_ID_LABEL) else {
                continue;
            };
            let service_name = labels
                .get(CONTAINER_SERVICE_NAME_LABEL)
                .unwrap_or(service_id);
            let Some((directory, _)) = mapped.iter().find(|(directory, _)| {
                directory
                    .to_str()
                    .is_some_and(|path| path.contains(&container.id))
            }) else {
                continue;
            };
            let inspect = match self.docker.containers().inspect(&container.id).await {
                Ok(Some(inspect)) => inspect,
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!(error = ?e, "container {} inspect failed, skipping it this round", container.id);
                    complete = false;
                    continue;
                }
            };
            let host_pid = inspect.state.pid;
            if host_pid == 0 {
                continue;
            }
            mapped.insert(
                directory.clone(),
                Some(ContainerInfo {
                    service_id: service_id.clone(),
                    service_name: service_name.clone(),
                    managed: labels
                        .get(insight_wire::MANAGED_LABEL)
                        .is_some_and(|value| value == "true"),
                    container_id: container.id,
                    container_started_at: inspect.state.started_at,
                    host_pid,
                    host_pid_start_ticks: self.net_collector.pid_start_ticks(host_pid),
                }),
            );
        }
        self.dir_to_container = mapped;
        Ok(complete)
    }

    pub(crate) async fn reconcile(&mut self) -> Result<()> {
        let mut directories = Vec::new();
        for entry in std::fs::read_dir(&self.docker_cgroup_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                directories.push(entry.path());
            }
        }
        directories.sort_unstable();
        let now = Instant::now();
        let periodic = self
            .last_reconciled
            .is_none_or(|last| now.saturating_duration_since(last) >= MAPPING_RECONCILE_INTERVAL);
        let net_collector = &self.net_collector;
        if !self.mapping_complete
            || periodic
            || directories
                .iter()
                .any(|directory| !self.dir_to_container.contains_key(directory))
            || self.dir_to_container.values().any(|container| {
                container.as_ref().is_some_and(|container| {
                    !container.host_pid_start_ticks.is_some_and(|start| {
                        net_collector.pid_start_ticks(container.host_pid) == Some(start)
                    })
                })
            })
        {
            self.mapping_complete = self.update_dir_to_container(&directories).await?;
            self.last_reconciled = Some(now);
        }
        self.directories = directories;
        Ok(())
    }

    pub(crate) fn collect(&mut self, host: &HostSample) -> Vec<ContainerSample> {
        let mut samples = Vec::new();
        for directory in &self.directories {
            let Some(container) = self
                .dir_to_container
                .get(directory)
                .and_then(|container| container.as_ref())
            else {
                continue;
            };
            let network = container
                .host_pid_start_ticks
                .filter(|start| {
                    self.net_collector.pid_start_ticks(container.host_pid) == Some(*start)
                })
                .and_then(|_| self.net_collector.collect(container.host_pid));
            let cpu_limit = read_cpu_limit(directory, host.cpu_cores);
            let [cpu_usage, cpu_throttled] = crate::procfs::pick_kv_file(
                &directory.join("cpu.stat"),
                ["usage_usec", "throttled_usec"],
            );
            let [memory_inactive_file, memory_anon] = crate::procfs::pick_kv_file(
                &directory.join("memory.stat"),
                ["inactive_file", "anon"],
            );
            let [oom_kills] =
                crate::procfs::pick_kv_file(&directory.join("memory.events"), ["oom_kill"]);
            let (psi_cpu_some, _) = crate::procfs::read_pressure(&directory.join("cpu.pressure"));
            let (psi_memory_some, psi_memory_full) =
                crate::procfs::read_pressure(&directory.join("memory.pressure"));
            let (psi_io_some, psi_io_full) =
                crate::procfs::read_pressure(&directory.join("io.pressure"));
            let disk = collect_io(directory);
            samples.push(ContainerSample {
                timestamp: host.timestamp,
                node_id: self.node_id.clone(),
                boot_id: host.boot_id.clone(),
                node_uptime_ms: host.node_uptime_ms,
                elapsed_ms: None,
                service_id: container.service_id.clone(),
                service_name: container.service_name.clone(),
                managed: container.managed,
                container_id: container.container_id.clone(),
                container_started_at: container.container_started_at,
                cpu_quota_usec: cpu_limit.map(|(quota_usec, _)| quota_usec),
                cpu_period_usec: cpu_limit.map(|(_, period_usec)| period_usec),
                cpu_usage_usec_total: cpu_usage,
                cpu_throttled_usec_total: cpu_throttled,
                memory_limit_bytes: read_memory_limit(directory, host.memory_total_bytes),
                memory_current_bytes: crate::procfs::read_value(&directory.join("memory.current")),
                memory_inactive_file_bytes: memory_inactive_file,
                memory_anon_bytes: memory_anon,
                memory_swap_current_bytes: crate::procfs::read_value(
                    &directory.join("memory.swap.current"),
                ),
                memory_oom_kills_total: oom_kills,
                pids_current: crate::procfs::read_value(&directory.join("pids.current")),
                pids_max: crate::procfs::read_value(&directory.join("pids.max")),
                psi_cpu_some_usec_total: psi_cpu_some,
                psi_memory_some_usec_total: psi_memory_some,
                psi_memory_full_usec_total: psi_memory_full,
                psi_io_some_usec_total: psi_io_some,
                psi_io_full_usec_total: psi_io_full,
                disk_device_set: disk.as_ref().map(|stats| stats.device_set.clone()),
                disk_read_bytes_total: disk.as_ref().map(|stats| stats.read_bytes),
                disk_write_bytes_total: disk.as_ref().map(|stats| stats.write_bytes),
                disk_read_ops_total: disk.as_ref().map(|stats| stats.read_ops),
                disk_write_ops_total: disk.as_ref().map(|stats| stats.write_ops),
                network_interface_set: network
                    .as_ref()
                    .map(|(totals, _)| totals.interface_set.clone()),
                network_receive_bytes_total: network.as_ref().map(|(totals, _)| totals.rx_bytes),
                network_transmit_bytes_total: network.as_ref().map(|(totals, _)| totals.tx_bytes),
                tcp_retrans_segs_total: network.as_ref().and_then(|(_, retrans)| *retrans),
                cpu_usage_usec_delta: None,
                cpu_throttled_usec_delta: None,
                memory_oom_kills_delta: None,
                psi_cpu_some_usec_delta: None,
                psi_memory_some_usec_delta: None,
                psi_memory_full_usec_delta: None,
                psi_io_some_usec_delta: None,
                psi_io_full_usec_delta: None,
                disk_read_bytes_delta: None,
                disk_write_bytes_delta: None,
                disk_read_ops_delta: None,
                disk_write_ops_delta: None,
                network_receive_bytes_delta: None,
                network_transmit_bytes_delta: None,
                tcp_retrans_segs_delta: None,
            });
        }
        samples
    }
}

fn read_cpu_limit(directory: &Path, cpu_cores: u16) -> Option<(u64, u64)> {
    let content = std::fs::read_to_string(directory.join("cpu.max")).ok()?;
    let mut parts = content.split_whitespace();
    let quota = parts.next()?;
    let period_usec = parts
        .next()?
        .parse::<u64>()
        .ok()
        .filter(|period| *period > 0)?;
    let quota_usec = if quota.eq_ignore_ascii_case("max") {
        u64::from(cpu_cores).saturating_mul(period_usec)
    } else {
        quota.parse().ok()?
    };
    Some((quota_usec, period_usec))
}

fn read_memory_limit(directory: &Path, memory_total_bytes: u64) -> Option<u64> {
    let value = std::fs::read_to_string(directory.join("memory.max")).ok()?;
    let value = value.trim();
    if value.eq_ignore_ascii_case("max") {
        Some(memory_total_bytes)
    } else {
        value.parse().ok()
    }
}

fn get_mount_point(host_mountinfo_path: &str, host_volume_prefix: &str) -> Option<String> {
    std::fs::read_to_string(host_mountinfo_path)
        .ok()?
        .lines()
        .find_map(|line| {
            let parts = line.splitn(2, " - ").collect::<Vec<_>>();
            if parts.len() != 2 {
                return None;
            }
            let pre = parts.first()?.split_whitespace().collect::<Vec<_>>();
            let post = parts.last()?.split_whitespace().collect::<Vec<_>>();
            if pre.len() < 5 || post.len() < 3 || *post.first()? != "cgroup2" {
                return None;
            }
            let mount_point = pre.get(4)?;
            Some(format!(
                "{host_volume_prefix}{}",
                mount_point.trim_end_matches('/')
            ))
        })
}

fn cgroup_contains_pid(cgroup_dir: &Path, pid: &str) -> bool {
    std::fs::read_to_string(cgroup_dir.join("cgroup.procs"))
        .is_ok_and(|content| content.lines().any(|line| line.trim() == pid))
}

fn find_own_cgroup(cgroup_root: &Path) -> Option<PathBuf> {
    let own_pid = std::process::id().to_string();
    let mut directories = vec![(cgroup_root.to_path_buf(), 0)];
    let max_depth = 2;
    while let Some((current, depth)) = directories.pop() {
        if depth > max_depth {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&current) {
            for entry in entries.flatten() {
                if let Ok(file_type) = entry.file_type()
                    && file_type.is_dir()
                {
                    let path = entry.path();
                    if cgroup_contains_pid(&path, &own_pid) {
                        return Some(path);
                    }
                    directories.push((path, depth + 1));
                }
            }
        }
    }
    None
}

fn collect_io(directory: &Path) -> Option<IoStats> {
    let content = std::fs::read_to_string(directory.join("io.stat")).ok()?;
    let mut devices = Vec::new();
    let mut read_bytes = 0_u64;
    let mut write_bytes = 0_u64;
    let mut read_ops = 0_u64;
    let mut write_ops = 0_u64;
    for line in content.lines() {
        let mut parts = line.split_whitespace();
        let Some(device) = parts.next() else {
            continue;
        };
        let mut rbytes = None;
        let mut wbytes = None;
        let mut rios = None;
        let mut wios = None;
        for part in parts {
            let Some((key, value)) = part.split_once('=') else {
                continue;
            };
            let Ok(value) = value.parse::<u64>() else {
                continue;
            };
            match key {
                "rbytes" => rbytes = Some(value),
                "wbytes" => wbytes = Some(value),
                "rios" => rios = Some(value),
                "wios" => wios = Some(value),
                _ => {}
            }
        }
        let (Some(rbytes), Some(wbytes), Some(rios), Some(wios)) = (rbytes, wbytes, rios, wios)
        else {
            continue;
        };
        read_bytes = read_bytes.saturating_add(rbytes);
        write_bytes = write_bytes.saturating_add(wbytes);
        read_ops = read_ops.saturating_add(rios);
        write_ops = write_ops.saturating_add(wios);
        devices.push(device);
    }
    if devices.is_empty() {
        return None;
    }
    devices.sort_unstable();
    devices.dedup();
    Some(IoStats {
        device_set: devices.join(","),
        read_bytes,
        write_bytes,
        read_ops,
        write_ops,
    })
}
