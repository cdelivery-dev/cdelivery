use axum::extract::{FromRequest, FromRequestParts, Request};
use http::StatusCode;
use http::request::Parts;
use serde::de::DeserializeOwned;

use crate::error::{Error, ErrorKind, Result};

pub(crate) type Authorized<O> = crate::http::extract::Authorized<O, Error>;

pub(crate) struct Body<T>(pub(crate) T);

pub(crate) struct Path<T>(pub(crate) T);

pub(crate) struct Query<T>(pub(crate) T);

impl<S, T> FromRequestParts<S> for Path<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Send,
{
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self> {
        let axum::extract::Path(value) = axum::extract::Path::<T>::from_request_parts(parts, state)
            .await
            .map_err(|e| ErrorKind::InvalidBody(e.body_text()))?;
        Ok(Self(value))
    }
}

impl<S, T> FromRequestParts<S> for Query<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self> {
        let axum_extra::extract::Query(value) =
            axum_extra::extract::Query::<T>::from_request_parts(parts, state)
                .await
                .map_err(|e| ErrorKind::InvalidBody(e.body_text()))?;
        Ok(Self(value))
    }
}

impl<S, T> FromRequest<S> for Body<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = Error;

    async fn from_request(req: Request, state: &S) -> Result<Self> {
        let axum::Json(value) = axum::Json::<T>::from_request(req, state)
            .await
            .map_err(|e| json_rejection(&e))?;
        Ok(Self(value))
    }
}

fn json_rejection(rejection: &axum::extract::rejection::JsonRejection) -> Error {
    let status = rejection.status();
    let detail = rejection.body_text();
    match status {
        StatusCode::PAYLOAD_TOO_LARGE => ErrorKind::PayloadTooLarge.into(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE => {
            ErrorKind::UnsupportedMediaType("application/json").into()
        }
        StatusCode::UNPROCESSABLE_ENTITY => ErrorKind::UnprocessableBody(detail).into(),
        _ => ErrorKind::InvalidBody(detail).into(),
    }
}
