use argon2::{Argon2, PasswordHasher, PasswordVerifier};

use crate::error::{ErrorKind, Result};

pub(crate) fn hash(password: &str) -> Result<String> {
    let mut salt_bytes = [0u8; 16];
    getrandom::fill(&mut salt_bytes).map_err(ErrorKind::Rng)?;
    Ok(Argon2::default()
        .hash_password_with_salt(password.as_bytes(), &salt_bytes)
        .map_err(ErrorKind::Argon2)?
        .to_string())
}

pub(crate) fn verify(candidate: &str, phc_hash: &str) -> Result<bool> {
    match Argon2::default().verify_password(candidate.as_bytes(), phc_hash) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::PasswordInvalid) => Ok(false),
        Err(e) => Err(ErrorKind::Argon2(e).into()),
    }
}
