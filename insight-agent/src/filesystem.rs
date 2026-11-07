use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::io::Read;
use std::path::Path;

use rustix::fs::{Dev, FileType};

use insight_schema::sample::FilesystemSample;

const MOUNTINFO_MAX_BYTES: u64 = 4 * 1024 * 1024;
const PATH_MAX_BYTES: usize = 4096;
const MOUNT_MAX: usize = 4096;
const DOMAIN_MAX: usize = 128;

#[derive(Debug, Default)]
pub(crate) struct Collector {
    failed: bool,
    last_round: Option<jiff::Timestamp>,
}

impl Collector {
    pub(crate) fn collect_due(
        &mut self,
        now: jiff::Timestamp,
        node_id: &str,
    ) -> Vec<FilesystemSample> {
        let interval = jiff::SignedDuration::from_secs(i64::from(
            insight_wire::metric::FILESYSTEM_SAMPLE_INTERVAL_SECS,
        ));
        if self
            .last_round
            .is_some_and(|last| now.duration_since(last) < interval)
        {
            return Vec::new();
        }
        self.last_round = Some(now);
        match collect_round(now, node_id) {
            Ok(samples) => {
                if self.failed {
                    self.failed = false;
                    tracing::info!("filesystem metrics collection recovered");
                }
                samples
            }
            Err(problem) => {
                if !self.failed {
                    self.failed = true;
                    tracing::warn!("filesystem metrics collection failed: {problem}");
                }
                Vec::new()
            }
        }
    }
}

#[derive(Debug)]
struct Candidate {
    identity: Dev,
    expected_dev: Option<Dev>,
    root: String,
    mount_point: String,
}

impl Candidate {
    fn rank(&self) -> (bool, bool, usize, usize, &str) {
        (
            self.root != "/",
            self.mount_point != "/",
            self.mount_point
                .split('/')
                .filter(|component| !component.is_empty())
                .count(),
            self.mount_point.len(),
            self.mount_point.as_str(),
        )
    }
}

fn collect_round(
    timestamp: jiff::Timestamp,
    node_id: &str,
) -> std::result::Result<Vec<FilesystemSample>, String> {
    let path = crate::config::HOST_PID1_MOUNTINFO_PATH;
    let mut content = String::new();
    std::fs::File::open(path)
        .map_err(|e| format!("open {path}: {e}"))?
        .take(MOUNTINFO_MAX_BYTES + 1)
        .read_to_string(&mut content)
        .map_err(|e| format!("read {path}: {e}"))?;
    if content.len() as u64 > MOUNTINFO_MAX_BYTES {
        return Err(format!("{path} exceeds {MOUNTINFO_MAX_BYTES} bytes"));
    }
    let mut domains: BTreeMap<Dev, Candidate> = BTreeMap::new();
    for (index, line) in content.lines().enumerate() {
        if index >= MOUNT_MAX {
            return Err(format!("{path} exceeds {MOUNT_MAX} mounts"));
        }
        let Some(mount) = eligible(line)? else {
            continue;
        };
        match domains.entry(mount.identity) {
            Entry::Vacant(slot) => {
                slot.insert(mount);
            }
            Entry::Occupied(mut slot) => {
                if mount.rank() < slot.get().rank() {
                    slot.insert(mount);
                }
            }
        }
    }
    if domains.len() > DOMAIN_MAX {
        return Err(format!("more than {DOMAIN_MAX} filesystems"));
    }
    let samples: Vec<FilesystemSample> = domains
        .into_values()
        .filter_map(|mount| measure(mount, timestamp, node_id))
        .collect();
    if samples.is_empty() {
        return Err("no filesystem yielded a sample".to_owned());
    }
    Ok(samples)
}

