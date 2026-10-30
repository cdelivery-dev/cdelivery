use getset::{CopyGetters, Getters};
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};

use crate::error::{ErrorKind, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Method {
    Hurl {
        image: super::image::Reference,
        config: super::app::XConfigRef,
    },
    Custom {
        command: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Target {
    Domain(super::domain::DomainName),
    PublishedPort { published: std::num::NonZeroU16 },
    InternalPort(std::num::NonZeroU16),
}

impl Target {
    fn domain_port(domain: &super::domain::DomainName, app: &super::app::App) -> Result<u16> {
        let mut targets = app
            .domains()
            .iter()
            .filter(|rule| rule.domain().as_str() == domain.as_str())
            .map(|rule| rule.route().target.get());
        let target = targets.next().ok_or(ErrorKind::XHealthTargetNotFound)?;
        if targets.any(|candidate| candidate != target) {
            return Err(ErrorKind::XHealthTargetAmbiguous.into());
        }
        Ok(target)
    }

    fn published_port(
        published: std::num::NonZeroU16,
        app: &super::app::App,
        hurl: bool,
    ) -> Result<u16> {
        let mut targets = app
            .ports()
            .iter()
            .filter(|port| {
                port.published() == published.get()
                    && port.target() != 0
                    && (!hurl || port.protocol() == super::app::PortProtocol::Tcp)
            })
            .map(super::app::Port::target);
        let target = targets.next().ok_or(ErrorKind::XHealthTargetNotFound)?;
        if targets.any(|candidate| candidate != target) {
            return Err(ErrorKind::XHealthTargetAmbiguous.into());
        }
        Ok(target)
    }

    pub(crate) fn resolve(&self, app: &super::app::App, hurl: bool) -> Result<u16> {
        match self {
            Self::Domain(domain) => Self::domain_port(domain, app),
            Self::PublishedPort { published } => Self::published_port(*published, app, hurl),
            Self::InternalPort(port) => Ok(port.get()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters)]
pub(crate) struct Spec {
    #[getset(get = "pub(crate)")]
    method: Method,
    #[getset(get_copy = "pub(crate)")]
    interval: SignedDuration,
    #[getset(get = "pub(crate)")]
    target: Target,
    #[getset(get_copy = "pub(crate)")]
    guardian: bool,
}

impl Spec {
    pub(crate) fn new(
        method: Method,
        interval: SignedDuration,
        target: Target,
        guardian: bool,
    ) -> Result<Self> {
        Self::validate_interval(interval)?;
        let spec = Self {
            method,
            interval: Self::whole_seconds(interval),
            target,
            guardian,
        };
        Ok(spec)
    }

    pub(crate) fn target_port(&self, app: &super::app::App) -> Result<u16> {
        self.target
            .resolve(app, matches!(self.method, Method::Hurl { .. }))
    }

    pub(crate) fn validate_interval(interval: SignedDuration) -> Result<()> {
        if Self::whole_seconds(interval) < crate::config::XHEALTH_MIN_INTERVAL {
            return Err(ErrorKind::XHealthIntervalInvalid.into());
        }
        Ok(())
    }

    fn whole_seconds(interval: SignedDuration) -> SignedDuration {
        SignedDuration::from_secs(interval.as_secs())
    }
}

impl super::versioned::Aggregate for Spec {}

#[derive(Debug, Getters, macros_derive::Ctor)]
pub(crate) struct XHealth {
    #[getset(get = "pub(crate)")]
    spec: Spec,
    #[getset(get = "pub(crate)")]
    state: super::run::State,
    round_started_at: Timestamp,
}

impl XHealth {
    pub(crate) fn due(&self, now: Timestamp) -> bool {
        self.state.finished()
            && crate::config::expiry(self.round_started_at, self.spec.interval()) < now
    }
}
