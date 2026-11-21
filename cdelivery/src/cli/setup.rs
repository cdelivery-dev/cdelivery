use std::io::{Read as _, Write as _};
use std::process::ExitCode;

use crate::config;
use crate::error::{ErrorKind, Result};

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    first_name: String,
    last_name: String,
    email: String,
    password: cipher::Secret,
}

pub(super) async fn run() -> Result<ExitCode> {
    let mut body = zeroize::Zeroizing::new(String::new());
    std::io::stdin()
        .take(config::PUBLIC_BODY_LIMIT as u64 + 1)
        .read_to_string(&mut body)?;
    if body.len() > config::PUBLIC_BODY_LIMIT {
        return Err(ErrorKind::PayloadTooLarge.into());
    }
    let input: Input = serde_json::from_str(&body)
        .map_err(|_| ErrorKind::InvalidBody("Invalid setup JSON.".into()))?;
    let password = input.password.reveal();
    if password.chars().count() < 8
        || !password.bytes().any(|b| b.is_ascii_lowercase())
        || !password.bytes().any(|b| b.is_ascii_uppercase())
        || !password.bytes().any(|b| b.is_ascii_digit())
        || password.contains(['\r', '\n', '\u{2028}', '\u{2029}'])
    {
        return Err(ErrorKind::InvalidBody(
            "Password requires at least 8 characters, uppercase and lowercase letters, a digit, and no line breaks.".into(),
        ).into());
    }
    let payload = serde_json::json!({
        "first_name": input.first_name,
        "last_name": input.last_name,
        "email": input.email,
        "password_sha256": super::sha256_hex(password),
    });
    let result = submit(payload).await;
    if result.is_err() {
        writeln!(
            std::io::stderr(),
            "Setup result is unconfirmed; changes may already be saved."
        )?;
    }
    result
}

async fn submit(payload: serde_json::Value) -> Result<ExitCode> {
    let mut response = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .timeout(config::SETUP_TIMEOUT.unsigned_abs())
        .build()?
        .post("http://127.0.0.1:9000/setup")
        .header(http::header::ORIGIN, "http://127.0.0.1:9000")
        .header(http::header::ACCEPT, "application/json")
        .form(&[("payload", payload.to_string())])
        .send()
        .await?;
    let status = response.status();
    let success = status == http::StatusCode::ACCEPTED;
    let media = if success {
        "application/json"
    } else {
        "application/problem+json"
    };
    if !(success || status.is_client_error() || status.is_server_error())
        || response
            .headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .map(str::trim)
            != Some(media)
    {
        return Err(ErrorKind::InvalidBody("Unexpected setup response.".into()).into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if chunk.len() > config::PUBLIC_BODY_LIMIT.saturating_sub(body.len()) {
            return Err(ErrorKind::PayloadTooLarge.into());
        }
        body.extend_from_slice(&chunk);
    }
    if success {
        output(std::io::stdout().lock(), &body, ExitCode::SUCCESS)
    } else {
        output(std::io::stderr().lock(), &body, ExitCode::FAILURE)
    }
}

fn output(mut writer: impl std::io::Write, body: &[u8], code: ExitCode) -> Result<ExitCode> {
    writer.write_all(body)?;
    writeln!(writer)?;
    writer.flush()?;
    Ok(code)
}
