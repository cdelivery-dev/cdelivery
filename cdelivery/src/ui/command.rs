use axum::extract::{FromRequest, Request};
use http::StatusCode;
use serde::de::DeserializeOwned;

use crate::error::ErrorKind;

pub(super) struct Payload<T>(pub(super) T);

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    payload: String,
}

impl<S, T> FromRequest<S> for Payload<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = super::response::Error;

    async fn from_request(req: Request, state: &S) -> std::result::Result<Self, Self::Rejection> {
        let axum::Form(envelope) = axum::Form::<Envelope>::from_request(req, state)
            .await
            .map_err(|rejection| form_rejection(&rejection))?;
        serde_json::from_str(&envelope.payload)
            .map(Self)
            .map_err(|error| ErrorKind::InvalidBody(error.to_string()).into())
    }
}

fn form_rejection(rejection: &axum::extract::rejection::FormRejection) -> super::response::Error {
    match rejection.status() {
        StatusCode::PAYLOAD_TOO_LARGE => ErrorKind::PayloadTooLarge.into(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE => {
            ErrorKind::UnsupportedMediaType("application/x-www-form-urlencoded").into()
        }
        _ => ErrorKind::InvalidBody(rejection.body_text()).into(),
    }
}
