use std::borrow::Cow;
use std::io::Write;
use std::time::Duration;

pub(crate) use pingress_wire::access::{
    AccessRecord, Host, INDEX_ID, Method, Outcome, PATH_MAX_BYTES,
};

const TRUNCATION_MARKER: &str = "…[truncated]";

pub(crate) fn write_to_stdout(record: &AccessRecord<'_>) {
    let mut line = serde_json::json!(insight_wire::Indexed::new(INDEX_ID, record))
        .to_string()
        .into_bytes();
    line.push(b'\n');
    if let Err(error) = std::io::stdout().lock().write_all(&line) {
        tracing::warn!(error = ?error, "access record stdout write failed");
    }
}

pub(crate) fn truncated_path(path: &str) -> Cow<'_, str> {
    if path.len() <= PATH_MAX_BYTES {
        return Cow::Borrowed(path);
    }
    let end = path.floor_char_boundary(PATH_MAX_BYTES.saturating_sub(TRUNCATION_MARKER.len()));
    let head = path.get(..end).unwrap_or_default();
    Cow::Owned(format!("{head}{TRUNCATION_MARKER}"))
}

pub(crate) fn duration_micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

pub(crate) fn saturating_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
