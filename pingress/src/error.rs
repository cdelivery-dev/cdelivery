use macros::located_error;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum ErrorKind {
    #[error("failed to read channel key file {path}: {source}")]
    KeyFile {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("channel key must be exactly 32 bytes")]
    KeyFormat,
    #[error("config fetch failed: {0}")]
    Fetch(#[source] reqwest::Error),
    #[error("config API returned {status}")]
    ConfigApi { status: http::StatusCode },
    #[error("sealed snapshot rejected: {0}")]
    Unseal(#[source] cipher::Error),
    #[error("config response echoed a foreign nonce")]
    NonceMismatch,
    #[error("certificate chain contains no certificate")]
    EmptyCertChain,
    #[error("private key does not match the leaf certificate")]
    CertKeyMismatch,
    #[error("TLS stack error: {0}")]
    Tls(#[source] pingora::tls::error::ErrorStack),
    #[error("pingora server error: {0}")]
    Server(#[source] Box<pingora::Error>),
    #[error("rng failure: {0}")]
    Rng(#[source] getrandom::Error),
    #[error("I/O error: {0}")]
    Io(#[source] std::io::Error),
    #[error("failed to initialize logging: {0}")]
    Logging(#[source] logging::Error),
}

located_error! {
    Error => ErrorKind,
    from {
        reqwest::Error => Fetch,
        cipher::Error => Unseal,
        pingora::tls::error::ErrorStack => Tls,
        Box<pingora::Error> => Server,
        getrandom::Error => Rng,
        std::io::Error => Io,
        logging::Error => Logging,
    }
}
