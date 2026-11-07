use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use macros::located_error;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum ErrorKind {
    #[error("volume {0} already exists")]
    VolumeExists(String),
    #[error("volume is in use: {0}")]
    VolumeInUse(#[source] docker::Error),
    #[error("node identity is unavailable: {0}")]
    NodeId(#[source] std::env::VarError),
    #[error("I/O error: {0}")]
    Io(#[source] std::io::Error),
    #[error("volume key must be exactly 32 bytes: {0}")]
    KeyLength(#[source] std::array::TryFromSliceError),
    #[error("Docker error: {0}")]
    Docker(#[source] docker::Error),
    #[error("runtime error: {0}")]
    Runtime(#[source] logging::Error),
}

located_error! {
    Error => ErrorKind,
    from {
        std::env::VarError => NodeId,
        std::io::Error => Io,
        std::array::TryFromSliceError => KeyLength,
        docker::Error => Docker,
        logging::Error => Runtime,
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        if matches!(
            self.kind,
            ErrorKind::VolumeExists(_) | ErrorKind::VolumeInUse(_)
        ) {
            return (StatusCode::CONFLICT, self.to_string()).into_response();
        }
        tracing::error!(error = ?self, "volume request failed");
        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()).into_response()
    }
}
