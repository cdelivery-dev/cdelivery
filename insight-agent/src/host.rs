use std::path::{Path, PathBuf};

use insight_schema::sample::HostSample;

use crate::procfs::{nth_u64, read_to_buffer};

fn parse_uptime_ms(value: &str) -> Option<u64> {
    let value = value.split_ascii_whitespace().next()?;
    let (seconds, fraction) = value.split_once('.').unwrap_or((value, ""));
    let seconds = seconds.parse::<u64>().ok()?;
    let mut milliseconds = 0_u64;
    for offset in 0..3 {
        milliseconds = milliseconds.saturating_mul(10);
        if let Some(digit) = fraction.as_bytes().get(offset) {
            if !digit.is_ascii_digit() {
                return None;
            }
            milliseconds = milliseconds.saturating_add(u64::from(digit - b'0'));
        }
    }
    seconds.checked_mul(1_000)?.checked_add(milliseconds)
}

fn count_cpu_cores(stat: &str) -> Option<u16> {
    let cores = stat
        .lines()
        .filter(|line| {
            let line = line.trim_start();
            line.starts_with("cpu") && line.as_bytes().get(3).is_some_and(u8::is_ascii_digit)
        })
        .count();
    u16::try_from(cores).ok().filter(|n| *n > 0)
}

#[derive(Debug)]
struct CpuTicks {
    user: u64,
    system: u64,
    idle: u64,
    iowait: u64,
    steal: u64,
}

#[derive(Debug)]
pub(crate) struct HostCollector {
    buffer: String,
    proc_path: PathBuf,
    root_path: PathBuf,
    boot_id: String,
}

impl HostCollector {
    pub(crate) fn new(proc_path: PathBuf) -> std::result::Result<Self, String> {
        let boot_id = std::fs::read_to_string(proc_path.join("sys/kernel/random/boot_id"))
            .map_err(|e| {
                format!(
                    "failed to read {}/sys/kernel/random/boot_id: {e}",
                    proc_path.display()
                )
            })?
            .trim()
            .to_owned();
        if boot_id.is_empty() {
            return Err(format!(
                "{}/sys/kernel/random/boot_id is empty",
                proc_path.display()
            ));
        }

        let root_path = proc_path
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| format!("{} has no parent", proc_path.display()))?;
        let mut collector = Self {
            buffer: String::with_capacity(8192),
            proc_path,
            root_path,
            boot_id,
        };
        collector
            .collect(jiff::Timestamp::now(), "")
            .ok_or_else(|| {
                format!(
                    "failed to collect an initial host sample under {}",
                    collector.proc_path.display()
                )
            })?;
        Ok(collector)
    }

    fn read_file_to_buffer(&mut self, file_name: &str) -> std::io::Result<()> {
        read_to_buffer(&mut self.buffer, &self.proc_path.join(file_name))
    }

    fn collect_uptime_ms(&mut self) -> Option<u64> {
        self.read_file_to_buffer("uptime").ok()?;
        parse_uptime_ms(&self.buffer)
    }

    fn collect_cpu(&mut self) -> Option<(u16, CpuTicks)> {
        self.read_file_to_buffer("stat").ok()?;
        let cores = count_cpu_cores(&self.buffer)?;
        let fields = self
            .buffer
            .lines()
            .find_map(|line| line.strip_prefix("cpu "))?
            .split_ascii_whitespace()
            .collect::<Vec<_>>();
        let user = nth_u64(&fields, 0)?;
        let nice = nth_u64(&fields, 1)?;
        let system = nth_u64(&fields, 2)?;
        let idle = nth_u64(&fields, 3)?;
        let iowait = nth_u64(&fields, 4)?;
        let irq = nth_u64(&fields, 5)?;
        let softirq = nth_u64(&fields, 6)?;
        let steal = nth_u64(&fields, 7)?;
        Some((
            cores,
            CpuTicks {
                user: user.saturating_add(nice),
                system: system.saturating_add(irq).saturating_add(softirq),
                idle,
                iowait,
                steal,
            },
        ))
    }

    fn collect_network(&mut self) -> Option<crate::procfs::NetDevTotals> {
        self.read_file_to_buffer("1/net/dev").ok()?;
        let net_class = self.root_path.join("sys/class/net");
        crate::procfs::sum_net_dev(&self.buffer, |name| {
            net_class.join(name).join("device").exists()
        })
    }

    pub(crate) fn collect(
        &mut self,
        timestamp: jiff::Timestamp,
        node_id: &str,
    ) -> Option<HostSample> {
        let node_uptime_ms = self.collect_uptime_ms()?;
        let (cpu_cores, ticks) = self.collect_cpu()?;
        self.read_file_to_buffer("meminfo").ok()?;
        let keys = [
            "MemTotal:",
            "MemAvailable:",
            "SwapTotal:",
            "SwapFree:",
            "MemFree:",
            "Inactive(file):",
        ];
        let [
            memory_total,
            memory_available,
            swap_total,
            swap_free,
            memory_free,
            inactive_file,
        ] = crate::procfs::pick_kv_u64(&self.buffer, keys)
            .map(|value| value.map(|kb| kb.saturating_mul(1_024)));
        let (psi_cpu_some, _) = crate::procfs::read_pressure(&self.proc_path.join("pressure/cpu"));
        let (psi_memory_some, psi_memory_full) =
            crate::procfs::read_pressure(&self.proc_path.join("pressure/memory"));
        let (psi_io_some, psi_io_full) =
            crate::procfs::read_pressure(&self.proc_path.join("pressure/io"));
        let network = self.collect_network();
        Some(HostSample {
            timestamp,
            node_id: node_id.to_owned(),
            boot_id: self.boot_id.clone(),
            node_uptime_ms,
            cpu_cores,
            cpu_user_ticks: ticks.user,
            cpu_system_ticks: ticks.system,
            cpu_idle_ticks: ticks.idle,
            cpu_iowait_ticks: ticks.iowait,
            cpu_steal_ticks: ticks.steal,
            memory_total_bytes: memory_total?,
            memory_available_bytes: memory_available,
            memory_free_bytes: memory_free,
            memory_inactive_file_bytes: inactive_file,
            swap_total_bytes: swap_total,
            swap_free_bytes: swap_free,
            psi_cpu_some_usec_total: psi_cpu_some,
            psi_memory_some_usec_total: psi_memory_some,
            psi_memory_full_usec_total: psi_memory_full,
            psi_io_some_usec_total: psi_io_some,
            psi_io_full_usec_total: psi_io_full,
            network_interface_set: network.as_ref().map(|totals| totals.interface_set.clone()),
            network_receive_bytes_total: network.as_ref().map(|totals| totals.rx_bytes),
            network_transmit_bytes_total: network.as_ref().map(|totals| totals.tx_bytes),
        })
    }
}
