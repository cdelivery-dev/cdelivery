use macros::located_error;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum ErrorKind {
    #[error("failed to decode base64")]
    Base64(#[source] base64::DecodeError),

    #[error("failed to generate random bytes")]
    Rng(#[source] getrandom::Error),

    #[error("ciphertext too short: {len} bytes, need at least {min}")]
    CiphertextTooShort { len: usize, min: usize },

    #[error("data encryption/decryption failed")]
    Aead(#[source] chacha20poly1305::aead::Error),

    #[error("failed to (de)serialize sealed value")]
    Json(#[source] serde_json::Error),
}

located_error! {
    Error => ErrorKind,
    from {
        base64::DecodeError => Base64,
        getrandom::Error => Rng,
        chacha20poly1305::aead::Error => Aead,
        serde_json::Error => Json,
    }
}
