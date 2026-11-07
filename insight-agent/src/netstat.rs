use std::path::Path;

use crate::procfs::read_to_buffer;

#[derive(Debug)]
pub(crate) struct NetStatCollector {
    host_volume_prefix: String,
    buffer: String,
}

impl NetStatCollector {
    pub(crate) fn new(host_volume_prefix: String) -> Self {
        Self {
            host_volume_prefix,
            buffer: String::with_capacity(4096),
        }
    }

    fn pid_path(&self, pid: u32) -> std::path::PathBuf {
        Path::new(&self.host_volume_prefix)
            .join("proc")
            .join(pid.to_string())
    }

    pub(crate) fn pid_start_ticks(&self, pid: u32) -> Option<u64> {
        let stat = std::fs::read_to_string(self.pid_path(pid).join("stat")).ok()?;
        let (_, fields) = stat.rsplit_once(") ")?;
        fields.split_ascii_whitespace().nth(19)?.parse().ok()
    }

    pub(crate) fn collect(
        &mut self,
        pid: u32,
    ) -> Option<(crate::procfs::NetDevTotals, Option<u64>)> {
        let net_dir = self.pid_path(pid).join("net");
        let tcp_retrans_segs = read_to_buffer(&mut self.buffer, &net_dir.join("snmp"))
            .ok()
            .and_then(|()| parse_retrans(&self.buffer));
        read_to_buffer(&mut self.buffer, &net_dir.join("dev")).ok()?;
        let totals = crate::procfs::sum_net_dev(&self.buffer, |name| name != "lo")?;
        Some((totals, tcp_retrans_segs))
    }
}

fn parse_retrans(content: &str) -> Option<u64> {
    let mut lines = content.lines().filter(|line| line.starts_with("Tcp:"));
    let headers = lines.next()?;
    let values = lines.next()?;
    let index = headers
        .split_ascii_whitespace()
        .position(|header| header == "RetransSegs")?;
    values.split_ascii_whitespace().nth(index)?.parse().ok()
}
