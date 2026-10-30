use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::concept;

#[macros_derive::validated_schema]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Validate)]
pub(crate) struct ResourceSpec {
    #[garde(inner(custom(super::super::super::validate::cpu_cores)))]
    pub(crate) cpus: Option<f64>,
    #[garde(dive)]
    pub(crate) memory: Option<super::super::super::typ::ByteSize>,
}

super::super::super::typ::mirror_struct! {
    ResourceSpec(spec: &ResourceSpec) <-> concept::app::ResourceSpec(spec: &concept::app::ResourceSpec) try {
        cpus: spec.cpus => spec.cpus(),
        memory: spec.memory.map(u64::from) => spec
            .memory_bytes()
            .map(super::super::super::typ::ByteSize::try_from)
            .transpose()?,
    }
}

#[macros_derive::validated_schema]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Validate)]
pub(crate) struct Resources {
    #[garde(dive)]
    pub(crate) limits: Option<ResourceSpec>,
    #[garde(dive)]
    pub(crate) reservations: Option<ResourceSpec>,
}

super::super::super::typ::mirror_struct! {
    Resources(resources: &Resources) <-> concept::app::Resources(resources: &concept::app::Resources) try {
        limits: resources.limits.as_ref().map(Into::into) => resources
            .limits()
            .as_ref()
            .map(ResourceSpec::try_from)
            .transpose()?,
        reservations: resources.reservations.as_ref().map(Into::into) => resources
            .reservations()
            .as_ref()
            .map(ResourceSpec::try_from)
            .transpose()?,
    }
}

#[macros_derive::validated_schema]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Validate)]
pub(crate) struct Placement {
    #[garde(inner(custom(super::super::super::validate::valid_name)))]
    pub(crate) label: Option<String>,
    #[garde(inner(range(min = 1)))]
    pub(crate) max_replicas_per_node: Option<u64>,
}

super::super::super::typ::mirror_struct! {
    Placement(p: &Placement) <-> concept::app::Placement(c: &concept::app::Placement) {
        label: p.label.clone() => c.label().clone(),
        max_replicas_per_node: p.max_replicas_per_node => c.max_replicas_per_node(),
    }
}

#[macros_derive::validated_schema]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Validate)]
pub(crate) struct RestartPolicy {
    #[garde(dive)]
    pub(crate) delay: super::super::super::typ::Duration,
}

super::super::super::typ::mirror_struct! {
    RestartPolicy(policy: &RestartPolicy) <-> concept::app::RestartPolicy(policy: concept::app::RestartPolicy) try {
        delay: policy.delay.into() => policy.delay().try_into()?,
    }
}

macros::mirror_enum! {
    #[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
    UpdateFailureAction both concept::app::UpdateFailureAction { Continue, Pause, Rollback }
}

macros::mirror_enum! {
    #[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
    UpdateOrder both concept::app::UpdateOrder { StopFirst, StartFirst }
}

#[macros_derive::validated_schema]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Validate)]
pub(crate) struct UpdateConfig {
    #[garde(inner(range(min = 1, max = 100)))]
    pub(crate) parallelism: Option<u64>,
    #[garde(skip)]
    pub(crate) failure_action: UpdateFailureAction,
    #[garde(dive)]
    pub(crate) monitor: super::super::super::typ::Duration,
    #[garde(range(max = 100))]
    pub(crate) max_failure_percent: u8,
    #[garde(skip)]
    pub(crate) order: UpdateOrder,
}

super::super::super::typ::mirror_struct! {
    UpdateConfig(config: &UpdateConfig) <-> concept::app::UpdateConfig(config: concept::app::UpdateConfig) try {
        parallelism: config.parallelism => config.parallelism(),
        failure_action: config.failure_action.into() => config.failure_action().into(),
        monitor: config.monitor.into() => config.monitor().try_into()?,
        max_failure_percent: config.max_failure_percent => config.max_failure_percent(),
        order: config.order.into() => config.order().into(),
    }
}