fn eligible(line: &str) -> std::result::Result<Option<Candidate>, String> {
    let mut fields = line.split(' ');
    take(&mut fields, "mount id")?;
    take(&mut fields, "parent id")?;
    let majmin = take(&mut fields, "device")?;
    let root = take(&mut fields, "root")?;
    let mount_point = take(&mut fields, "mount point")?;
    let mount_options = take(&mut fields, "mount options")?;
    loop {
        match fields.next() {
            Some("-") => break,
            Some(_) => {}
            None => return Err("mountinfo line missing the separator".to_owned()),
        }
    }
    let fstype = take(&mut fields, "filesystem type")?;
    let source = take(&mut fields, "source")?;
    let super_options = take(&mut fields, "super options")?;
    let (major, minor) =
        parse_dev(majmin).ok_or_else(|| format!("invalid device number {majmin}"))?;

    if !has_option(mount_options, "rw") || !has_option(super_options, "rw") {
        return Ok(None);
    }
    let Some(mount_point) = unescape(mount_point).filter(|point| valid_path(point)) else {
        return Ok(None);
    };
    let line_dev = rustix::fs::makedev(major, minor);
    let (identity, expected_dev) = if fstype == "btrfs" {
        let Some(device) = unescape(source)
            .filter(|source| valid_path(source))
            .and_then(|source| block_device(&source))
        else {
            return Ok(None);
        };
        (device, None)
    } else {
        if !block_device_registered(major, minor) {
            return Ok(None);
        }
        (line_dev, Some(line_dev))
    };
    let Some(root) = unescape(root) else {
        return Ok(None);
    };
    Ok(Some(Candidate {
        identity,
        expected_dev,
        root,
        mount_point,
    }))
}

fn take<'a>(fields: &mut std::str::Split<'a, char>, name: &str) -> Result<&'a str, String> {
    fields
        .next()
        .ok_or_else(|| format!("mountinfo line missing the {name} field"))
}

fn has_option(options: &str, wanted: &str) -> bool {
    options.split(',').any(|option| option == wanted)
}

fn parse_dev(majmin: &str) -> Option<(u32, u32)> {
    let (major, minor) = majmin.split_once(':')?;
    Some((major.parse().ok()?, minor.parse().ok()?))
}

fn unescape(raw: &str) -> Option<String> {
    if !raw.contains('\\') {
        return Some(raw.to_owned());
    }
    let mut decoded = Vec::with_capacity(raw.len());
    let mut bytes = raw.bytes();
    while let Some(byte) = bytes.next() {
        if byte != b'\\' {
            decoded.push(byte);
            continue;
        }
        let mut value: u16 = 0;
        for _ in 0..3 {
            let digit = bytes.next()?;
            if !(b'0'..=b'7').contains(&digit) {
                return None;
            }
            value = value * 8 + u16::from(digit - b'0');
        }
        let byte = u8::try_from(value).ok()?;
        if byte == 0 {
            return None;
        }
        decoded.push(byte);
    }
    String::from_utf8(decoded).ok()
}

fn valid_path(path: &str) -> bool {
    path.starts_with('/')
        && path.len() <= PATH_MAX_BYTES
        && !path.split('/').any(|component| component == "..")
}

fn host_path(path: &str) -> String {
    format!("{}{path}", insight_wire::HOST_MOUNT_PREFIX)
}

fn block_device_registered(major: u32, minor: u32) -> bool {
    Path::new(&format!(
        "{}/{major}:{minor}",
        crate::config::HOST_SYS_DEV_BLOCK_DIR
    ))
    .exists()
}

fn block_device(source: &str) -> Option<Dev> {
    let stat = rustix::fs::stat(host_path(source)).ok()?;
    (FileType::from_raw_mode(stat.st_mode) == FileType::BlockDevice).then_some(stat.st_rdev)
}

fn measure(
    mount: Candidate,
    timestamp: jiff::Timestamp,
    node_id: &str,
) -> Option<FilesystemSample> {
    let path = host_path(&mount.mount_point);
    let before = rustix::fs::stat(&path).ok()?;
    if mount
        .expected_dev
        .is_some_and(|expected| before.st_dev != expected)
    {
        return None;
    }
    let vfs = rustix::fs::statvfs(&path).ok()?;
    let after = rustix::fs::stat(&path).ok()?;
    if after.st_dev != before.st_dev || vfs.f_frsize == 0 {
        return None;
    }
    let total_bytes = vfs.f_blocks.checked_mul(vfs.f_frsize)?;
    let free_bytes = vfs.f_bfree.checked_mul(vfs.f_frsize)?;
    let available_bytes = vfs.f_bavail.checked_mul(vfs.f_frsize)?;
    if total_bytes == 0 || free_bytes > total_bytes || available_bytes > free_bytes {
        return None;
    }
    let (inodes_total, inodes_free) = if vfs.f_files > 0 && vfs.f_ffree <= vfs.f_files {
        (Some(vfs.f_files), Some(vfs.f_ffree))
    } else {
        (None, None)
    };
    Some(FilesystemSample {
        timestamp,
        node_id: node_id.to_owned(),
        mount_point: mount.mount_point,
        total_bytes,
        free_bytes,
        available_bytes,
        inodes_total,
        inodes_free,
    })
}
