pub const MAX: usize = 16;
pub const AGENT_VOLUME: &str = "agent";
pub const INSIGHT_VOLUME: &str = "insight-data";

pub fn shape_ok(s: &str) -> bool {
    let charset_ok = s
        .bytes()
        .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-'));
    !s.is_empty() && s.len() <= MAX && charset_ok && !s.starts_with('-') && !s.ends_with('-')
}

pub fn platform(s: &str) -> bool {
    s == "cdelivery" || s.starts_with("cdelivery-")
}

pub fn volume_reserved(s: &str) -> bool {
    matches!(s, AGENT_VOLUME | INSIGHT_VOLUME)
}
