use garde::Validate;
use jiff::{SignedDuration, Timestamp};
use pingress_wire::access::{Outcome, StatsRow, StatusClass, StatusRow};
use serde::{Deserialize, Serialize};
use strum::VariantArray;
use utoipa::{IntoParams, ToSchema};

use crate::concept;

const _: () = assert!(
    pingress_wire::access::PATH_MAX_BYTES == 2048 && insight_wire::metric::MAX_BUCKETS == 720
);

macro_rules! access_query {
    (full $name:ident { $($extra:tt)* }) => {
        access_query!(@declare $name {
            #[garde(inner(custom(super::super::validate::valid_host)))]
            pub(crate) unmatched_host: Option<String>,
            #[garde(inner(custom(super::super::validate::valid_name)))]
            pub(crate) app_name: Option<String>,
            #[garde(inner(length(min = 1, max = 64), ascii, alphanumeric))]
            pub(crate) service_id: Option<String>,
        } { $($extra)* });

        impl $name {
            pub(crate) fn into_filter(
                self,
            ) -> crate::error::Result<pingress_wire::access::Filter> {
                Ok(access_query!(@filter self {
                    unmatched_host: self
                        .unmatched_host
                        .as_deref()
                        .map(canonical_host)
                        .transpose()?,
                    app_name: self.app_name,
                    service_id: self.service_id,
                }))
            }
        }
    };
    (scoped $name:ident { $($extra:tt)* }) => {
        access_query!(@declare $name {} { $($extra)* });

        impl $name {
            pub(crate) fn into_filter(
                self,
            ) -> crate::error::Result<pingress_wire::access::Filter> {
                Ok(access_query!(@filter self {
                    unmatched_host: None,
                    app_name: None,
                    service_id: None,
                }))
            }
        }
    };
    (@declare $name:ident { $($scope:tt)* } { $($extra:tt)* }) => {
        #[macros_derive::validated_schema]
        #[derive(Debug, Default, Deserialize, IntoParams, ToSchema, Validate)]
        #[into_params(parameter_in = Query)]
        pub(crate) struct $name {
            /// Defaults to one hour before `end`; must be within 31 days before `end`.
            #[garde(skip)]
            pub(crate) start: Option<Timestamp>,
            /// Defaults to now.
            #[garde(skip)]
            pub(crate) end: Option<Timestamp>,
            #[garde(inner(custom(super::super::validate::valid_node_id)))]
            pub(crate) node_id: Option<String>,
            #[garde(inner(custom(super::super::validate::valid_domain)))]
            pub(crate) domain: Option<String>,
            #[garde(inner(ip))]
            pub(crate) client_ip: Option<String>,
            #[garde(skip)]
            #[param(inline)]
            pub(crate) method: Option<pingress_wire::access::Method>,
            #[garde(inner(length(min = 1, max = 2048)))]
            pub(crate) path: Option<String>,
            #[garde(inner(range(min = 100, max = 999)))]
            pub(crate) status: Option<u16>,
            #[garde(skip)]
            pub(crate) status_class: Option<StatusClass>,
            #[garde(skip)]
            pub(crate) outcome: Option<Outcome>,
            #[garde(inner(range(min = 1, max = 65535)))]
            pub(crate) target_port: Option<u16>,
            #[garde(inner(length(min = 1, max = 128), ascii))]
            pub(crate) error: Option<String>,
            #[garde(inner(length(min = 1, max = 2048)))]
            pub(crate) search_path: Option<String>,
            #[garde(inner(length(min = 1, max = 64)))]
            pub(crate) search_client_ip: Option<String>,
            $($scope)*
            $($extra)*
        }
    };
    (@filter $query:ident { $($scope_filter:tt)* }) => {
        pingress_wire::access::Filter {
            node_id: $query.node_id,
            domain: $query.domain.as_deref().map(canonical_domain).transpose()?,
            client_ip: $query.client_ip.as_deref().map(canonical_ip).transpose()?,
            method: $query.method,
            path: $query.path,
            status: $query.status,
            status_class: $query.status_class,
            outcome: $query.outcome,
            target_port: $query.target_port,
            error: $query.error,
            search_path: $query.search_path,
            search_client_ip: $query.search_client_ip,
            $($scope_filter)*
        }
    };
}

access_query!(full AccessQuery {
    #[garde(skip)]
    #[serde(default)]
    pub(crate) skip: u32,
    /// Defaults to 100.
    #[garde(range(min = 1, max = 1000))]
    #[serde(default = "super::super::typ::default_query_limit")]
    pub(crate) limit: u16,
});

access_query!(full AccessStatsQuery {
    #[garde(inner(range(min = 1, max = 720)))]
    pub(crate) max_buckets: Option<u16>,
    #[garde(skip)]
    #[serde(default)]
    pub(crate) group_by_status: bool,
});

