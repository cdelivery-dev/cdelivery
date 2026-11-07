use garde::Validate;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::{concept, infra};

macro_rules! query {
    ($name:ident { $($extra:tt)* }) => {
        #[macros_derive::validated_schema]
        #[derive(Debug, Deserialize, IntoParams, ToSchema, Validate)]
        #[into_params(parameter_in = Query)]
        pub(crate) struct $name {
            /// Defaults to one hour before `end`; must be within 31 days before `end`.
            #[garde(skip)]
            pub(crate) start: Option<Timestamp>,
            /// Defaults to now.
            #[garde(skip)]
            pub(crate) end: Option<Timestamp>,
            #[garde(length(max = 32), inner(length(min = 1, max = 64)))]
            #[serde(default)]
            pub(crate) words: Vec<String>,
            #[garde(inner(length(min = 1, max = 16)))]
            pub(crate) source: Option<String>,
            #[garde(inner(custom(super::super::validate::valid_node_id)))]
            pub(crate) node_id: Option<String>,
            #[garde(inner(length(min = 1, max = 64), ascii, alphanumeric))]
            pub(crate) container_id: Option<String>,
            #[garde(skip)]
            #[serde(default)]
            pub(crate) skip: u32,
            /// Defaults to 100.
            #[garde(range(min = 1, max = 1000))]
            #[serde(default = "super::default_query_limit")]
            pub(crate) limit: u16,
            $($extra)*
        }

        impl $name {
            pub(crate) fn into_params(self, now: Timestamp) -> infra::insight::query::LogParams {
                infra::insight::query::LogParams::new(
                    super::super::validate::query_range(self.start, self.end, now),
                    self.skip,
                    self.limit,
                    self.words,
                    self.source,
                    self.node_id,
                    self.container_id,
                )
            }
        }
    };
}

query!(Query {});

query!(AppQuery {
    /// Defaults to App.
    #[garde(skip)]
    #[serde(default)]
    #[param(inline)]
    pub(crate) target: AppTarget,
    /// Required for `XJob`; forbidden for other targets.
    #[garde(inner(custom(super::super::validate::valid_name)))]
    pub(crate) job: Option<String>,
});

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) enum AppTarget {
    #[default]
    App,
    XHealth,
    XJob,
    GuardianApp,
    GuardianCheck,
}

macros::mirror_enum! {
    #[derive(Debug, Serialize, ToSchema)]
    #[serde(tag = "target")]
    Source from concept::app::LogSource {
        App,
        XHealth,
        XJob {
            #[schema(value_type = String)]
            job: concept::name::Name,
        },
        GuardianApp,
        GuardianCheck,
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct Sources {
    pub(crate) sources: Vec<Source>,
}

#[derive(Debug, Serialize, ToSchema, macros_derive::Ctor)]
pub(crate) struct Records {
    pub(crate) service_id: String,
    pub(crate) records: Vec<insight_wire::log::Log>,
}
