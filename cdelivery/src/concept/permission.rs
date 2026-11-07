use getset::{CopyGetters, Getters};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Requirement {
    Shared,
    Own,
    Operator,
    Admin,
}

#[derive(Debug, Clone, Getters, CopyGetters, macros_derive::Ctor)]
pub(crate) struct Access {
    #[getset(get = "pub(crate)")]
    actor: super::actor::Actor,
    #[getset(get_copy = "pub(crate)")]
    role: super::user::Role,
}

impl Access {
    pub(crate) fn user_id(&self) -> crate::error::Result<uuid::Uuid> {
        self.actor
            .user_id()
            .ok_or_else(|| crate::error::ErrorKind::Forbidden.into())
    }

    pub(crate) fn allows(&self, requirement: Requirement) -> bool {
        match requirement {
            Requirement::Shared => true,
            Requirement::Own => self.actor.user_id().is_some(),
            Requirement::Operator => self.role >= super::user::Role::Operator,
            Requirement::Admin => self.role >= super::user::Role::Admin,
        }
    }
}
