use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum ErrorKind {
    #[error("I/O failed: {0}")]
    Io(#[source] std::io::Error),
    #[error("executor key must contain 32 bytes: {0}")]
    Key(#[source] std::array::TryFromSliceError),
    #[error("encrypted message failed: {0}")]
    Cipher(#[source] cipher::Error),
    #[error("invalid dependency: {0}")]
    Fields(#[source] cdelivery_wire::postgres::Error),
    #[error("provider request failed: {0}")]
    Http(#[source] reqwest::Error),
    #[error("provider returned invalid JSON: {0}")]
    Json(#[source] serde_json::Error),
    #[error("invalid provider URL: {0}")]
    Url(#[source] url::ParseError),
    #[error("execution task failed: {0}")]
    Task(#[source] tokio::task::JoinError),
    #[error("logging setup failed: {0}")]
    Logging(#[source] tracing_subscriber::util::TryInitError),
    #[error("PostgreSQL error: {0}")]
    Postgres(#[source] libpq::errors::Error),
    #[error("{0}")]
    Invalid(String),
    #[error("provider returned HTTP {status}: {message}")]
    Provider { status: u16, message: String },
}

macros::located_error! {
    Error => ErrorKind,
    from {
        std::io::Error => Io,
        std::array::TryFromSliceError => Key,
        cipher::Error => Cipher,
        cdelivery_wire::postgres::Error => Fields,
        reqwest::Error => Http,
        serde_json::Error => Json,
        url::ParseError => Url,
        tokio::task::JoinError => Task,
        tracing_subscriber::util::TryInitError => Logging,
        libpq::errors::Error => Postgres,
    }
}
