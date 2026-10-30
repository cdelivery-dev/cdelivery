use cipher::{Domain, MasterKey};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::{ErrorKind, Result};

#[derive(Serialize, Deserialize)]
struct Claims {
    sub: Uuid,
    exp: Timestamp,
    password_fingerprint: [u8; 32],
}

pub(crate) fn issue(
    key: &MasterKey,
    user_id: Uuid,
    expires_at: Timestamp,
    password_hash: &str,
) -> Result<String> {
    Ok(key.cipher_for(Domain::Session).seal(&Claims {
        sub: user_id,
        exp: expires_at,
        password_fingerprint: fingerprint(password_hash),
    })?)
}

pub(crate) fn verify(
    key: &MasterKey,
    token: &str,
    now: Timestamp,
) -> Result<(Uuid, Timestamp, [u8; 32])> {
    let claims: Claims = key
        .cipher_for(Domain::Session)
        .open(token)
        .map_err(|_| ErrorKind::TokenInvalid)?;
    if claims.exp <= now {
        return Err(ErrorKind::TokenInvalid.into());
    }
    Ok((claims.sub, claims.exp, claims.password_fingerprint))
}

pub(crate) fn matches_password(expected: &[u8; 32], password_hash: &str) -> bool {
    expected == &fingerprint(password_hash)
}

fn fingerprint(password_hash: &str) -> [u8; 32] {
    Sha256::digest(password_hash).into()
}
