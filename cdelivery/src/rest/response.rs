use axum::response::{IntoResponse, Response};
use http::header::{CONTENT_DISPOSITION, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};

use crate::operation;

pub(crate) fn attachment<O>(artifact: operation::Artifact) -> crate::error::Result<Response>
where
    O: operation::Operation<Output = operation::Artifact>,
{
    let operation::Media::Artifact(mime) = O::MEDIA else {
        return Err(crate::error::ErrorKind::InvalidOperationOutput.into());
    };
    Ok((
        [
            (CONTENT_TYPE, mime.to_owned()),
            (
                CONTENT_DISPOSITION,
                format!(
                    "attachment; filename=\"{}\"",
                    safe_filename(&artifact.filename)
                ),
            ),
            (X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
        ],
        artifact.bytes,
    )
        .into_response())
}

fn safe_filename(value: &str) -> String {
    let filename = value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_') {
                char::from(byte)
            } else {
                '_'
            }
        })
        .collect::<String>();
    let filename = filename.trim_matches('.');
    if filename.is_empty() {
        "download".to_owned()
    } else {
        filename.to_owned()
    }
}
