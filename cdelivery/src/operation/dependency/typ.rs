use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::concept;

macros::mirror_enum! {
    #[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
    Provider both concept::dependency::Provider { Neon, PlanetScale, Local }
}

#[macros_derive::validated_schema]
#[derive(Deserialize, ToSchema, IntoParams, Validate)]
#[into_params(parameter_in = Query)]
pub(crate) struct Query {
    #[garde(inner(custom(super::super::validate::valid_name)))]
    #[serde(default)]
    pub(crate) group: Option<String>,
}

#[macros_derive::validated_schema]
#[derive(Deserialize, ToSchema, Validate)]
#[schema(as = DependencyCreateRequest)]
pub(crate) struct CreateRequest {
    #[garde(custom(super::super::validate::valid_name))]
    pub(crate) name: String,
    #[garde(inner(length(chars, max = 256)))]
    #[serde(default)]
    pub(crate) description: Option<String>,
    #[garde(skip)]
    pub(crate) provider: Provider,
    #[garde(length(min = 1, max = 16_384))]
    pub(crate) dsn: String,
    /// Neon API key or `PlanetScale` service token; required for cloud providers and forbidden for Local.
    #[garde(inner(length(min = 1, max = 16_384)))]
    #[serde(default)]
    pub(crate) api_token: Option<String>,
    /// Required for Neon; forbidden for `PlanetScale` and Local.
    #[garde(inner(length(min = 1, max = 255)))]
    #[serde(default)]
    pub(crate) project_id: Option<String>,
    /// Required for Local and forbidden for cloud providers; the source app must use ZFS volumes and determines the dependency's group.
    #[garde(inner(custom(super::super::validate::valid_name)))]
    #[serde(default)]
    pub(crate) app: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct Dependency {
    pub(crate) name: String,
    pub(crate) id: String,
    pub(crate) group: Option<String>,
    pub(crate) provider: Provider,
    pub(crate) app: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) created_at: jiff::Timestamp,
}

impl From<concept::dependency::Dependency> for Dependency {
    fn from(dependency: concept::dependency::Dependency) -> Self {
        Self {
            name: dependency.reference().name().to_string(),
            id: dependency.reference().id().clone(),
            group: dependency.group().as_ref().map(ToString::to_string),
            provider: dependency.provider().into(),
            app: dependency.local_app().as_ref().map(ToString::to_string),
            description: dependency.description().clone(),
            created_at: dependency.created_at(),
        }
    }
}

#[derive(Serialize, ToSchema)]
pub(crate) struct Dependencies {
    pub(crate) dependencies: Vec<Dependency>,
}

#[macros_derive::validated_schema]
#[derive(Deserialize, ToSchema, IntoParams, Validate)]
#[into_params(parameter_in = Query)]
pub(crate) struct PlanetScaleSizesQuery {
    /// Match this `PlanetScale` dependency's cloud provider and CPU architecture; omission returns the public catalog.
    #[garde(inner(custom(super::super::validate::valid_name)))]
    #[serde(default)]
    pub(crate) dependency: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct PlanetScaleSize {
    pub(crate) name: String,
    /// Memory per node in bytes; null when variants have different memory capacities.
    pub(crate) memory_bytes: Option<u64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct PlanetScaleSizes {
    pub(crate) sizes: Vec<PlanetScaleSize>,
}

impl From<Vec<cdelivery_wire::postgres::PlanetScaleSku>> for PlanetScaleSizes {
    fn from(skus: Vec<cdelivery_wire::postgres::PlanetScaleSku>) -> Self {
        let mut groups = std::collections::BTreeMap::<String, (u64, Option<u64>)>::new();
        for sku in skus {
            groups
                .entry(sku.size)
                .and_modify(|(minimum, memory)| {
                    *minimum = (*minimum).min(sku.memory_bytes);
                    *memory = memory.filter(|value| *value == sku.memory_bytes);
                })
                .or_insert((sku.memory_bytes, Some(sku.memory_bytes)));
        }
        let mut groups: Vec<_> = groups.into_iter().collect();
        groups.sort_by(|(a, (a_memory, _)), (b, (b_memory, _))| {
            a_memory.cmp(b_memory).then_with(|| a.cmp(b))
        });
        Self {
            sizes: groups
                .into_iter()
                .map(|(name, (_, memory_bytes))| PlanetScaleSize { name, memory_bytes })
                .collect(),
        }
    }
}
