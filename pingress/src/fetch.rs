use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::error::{ErrorKind, Result};

pub(crate) struct Client {
    http: reqwest::Client,
    url: String,
    cipher: cipher::Cipher,
}

impl Client {
    pub(crate) fn new(http: reqwest::Client, endpoint: &str) -> Result<Self> {
        Ok(Self {
            http,
            url: format!("http://{endpoint}{}", pingress_wire::CONFIG_PATH),
            cipher: load_cipher()?,
        })
    }

    pub(crate) async fn fetch(
        &self,
        etag: Option<&str>,
    ) -> Result<Option<pingress_wire::Snapshot>> {
        let nonce = fresh_nonce()?;
        let mut request = self.http.get(&self.url).query(&[("nonce", nonce.as_str())]);
        if let Some(etag) = etag {
            request = request.query(&[("etag", etag)]);
        }
        let response = request.send().await?;
        match response.status() {
            reqwest::StatusCode::NO_CONTENT => Ok(None),
            reqwest::StatusCode::OK => {
                let body = response.text().await?;
                let envelope: pingress_wire::Envelope = self.cipher.open(&body)?;
                if envelope.nonce != nonce {
                    return Err(ErrorKind::NonceMismatch.into());
                }
                Ok(Some(envelope.snapshot))
            }
            status => Err(ErrorKind::ConfigApi {
                status: http::StatusCode::from_u16(status.as_u16())
                    .unwrap_or(http::StatusCode::INTERNAL_SERVER_ERROR),
            }
            .into()),
        }
    }
}

fn load_cipher() -> Result<cipher::Cipher> {
    let path = format!("/run/secrets/{}", pingress_wire::KEY_SECRET_NAME);
    let bytes = std::fs::read(&path).map_err(|source| ErrorKind::KeyFile { path, source })?;
    let key: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| ErrorKind::KeyFormat)?;
    Ok(cipher::Cipher::from_key(&key))
}

fn fresh_nonce() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
