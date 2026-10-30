use getset::{CopyGetters, Getters};
use insight_wire::ObjectStoreConfig;
use jiff::SignedDuration;

#[derive(Debug, Clone, PartialEq, Eq, Getters, CopyGetters, macros_derive::Ctor)]
pub(crate) struct Insight {
    #[getset(get = "pub(crate)")]
    storage: ObjectStoreConfig,
    #[getset(get_copy = "pub(crate)")]
    retention_days: u16,
    #[getset(get_copy = "pub(crate)")]
    flush_interval: SignedDuration,
    #[getset(get = "pub(crate)")]
    placement: super::node::Selector,
    #[getset(get = "pub(crate)")]
    image: super::image::Reference,
}

impl super::versioned::Aggregate for Insight {}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Status {
    Enabled,
    Disabled,
    Degraded,
}

#[derive(Debug, Getters, macros_derive::Ctor)]
pub(crate) struct Deployment {
    #[getset(get = "pub(crate)")]
    server: Option<Insight>,
    #[getset(get = "pub(crate)")]
    agent: Option<(super::image::Reference, u16)>,
}

impl Deployment {
    pub(crate) fn status(&self) -> Status {
        match (self.server.is_some(), self.agent.is_some()) {
            (true, true) => Status::Enabled,
            (false, false) => Status::Disabled,
            _ => Status::Degraded,
        }
    }
}
