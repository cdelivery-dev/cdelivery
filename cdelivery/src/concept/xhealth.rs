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
    fn unique_port(mut targets: impl Iterator<Item = u16>) -> Result<u16> {
        let target = targets.next().ok_or(ErrorKind::XHealthTargetNotFound)?;
        if targets.any(|candidate| candidate != target) {
            return Err(ErrorKind::XHealthTargetAmbiguous.into());
        }
        Ok(target)
    }

    pub(crate) fn internal_port(&self, app: &super::app::App, hurl: bool) -> Result<u16> {
        match self {
            Self::Domain(domain) => Self::unique_port(
                app.domains()
                    .iter()
                    .filter(|rule| rule.domain().as_str() == domain.as_str())
                    .map(|rule| rule.route().target.get()),
            ),
            Self::PublishedPort { published } => Self::unique_port(
                app.ports()
                    .iter()
                    .filter(|port| {
                        port.published() == published.get()
                            && port.target() != 0
                            && (!hurl || port.protocol() == super::app::PortProtocol::Tcp)
                    })
                    .map(super::app::Port::target),
            ),
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

    pub(crate) fn internal_port(&self, app: &super::app::App) -> Result<u16> {
        self.target
            .internal_port(app, matches!(self.method, Method::Hurl { .. }))
    }

    pub(crate) fn reserved_env(&self) -> &'static [&'static str] {
        match (&self.method, &self.target) {
            (Method::Hurl { .. }, _) => &[crate::config::HURL_TARGET_URL_ENV],
            (Method::Custom { .. }, Target::Domain(_)) => &[
                crate::config::TARGET_HOST_ENV,
                crate::config::TARGET_PORT_ENV,
                crate::config::TARGET_SCHEME_ENV,
            ],
            (Method::Custom { .. }, _) => &[
                crate::config::TARGET_HOST_ENV,
                crate::config::TARGET_PORT_ENV,
            ],
        }
    }

    pub(crate) fn validate_app(&self, app: &super::app::App) -> Result<()> {
        self.internal_port(app)?;
        let reserved = self.reserved_env();
        if app
            .text_secrets()
            .map(|(name, _)| name)
            .chain(app.dependency_env().map(|(name, _)| name))
            .any(|name| reserved.contains(&name))
        {
            return Err(ErrorKind::SecretBindingInvalid.into());
        }
        if matches!(self.method, Method::Hurl { .. })
            && app.xconfigs().iter().any(|file| {
                let target = std::path::Path::new(file.target());
                let script = std::path::Path::new(crate::config::HURL_FILE_PATH);
                target.starts_with(script) || script.starts_with(target)
            })
        {
            return Err(ErrorKind::SecretBindingInvalid.into());
        }
        Ok(())
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
