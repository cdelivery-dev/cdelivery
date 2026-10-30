use getset::{CopyGetters, Getters};
use jiff::Timestamp;

use crate::error::{ErrorKind, Result};

#[derive(Debug, Clone, Getters, CopyGetters, macros_derive::Ctor)]
pub(crate) struct Invitation {
    #[getset(get_copy = "pub(crate)")]
    role: super::Role,
    expires_at: Timestamp,
    #[getset(get = "pub(crate)")]
    issuer: super::super::actor::Actor,
}

impl Invitation {
    pub(crate) fn user_role(
        &self,
        user: &super::super::user::User,
        now: Timestamp,
    ) -> Result<super::super::user::Role> {
        let super::super::actor::Subject::User { id, api_key, .. } = self.issuer.subject() else {
            return Err(ErrorKind::TokenInvalid.into());
        };
        if *id != user.id() {
            return Err(ErrorKind::TokenInvalid.into());
        }
        let api_key = api_key
            .map(|id| {
                user.api_keys()
                    .iter()
                    .find(|key| {
                        key.id() == id && key.expires_at().is_none_or(|expiry| expiry > now)
                    })
                    .ok_or(ErrorKind::TokenInvalid)
            })
            .transpose()?;
        Ok(user.effective_role(api_key))
    }

    pub(crate) fn expired(&self, now: Timestamp) -> bool {
        self.expires_at <= now
    }
}
