use std::sync::LazyLock;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jiff::Timestamp;

use crate::concept;
use crate::error::{ErrorKind, Result};

const PREFIX: &str = "cdj_";
static INVITATIONS: LazyLock<scc::HashMap<String, concept::node::join::Invitation>> =
    LazyLock::new(scc::HashMap::new);

pub(crate) async fn issue(
    invitation: &concept::node::join::Invitation,
    now: Timestamp,
) -> Result<String> {
    INVITATIONS
        .retain_async(|_, invitation| !invitation.expired(now))
        .await;
    loop {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes)?;
        let token = format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes));
        if INVITATIONS
            .insert_async(token.clone(), invitation.clone())
            .await
            .is_ok()
        {
            return Ok(token);
        }
    }
}

pub(crate) async fn verify(token: &str, now: Timestamp) -> Result<concept::node::join::Invitation> {
    INVITATIONS
        .read_async(token, |_, invitation| {
            (!invitation.expired(now)).then(|| invitation.clone())
        })
        .await
        .flatten()
        .ok_or_else(|| ErrorKind::TokenInvalid.into())
}
