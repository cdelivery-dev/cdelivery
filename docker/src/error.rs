use macros::located_error;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum ErrorKind {
    #[error("request to the Docker daemon failed: {0}")]
    Reqwest(#[source] reqwest::Error),

    #[error("failed to (de)serialize JSON: {0}")]
    Json(#[source] serde_json::Error),

    #[error("invalid API version: {0}")]
    Version(#[source] semver::Error),

    #[error("requested API version {requested} is below daemon minimum {minimum}")]
    VersionTooLow { requested: String, minimum: String },

    #[error("requested API version {requested} is above daemon maximum {maximum}")]
    VersionTooHigh { requested: String, maximum: String },

    #[error("Docker API error {code}: {msg}")]
    Docker { code: http::StatusCode, msg: String },

    #[error("Docker service {0} response is missing status")]
    MissingServiceStatus(String),

    #[error("data is too large: {size} bytes, maximum is {max}")]
    DataTooLarge { size: usize, max: usize },
}

located_error! {
    Error => ErrorKind,
    from {
        reqwest::Error => Reqwest,
        serde_json::Error => Json,
        semver::Error => Version,
    }
}

impl Error {
    pub(crate) fn as_docker(&self) -> Option<(http::StatusCode, &str)> {
        match &self.kind {
            ErrorKind::Docker { code, msg } => Some((*code, msg)),
            _ => None,
        }
    }

    pub fn is_version_conflict(&self) -> bool {
        self.as_docker()
            .is_some_and(|(_, msg)| msg.contains("out of sequence"))
    }

    pub fn is_conflict(&self) -> bool {
        self.as_docker()
            .is_some_and(|(code, _)| code == http::StatusCode::CONFLICT)
    }

    pub fn is_not_found(&self) -> bool {
        self.as_docker()
            .is_some_and(|(code, _)| code == http::StatusCode::NOT_FOUND)
    }

    pub fn is_in_use(&self) -> bool {
        self.as_docker()
            .is_some_and(|(_, msg)| msg.contains("is in use by"))
    }
}
