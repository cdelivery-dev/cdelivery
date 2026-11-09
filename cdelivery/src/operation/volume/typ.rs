use crate::concept;

macros::mirror_enum! {
    #[derive(Debug, serde::Serialize, utoipa::ToSchema)]
    Backend both cdelivery_wire::Backend { Local, Zfs }
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub(crate) struct Settings {
    #[schema(value_type = Option<String>)]
    pub(crate) image: Option<concept::image::Reference>,
    pub(crate) zfs_enabled: Option<bool>,
}

#[macros_derive::validated_schema(patch)]
#[derive(Debug, serde::Deserialize, utoipa::ToSchema, garde::Validate)]
#[schema(as = VolumeChanges)]
pub(crate) struct Changes {
    /// Must be the latest `cdelivery_agent` version from `version_list.components`.
    #[garde(inner(length(min = 1, max = 255)))]
    pub(crate) version: Option<String>,
    /// Required when enabling Volumes; change mode by disabling and enabling again.
    #[garde(skip)]
    pub(crate) zfs_enabled: Option<bool>,
}

super::super::input::composite_input!(Target {
    #[garde(custom(super::super::validate::valid_node_id))]
    id: String,
    #[garde(custom(super::super::validate::valid_name))]
    #[garde(custom(super::super::validate::available_volume_name))]
    name: String,
});

super::super::input::composite_input!(GuardianTarget {
    #[garde(custom(super::super::validate::valid_node_id))]
    id: String,
    #[garde(custom(super::super::validate::valid_name))]
    name: String,
});

#[macros_derive::validated_schema]
#[derive(Debug, serde::Deserialize, utoipa::ToSchema, utoipa::IntoParams, garde::Validate)]
#[into_params(parameter_in = Query)]
pub(crate) struct Query {
    #[serde(default)]
    #[garde(skip)]
    pub(crate) sizes: bool,
}

#[macros_derive::validated_schema]
#[derive(Debug, serde::Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct CreateVolumeRequest {
    #[garde(custom(super::super::validate::valid_name))]
    #[garde(custom(super::super::validate::available_volume_name))]
    pub(crate) name: String,
    #[garde(inner(length(max = 256)))]
    #[serde(default)]
    pub(crate) description: Option<String>,
}

#[macros_derive::validated_schema(patch)]
#[derive(Debug, serde::Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct UpdateVolumeRequest {
    #[garde(inner(inner(length(max = 256))))]
    pub(crate) description: Option<Option<String>>,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
#[schema(as = NodeVolume)]
pub(crate) struct Volume {
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    pub(crate) size_bytes: Option<u64>,
    pub(crate) backend: Option<Backend>,
    pub(crate) ready: bool,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
#[schema(as = NodeVolumes)]
pub(crate) struct Report {
    pub(crate) volumes: Vec<Volume>,
    pub(crate) guardians: std::collections::BTreeSet<String>,
    pub(crate) zfs_enabled: bool,
    pub(crate) total_bytes: u64,
    pub(crate) available_bytes: u64,
}
