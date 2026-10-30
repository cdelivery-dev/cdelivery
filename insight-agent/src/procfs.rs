use std::io::Read;

pub(crate) fn read_to_buffer(buffer: &mut String, path: &std::path::Path) -> std::io::Result<()> {
    buffer.clear();
    std::fs::File::open(path)?.read_to_string(buffer)?;
    Ok(())
}

pub(crate) fn nth_u64(fields: &[&str], i: usize) -> Option<u64> {
    fields.get(i)?.parse().ok()
}

pub(crate) fn pick_kv_u64<const N: usize>(content: &str, keys: [&str; N]) -> [Option<u64>; N] {
    let mut picked = [None; N];
    for line in content.lines() {
        let mut parts = line.split_whitespace();
        if let (Some(key), Some(value)) = (parts.next(), parts.next())
            && let Ok(value) = value.parse::<u64>()
        {
            for (slot, wanted) in picked.iter_mut().zip(keys) {
                if wanted == key {
                    *slot = Some(value);
                }
            }
        }
    }
    picked
}

pub(crate) fn pick_kv_file<const N: usize>(
    path: &std::path::Path,
    keys: [&str; N],
) -> [Option<u64>; N] {
    std::fs::read_to_string(path).map_or([None; N], |content| pick_kv_u64(&content, keys))
}

pub(crate) fn read_value<T: std::str::FromStr>(path: &std::path::Path) -> Option<T> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

pub(crate) fn read_pressure(path: &std::path::Path) -> (Option<u64>, Option<u64>) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return (None, None);
    };
    let mut some = None;
    let mut full = None;
    for line in content.lines() {
        let mut parts = line.split_ascii_whitespace();
        let slot = match parts.next() {
            Some("some") => &mut some,
            Some("full") => &mut full,
            _ => continue,
        };
        *slot = parts.find_map(|part| part.strip_prefix("total=")?.parse().ok());
    }
    (some, full)
}

#[derive(Debug)]
pub(crate) struct NetDevTotals {
    pub(crate) interface_set: String,
    pub(crate) rx_bytes: u64,
    pub(crate) tx_bytes: u64,
}

pub(crate) fn sum_net_dev(
    content: &str,
    mut include: impl FnMut(&str) -> bool,
) -> Option<NetDevTotals> {
    let mut interfaces = Vec::new();
    let mut rx_total = 0_u64;
    let mut tx_total = 0_u64;
    for line in content.lines().skip(2) {
        let Some((name, data)) = line.trim_start().split_once(':') else {
            continue;
        };
        let name = name.trim();
        if !include(name) {
            continue;
        }
        let fields: Vec<&str> = data.split_ascii_whitespace().collect();
        let (Some(rx), Some(tx)) = (nth_u64(&fields, 0), nth_u64(&fields, 8)) else {
            continue;
        };
        interfaces.push(name.to_owned());
        rx_total = rx_total.saturating_add(rx);
        tx_total = tx_total.saturating_add(tx);
    }
    if interfaces.is_empty() {
        return None;
    }
    interfaces.sort_unstable();
    interfaces.dedup();
    Some(NetDevTotals {
        interface_set: interfaces.join(","),
        rx_bytes: rx_total,
        tx_bytes: tx_total,
    })
}
