use base64::{Engine as _, engine::general_purpose::URL_SAFE};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, AeadCore, Generate, Key, KeyInit, array::typenum::Unsigned},
};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{ErrorKind, Result};

const NONCE_LEN: usize = <XChaCha20Poly1305 as AeadCore>::NonceSize::USIZE;
const TAG_LEN: usize = <XChaCha20Poly1305 as AeadCore>::TagSize::USIZE;
const MIN_LEN: usize = NONCE_LEN + TAG_LEN;

pub struct Cipher {
    inner: XChaCha20Poly1305,
}

impl std::fmt::Debug for Cipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cipher").finish_non_exhaustive()
    }
}

impl Cipher {
    pub fn from_key(key: &[u8; 32]) -> Self {
        let key = Key::<XChaCha20Poly1305>::from(*key);
        Self {
            inner: XChaCha20Poly1305::new(&key),
        }
    }

    fn encrypt(&self, plaintext: &str) -> Result<String> {
        let nonce = XNonce::try_generate()?;
        let ciphertext = self.inner.encrypt(&nonce, plaintext.as_bytes())?;
        let mut buf = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        buf.extend_from_slice(nonce.as_slice());
        buf.extend_from_slice(&ciphertext);
        Ok(URL_SAFE.encode(buf))
    }

    fn decrypt(&self, encoded: &str) -> Result<Vec<u8>> {
        let decoded = URL_SAFE.decode(encoded)?;
        let Some((nonce_bytes, ciphertext)) = decoded
            .split_first_chunk::<NONCE_LEN>()
            .filter(|(_, ct)| ct.len() >= TAG_LEN)
        else {
            return Err(ErrorKind::CiphertextTooShort {
                len: decoded.len(),
                min: MIN_LEN,
            }
            .into());
        };
        let nonce = XNonce::from(*nonce_bytes);
        Ok(self.inner.decrypt(&nonce, ciphertext)?)
    }

    pub fn seal<T: Serialize>(&self, value: &T) -> Result<String> {
        self.encrypt(&serde_json::to_string(value)?)
    }

    pub fn open<T: DeserializeOwned>(&self, encoded: &str) -> Result<T> {
        Ok(serde_json::from_slice(&self.decrypt(encoded)?)?)
    }
}
