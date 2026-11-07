use macros::located_error;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ErrorKind {
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("invalid configuration: {0}")]
    Config(String),
    #[error("invariant violation: {0}")]
    Invariant(String),
    #[error("I/O error: {0}")]
    Io(#[source] std::io::Error),
    #[error("DataFusion error: {0}")]
    DataFusion(#[source] datafusion::error::DataFusionError),
    #[error("Arrow error: {0}")]
    Arrow(#[source] datafusion::arrow::error::ArrowError),
    #[error("timestamp error: {0}")]
    Timestamp(#[source] jiff::Error),
    #[error("Parquet error: {0}")]
    Parquet(#[source] parquet::errors::ParquetError),
    #[error("object store error: {0}")]
    ObjectStore(#[source] object_store::Error),
    #[error("remote storage startup timed out: {0}")]
    RemoteStartupTimeout(#[source] tokio::time::error::Elapsed),
    #[error("healthz probe to {addr} timed out: {source}")]
    HealthzTimeout {
        addr: std::net::SocketAddr,
        #[source]
        source: tokio::time::error::Elapsed,
    },
    #[error("schema error: {0}")]
    Schema(#[source] insight_schema::Error),
    #[error("storage config rejected: {0}")]
    Unseal(#[source] cipher::Error),
    #[error("runtime error: {0}")]
    Runtime(#[source] logging::Error),
}

impl Error {
    pub(crate) fn kind(&self) -> &ErrorKind {
        &self.kind
    }
}

located_error! {
    Error => ErrorKind,
    from {
        std::io::Error => Io,
        datafusion::error::DataFusionError => DataFusion,
        datafusion::arrow::error::ArrowError => Arrow,
        jiff::Error => Timestamp,
        parquet::errors::ParquetError => Parquet,
        object_store::Error => ObjectStore,
        insight_schema::Error => Schema,
        cipher::Error => Unseal,
        logging::Error => Runtime,
    }
}
