use serde_with::{DeserializeFromStr, SerializeDisplay};

use crate::error::{Error, ErrorKind, Result};

const MAX_DOMAIN_LEN: usize = 253;
const MAX_LABEL_LEN: usize = 63;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, SerializeDisplay, DeserializeFromStr)]
pub(crate) struct DomainName(String);

impl std::str::FromStr for DomainName {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self> {
        let unrooted = input.strip_suffix('.').unwrap_or(input);
        let normalized = unrooted.to_ascii_lowercase();
        let err = || Error::from(ErrorKind::InvalidDomain(input.to_owned()));
        if normalized.is_empty() || normalized.len() > MAX_DOMAIN_LEN {
            return Err(err());
        }
        let labels: Vec<&str> = normalized.split('.').collect();
        if labels.len() < 2 {
            return Err(err());
        }
        for label in &labels {
            let charset_ok = label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
            if label.is_empty()
                || label.len() > MAX_LABEL_LEN
                || !charset_ok
                || label.starts_with('-')
                || label.ends_with('-')
            {
                return Err(err());
            }
        }
        if labels.last().is_some_and(|tail| label_is_number(tail)) {
            return Err(err());
        }
        Ok(Self(normalized))
    }
}

fn label_is_number(label: &str) -> bool {
    label.bytes().all(|b| b.is_ascii_digit())
        || label
            .strip_prefix("0x")
            .is_some_and(|hex| !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit()))
}

impl DomainName {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DomainName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "pingress_wire::Route", into = "pingress_wire::Route")]
pub(crate) struct Rule(pingress_wire::Route);

impl Rule {
    pub(crate) fn domain(&self) -> DomainName {
        DomainName(self.0.domain.clone())
    }

    pub(crate) fn https(&self) -> bool {
        self.0.https
    }

    pub(crate) fn route(&self) -> &pingress_wire::Route {
        &self.0
    }
}

impl TryFrom<pingress_wire::Route> for Rule {
    type Error = Error;

    fn try_from(mut route: pingress_wire::Route) -> Result<Self> {
        route.domain = route.domain.parse::<DomainName>()?.0;
        Ok(Self(route))
    }
}

impl From<Rule> for pingress_wire::Route {
    fn from(rule: Rule) -> Self {
        rule.0
    }
}
