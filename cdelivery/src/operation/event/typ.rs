use garde::Validate;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct Conditions {
    pub(crate) conditions: Vec<insight_wire::event::Condition>,
}

#[macros_derive::validated_schema]
#[derive(Debug, Deserialize, IntoParams, ToSchema, Validate)]
#[into_params(parameter_in = Query)]
pub(crate) struct EventQuery {
    /// Defaults to one hour before `end`; must be within 31 days before `end`.
    #[garde(skip)]
    pub(crate) start: Option<Timestamp>,
    /// Defaults to now.
    #[garde(skip)]
    pub(crate) end: Option<Timestamp>,
    #[garde(skip)]
    pub(crate) kind: Option<insight_wire::event::Kind>,
    /// Exact match on the event's object: an app or group name, node ID, domain, `app/job` for jobs, or platform service name.
    #[garde(inner(length(max = 255)))]
    pub(crate) subject: Option<String>,
    #[garde(skip)]
    #[serde(default)]
    pub(crate) skip: u32,
    /// Defaults to 100.
    #[garde(range(min = 1, max = 1000))]
    #[serde(default = "super::super::typ::default_query_limit")]
    pub(crate) limit: u16,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct EventRecords {
    /// Inclusive start of the requested window after applying defaults, not collection coverage.
    pub(crate) start: Timestamp,
    /// Exclusive end of the requested window after applying defaults, not collection coverage.
    pub(crate) end: Timestamp,
    pub(crate) records: Vec<insight_wire::event::Event>,
}
