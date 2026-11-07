use jiff::Timestamp;
use totp_rs::{Builder, Secret};
use zeroize::Zeroizing;

use crate::error::Result;

pub(crate) fn enrollment(account: &str) -> Result<(String, String)> {
    let mut secret = Zeroizing::new([0; 20]);
    getrandom::fill(&mut *secret)?;
    let totp = Builder::new()
        .with_step_duration(crate::config::TOTP_STEP_SECS)
        .with_secret(*secret)
        .with_account_name(account.replace(':', "_"))
        .with_issuer(Some("CDelivery"))
        .build()?;
    Ok((totp.secret().to_base32(), totp.to_url()?))
}

pub(crate) fn verify(secret: &Secret, code: &str, now: Timestamp) -> Result<Option<u64>> {
    let Ok(seconds) = u64::try_from(now.as_second()) else {
        return Ok(None);
    };
    Ok(Builder::new()
        .with_step_duration(crate::config::TOTP_STEP_SECS)
        .with_secret(secret.clone())
        .build()?
        .check(code, seconds))
}
