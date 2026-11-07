use macros::located_error;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum ErrorKind {
    #[error("Arrow error: {0}")]
    Arrow(#[source] arrow::error::ArrowError),
    #[error("{label} timestamp {timestamp} outside nanosecond range")]
    TimestampRange {
        label: &'static str,
        timestamp: jiff::Timestamp,
    },
}

located_error! {
    Error => ErrorKind,
    from {
        arrow::error::ArrowError => Arrow,
    }
}