access_query!(scoped AppAccessQuery {
    #[garde(skip)]
    #[serde(default)]
    pub(crate) skip: u32,
    /// Defaults to 100.
    #[garde(range(min = 1, max = 1000))]
    #[serde(default = "super::super::typ::default_query_limit")]
    pub(crate) limit: u16,
});

fn canonical_domain(value: &str) -> crate::error::Result<String> {
    Ok(value.parse::<concept::domain::DomainName>()?.to_string())
}

fn canonical_host(value: &str) -> crate::error::Result<String> {
    pingress_wire::access::Host::parse(value)
        .map(pingress_wire::access::Host::into_string)
        .ok_or_else(|| crate::error::ErrorKind::InvalidBody(format!("invalid host {value}")).into())
}

fn canonical_ip(value: &str) -> crate::error::Result<String> {
    value
        .parse::<std::net::IpAddr>()
        .map(|ip| ip.to_canonical().to_string())
        .map_err(|_| crate::error::ErrorKind::InvalidBody(format!("invalid ip {value}")).into())
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct AccessRecords {
    pub(crate) records: Vec<pingress_wire::access::Access>,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct AccessStats {
    pub(crate) start: Timestamp,
    pub(crate) end: Timestamp,
    pub(crate) step_secs: u32,
    pub(crate) buckets: Vec<AccessBucket>,
    pub(crate) by_status: Option<Vec<StatusSeries>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct AccessBucket {
    pub(crate) start: Timestamp,
    pub(crate) end: Timestamp,
    pub(crate) aggregate: AccessAggregate,
    pub(crate) status_classes: Vec<StatusClassCount>,
    outcomes: Vec<OutcomeCount>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct AccessAggregate {
    request_count: u64,
    pub(crate) qps: f64,
    pub(crate) request_bytes: u64,
    pub(crate) response_bytes: u64,
    pub(crate) duration_us: Distribution,
    pub(crate) upstream_ttfb_us: Distribution,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct Distribution {
    sample_count: u64,
    pub(crate) p50: Option<f64>,
    pub(crate) p95: Option<f64>,
    pub(crate) p99: Option<f64>,
    max: Option<u64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct StatusClassCount {
    pub(crate) status_class: StatusClass,
    count: u64,
    pub(crate) ratio: f64,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct OutcomeCount {
    outcome: Outcome,
    count: u64,
    ratio: f64,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct StatusSeries {
    pub(crate) status: Option<u16>,
    pub(crate) buckets: Vec<CountBucket>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct CountBucket {
    pub(crate) start: Timestamp,
    end: Timestamp,
    request_count: u64,
    pub(crate) qps: f64,
}

impl AccessStats {
    pub(crate) fn assemble(
        range: (Timestamp, Timestamp),
        step_secs: u32,
        mut rows: Vec<StatsRow>,
        status_rows: Option<Vec<StatusRow>>,
    ) -> Self {
        let step = SignedDuration::from_secs(i64::from(step_secs));
        rows.sort_unstable_by_key(|row| row.timestamp);
        Self {
            start: range.0,
            end: range.1,
            step_secs,
            buckets: rows
                .iter()
                .map(|row| AccessBucket::build(row, step, range.1))
                .collect(),
            by_status: status_rows.map(|rows| status_series(rows, step, range.1)),
        }
    }
}

impl AccessBucket {
    fn build(row: &StatsRow, step: SignedDuration, range_end: Timestamp) -> Self {
        let (end, width) = bucket_window(row.timestamp, step, range_end);
        let status_classes = tallies(
            StatusClass::VARIANTS,
            row,
            StatusClass::as_str,
            |status_class, count, ratio| StatusClassCount {
                status_class,
                count,
                ratio,
            },
        );
        let outcomes = tallies(
            Outcome::VARIANTS,
            row,
            Outcome::as_str,
            |outcome, count, ratio| OutcomeCount {
                outcome,
                count,
                ratio,
            },
        );
        Self {
            start: row.timestamp,
            end,
            aggregate: AccessAggregate {
                request_count: row.count,
                qps: round4(concept::pingress::rate(row.count, width)),
                request_bytes: row.request_bytes,
                response_bytes: row.response_bytes,
                duration_us: Distribution {
                    sample_count: row.count,
                    p50: row.duration_p50,
                    p95: row.duration_p95,
                    p99: row.duration_p99,
                    max: row.duration_max,
                },
                upstream_ttfb_us: Distribution {
                    sample_count: row.ttfb_count,
                    p50: row.ttfb_p50,
                    p95: row.ttfb_p95,
                    p99: row.ttfb_p99,
                    max: row.ttfb_max,
                },
            },
            status_classes,
            outcomes,
        }
    }
}

fn status_series(
    rows: Vec<StatusRow>,
    step: SignedDuration,
    range_end: Timestamp,
) -> Vec<StatusSeries> {
    let mut series: std::collections::BTreeMap<(bool, u16), Vec<CountBucket>> =
        std::collections::BTreeMap::new();
    for row in rows {
        let (end, width) = bucket_window(row.timestamp, step, range_end);
        series
            .entry((row.status.is_none(), row.status.unwrap_or_default()))
            .or_default()
            .push(CountBucket {
                start: row.timestamp,
                end,
                request_count: row.count,
                qps: round4(concept::pingress::rate(row.count, width)),
            });
    }
    series
        .into_iter()
        .map(|((no_status, code), mut buckets)| {
            buckets.sort_unstable_by_key(|bucket| bucket.start);
            StatusSeries {
                status: (!no_status).then_some(code),
                buckets,
            }
        })
        .collect()
}

fn bucket_window(
    start: Timestamp,
    step: SignedDuration,
    range_end: Timestamp,
) -> (Timestamp, SignedDuration) {
    let end = crate::config::expiry(start, step).min(range_end);
    (end, end.duration_since(start))
}

fn tallies<T: Copy, U>(
    variants: &[T],
    row: &StatsRow,
    key: fn(T) -> &'static str,
    build: fn(T, u64, f64) -> U,
) -> Vec<U> {
    variants
        .iter()
        .filter_map(|variant| {
            let count = row.count_of(key(*variant));
            (count > 0).then(|| build(*variant, count, ratio(count, row.count)))
        })
        .collect()
}

#[expect(clippy::cast_precision_loss)]
fn ratio(count: u64, total: u64) -> f64 {
    round4(count as f64 / total.max(1) as f64)
}

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

#[macros_derive::validated_schema]
#[derive(Debug, Deserialize, ToSchema, Validate)]
pub(crate) struct CreatePingressRequest {
    /// Must be the latest `pingress` version from `version_list`.
    #[garde(length(min = 1, max = 255))]
    pub(crate) version: String,
    #[garde(dive)]
    pub(crate) placement: super::super::node::typ::NodeSelector,
}

#[macros_derive::validated_schema(patch)]
#[derive(Debug, Deserialize, ToSchema, Validate)]
pub(crate) struct UpdatePingressRequest {
    /// Must be the latest `pingress` version from `version_list`.
    #[garde(inner(length(min = 1, max = 255)))]
    pub(crate) version: Option<String>,
    #[garde(dive)]
    pub(crate) placement: Option<super::super::node::typ::NodeSelector>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct Pingress {
    pub(crate) image: String,
    pub(crate) version: String,
    pub(crate) placement: super::super::node::typ::NodeSelector,
    pub(crate) instances: u64,
    pub(crate) acme_enabled: bool,
}

impl From<concept::pingress::Deployment> for Pingress {
    fn from(deployment: concept::pingress::Deployment) -> Self {
        Self {
            version: deployment.config().image().tag().clone(),
            image: deployment.config().image().to_string(),
            placement: deployment.config().placement().clone().into(),
            instances: deployment.instances(),
            acme_enabled: deployment.acme_enabled(),
        }
    }
}

macros::mirror_enum! {
    #[macros_derive::validated_schema]
    #[derive(Debug, Deserialize, ToSchema, Validate)]
    CaChoice into concept::acme::CA {
        LetsEncrypt,
        LetsEncryptStaging,
        Pebble {
            #[garde(length(min = 1, max = 2048))]
            url: String,
            #[garde(length(min = 1, max = 65536))]
            root_ca_pem: String,
        },
    }
}

#[macros_derive::validated_schema(patch)]
#[derive(Debug, Deserialize, ToSchema, Validate)]
pub(crate) struct EnableAcmeRequest {
    #[garde(dive)]
    pub(crate) ca: Option<CaChoice>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct AcmeAccount {
    pub(crate) uri: String,
    pub(crate) created: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) enum CertSource {
    Manual,
    Acme,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct CertEntry {
    pub(crate) domain: String,
    pub(crate) apps: Vec<String>,
    pub(crate) certificate: Option<Certificate>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct Certificate {
    pub(crate) source: CertSource,
    pub(crate) not_after: Timestamp,
    pub(crate) expired: bool,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct Certs {
    pub(crate) certs: Vec<CertEntry>,
}

impl CertEntry {
    pub(crate) fn new(
        domain: &concept::domain::DomainName,
        apps: Vec<String>,
        cert: Option<&concept::acme::Cert>,
        now: Timestamp,
    ) -> Self {
        Self {
            domain: domain.to_string(),
            apps,
            certificate: cert.map(|cert| Certificate {
                source: if cert.is_static() {
                    CertSource::Manual
                } else {
                    CertSource::Acme
                },
                not_after: *cert.pem().not_after(),
                expired: cert.expired(now),
            }),
        }
    }
}

#[macros_derive::validated_schema]
#[derive(Debug, Deserialize, ToSchema, Validate)]
pub(crate) struct UploadCertRequest {
    #[garde(length(min = 1, max = 65536))]
    pub(crate) cert_chain_pem: String,
    #[schema(write_only, value_type = String)]
    #[garde(custom(super::super::validate::key_pem_length))]
    pub(crate) private_key_pem: cipher::Secret,
}
