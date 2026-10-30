use garde::Validate;
use jiff::Timestamp;
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

const _: () = assert!(insight_wire::metric::MAX_BUCKETS == 720);

#[macros_derive::validated_schema]
#[derive(Debug, Deserialize, IntoParams, ToSchema, Validate)]
#[into_params(parameter_in = Query)]
pub(crate) struct Query {
    /// Defaults to one hour before `end`; must be within 31 days before `end`.
    #[garde(skip)]
    pub(crate) start: Option<Timestamp>,
    /// Defaults to now.
    #[garde(skip)]
    pub(crate) end: Option<Timestamp>,
    #[garde(inner(length(min = 1, max = 64), ascii, alphanumeric))]
    pub(crate) instance: Option<String>,
    #[garde(inner(range(min = 1, max = 720)))]
    pub(crate) max_buckets: Option<u16>,
}

#[macros_derive::validated_schema]
#[derive(Debug, Deserialize, IntoParams, ToSchema, Validate)]
#[into_params(parameter_in = Query)]
pub(crate) struct RangeQuery {
    /// Defaults to one hour before `end`; must be within 31 days before `end`.
    #[garde(skip)]
    pub(crate) start: Option<Timestamp>,
    /// Defaults to now.
    #[garde(skip)]
    pub(crate) end: Option<Timestamp>,
    #[garde(inner(range(min = 1, max = 720)))]
    pub(crate) max_buckets: Option<u16>,
}

#[derive(Debug, serde::Serialize, ToSchema)]
pub(crate) struct ContainerSeries {
    pub(crate) step_secs: u32,
    pub(crate) instances: Vec<insight_wire::metric::ContainerInstance>,
    pub(crate) points: Vec<insight_wire::metric::ContainerBucket>,
}
