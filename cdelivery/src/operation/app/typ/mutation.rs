use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::concept;
use crate::error::{Error, ErrorKind, Result};

#[macros_derive::validated_schema]
#[derive(Debug, Serialize, Deserialize, ToSchema, Validate)]
pub(crate) struct CreateAppRequest {
    #[garde(custom(super::super::super::validate::valid_name))]
    pub(crate) name: String,
    #[garde(length(max = 500))]
    #[serde(default)]
    pub(crate) description: Option<String>,
    /// An existing group.
    #[garde(custom(super::super::super::validate::valid_name))]
    pub(crate) group: String,
    /// Nodes pull it during deployment.
    #[garde(custom(super::super::super::validate::image_reference))]
    pub(crate) image: String,
    #[garde(dive)]
    #[serde(default)]
    pub(crate) image_auth: Option<super::image::ImageAuth>,
    /// Dynamic replicas require Insight collection.
    #[garde(dive)]
    pub(crate) replicas: super::replicas::Replicas,
    #[garde(dive)]
    #[serde(default)]
    pub(crate) ports: Vec<super::network::Port>,
    /// Requires Pingress.
    #[garde(dive)]
    #[serde(default)]
    pub(crate) domains: Vec<DomainSpec>,
    #[garde(dive)]
    #[serde(default)]
    pub(crate) xconfigs: Vec<super::xconfig::XConfigUpload>,
    /// Makes the app stateful on one node: one replica, no xconfigs or placement, no guarded updates. Cannot be added later.
    #[garde(dive)]
    #[serde(default)]
    pub(crate) mounts: Option<super::container::NodeMounts>,
    #[garde(dive)]
    #[serde(default)]
    pub(crate) container_options: Option<super::container::ContainerOptions>,
    #[garde(dive)]
    #[serde(default)]
    pub(crate) resources: Option<super::deployment::Resources>,
    #[garde(dive)]
    #[serde(default)]
    pub(crate) placement: Option<super::deployment::Placement>,
    /// Defaults to `FollowHost`, the node's Docker default. The `Insight` driver needs Insight enabled; `app_logs` returns logs only for apps using it.
    #[garde(dive)]
    #[serde(default)]
    pub(crate) log_driver: Option<super::container::LogDriver>,
    #[garde(dive)]
    #[serde(default)]
    pub(crate) restart_policy: Option<super::deployment::RestartPolicy>,
    #[garde(dive)]
    #[serde(default)]
    pub(crate) update_config: Option<super::deployment::UpdateConfig>,
}

impl CreateAppRequest {
    pub(crate) fn set_command(&mut self, command: Vec<String>) {
        match self.container_options.as_mut() {
            Some(options) => options.set_command(command),
            None if !command.is_empty() => {
                self.container_options =
                    Some(super::container::ContainerOptions::from_command(command));
            }
            None => {}
        }
    }
}

impl TryFrom<&CreateAppRequest> for concept::app::Desired {
    type Error = Error;

    fn try_from(dto: &CreateAppRequest) -> Result<Self> {
        let (replicas, scaling) = match (&dto.replicas).try_into()? {
            concept::app::Replicas::Fixed(count) => (count, None),
            concept::app::Replicas::Dynamic(scaling) => (scaling.min(), Some(scaling)),
        };
        Ok(Self {
            name: dto.name.parse()?,
            description: dto.description.clone(),
            group: dto.group.parse()?,
            image: dto.image.parse()?,
            image_auth: dto
                .image_auth
                .as_ref()
                .map(concept::image::ImageAuth::try_from)
                .transpose()?,
            replicas,
            scaling,
            ports: dto.ports.iter().map(|p| (*p).into()).collect(),
            domains: dto
                .domains
                .iter()
                .map(TryInto::try_into)
                .collect::<Result<_>>()?,
            xconfigs: Vec::new(),
            mounts: dto.mounts.as_ref().map(Into::into),
            container_options: dto.container_options.as_ref().map(Into::into),
            resources: dto.resources.as_ref().map(Into::into),
            placement: dto.placement.as_ref().map(Into::into),
            log_driver: dto
                .log_driver
                .as_ref()
                .map_or_else(concept::app::LogDriver::default, Into::into),
            restart_policy: dto
                .restart_policy
                .as_ref()
                .map_or_else(concept::app::RestartPolicy::default, Into::into),
            update_config: dto
                .update_config
                .as_ref()
                .map_or_else(concept::app::UpdateConfig::default, Into::into),
        })
    }
}

#[serde_with::skip_serializing_none]
#[macros_derive::validated_schema]
#[derive(Debug, Serialize, Deserialize, ToSchema, Validate)]
pub(crate) struct DomainSpec {
    #[garde(custom(super::super::super::validate::valid_domain))]
    pub(crate) domain: String,
    /// Defaults to false. Needs a certificate, issued automatically after `acme_enable_account` or uploaded with `acme_upload_cert`; at most 64 HTTPS domains in total.
    #[garde(skip)]
    #[serde(default)]
    pub(crate) https: bool,
    #[garde(range(min = 1))]
    pub(crate) target: u16,
    #[garde(inner(custom(super::super::super::validate::valid_path_prefix)))]
    #[serde(default)]
    pub(crate) path_prefix: Option<String>,
    #[garde(dive)]
    #[serde(default)]
    pub(crate) header: Option<MatchSpec>,
    #[garde(dive)]
    #[serde(default)]
    pub(crate) cookie: Option<MatchSpec>,
}

#[serde_with::skip_serializing_none]
#[macros_derive::validated_schema]
#[derive(Debug, Serialize, Deserialize, ToSchema, Validate)]
pub(crate) struct MatchSpec {
    #[garde(length(min = 1, max = 255))]
    pub(crate) key: String,
    #[garde(inner(length(max = 1024)))]
    #[serde(default)]
    pub(crate) value: Option<String>,
}

super::super::super::typ::mirror_struct! {
    pingress_wire::MatchKV from spec: &MatchSpec {
        key: spec.key.clone(),
        value: spec.value.clone(),
    }
}

super::super::super::typ::mirror_struct! {
    MatchSpec from kv: &pingress_wire::MatchKV {
        key: kv.key.clone(),
        value: kv.value.clone(),
    }
}

impl TryFrom<&DomainSpec> for concept::domain::Rule {
    type Error = Error;

    fn try_from(spec: &DomainSpec) -> Result<Self> {
        let target = std::num::NonZeroU16::new(spec.target)
            .ok_or_else(|| ErrorKind::InvalidBody("target must be at least 1".to_owned()))?;
        pingress_wire::Route {
            domain: spec.domain.clone(),
            https: spec.https,
            target,
            path_prefix: spec.path_prefix.clone(),
            header: spec.header.as_ref().map(Into::into),
            cookie: spec.cookie.as_ref().map(Into::into),
        }
        .try_into()
    }
}

impl From<&concept::domain::Rule> for DomainSpec {
    fn from(rule: &concept::domain::Rule) -> Self {
        let route = rule.route();
        Self {
            domain: route.domain.clone(),
            https: route.https,
            target: route.target.get(),
            path_prefix: route.path_prefix.clone(),
            header: route.header.as_ref().map(Into::into),
            cookie: route.cookie.as_ref().map(Into::into),
        }
    }
}

#[expect(clippy::option_option, clippy::ref_option)]
fn tri<T, U>(field: &Option<Option<T>>, f: impl Fn(&T) -> U) -> Option<Option<U>> {
    field.as_ref().map(|inner| inner.as_ref().map(f))
}

#[expect(clippy::option_option, clippy::ref_option)]
fn try_tri<T, U>(
    field: &Option<Option<T>>,
    f: impl Fn(&T) -> Result<U>,
) -> Result<Option<Option<U>>> {
    field
        .as_ref()
        .map(|inner| inner.as_ref().map(f).transpose())
        .transpose()
}

#[expect(clippy::ref_option)]
fn each<T, U>(items: &Option<Vec<T>>, f: impl Fn(&T) -> U) -> Option<Vec<U>> {
    items.as_ref().map(|items| items.iter().map(f).collect())
}

#[serde_with::skip_serializing_none]
#[macros_derive::validated_schema(patch)]
#[derive(Debug, Serialize, Deserialize, ToSchema, Validate)]
pub(crate) struct Changes {
    #[garde(inner(inner(length(max = 500))))]
    pub(crate) description: Option<Option<String>>,
    /// An existing group.
    #[garde(inner(custom(super::super::super::validate::valid_name)))]
    pub(crate) group: Option<String>,
    /// Nodes pull a changed image during the rollout.
    #[garde(inner(custom(super::super::super::validate::image_reference)))]
    pub(crate) image: Option<String>,
    /// Null removes the credentials; moving `image` to another registry also drops them unless resent with a token.
    #[garde(dive)]
    pub(crate) image_auth: Option<Option<super::image::ImageAuth>>,
    /// Replaces the entire replica policy; Dynamic requires Insight collection.
    #[garde(dive)]
    pub(crate) replicas: Option<super::replicas::Replicas>,
    #[garde(dive)]
    pub(crate) ports: Option<Vec<super::network::Port>>,
    /// Requires Pingress.
    #[garde(dive)]
    pub(crate) domains: Option<Vec<DomainSpec>>,
    /// Replaces the list; resend kept files as `InUse`.
    #[garde(dive)]
    pub(crate) xconfigs: Option<Vec<super::xconfig::XConfigAction>>,
    /// Only for apps created with mounts; the node stays the same.
    #[garde(length(min = 1), dive)]
    pub(crate) mounts: Option<Vec<super::container::Mount>>,
    /// Replaces all options, including `command` and `env`.
    #[garde(dive)]
    pub(crate) container_options: Option<Option<super::container::ContainerOptions>>,
    #[garde(dive)]
    pub(crate) resources: Option<Option<super::deployment::Resources>>,
    #[garde(dive)]
    pub(crate) placement: Option<Option<super::deployment::Placement>>,
    /// The `Insight` driver needs Insight enabled; `app_logs` returns logs only for apps using it.
    #[garde(dive)]
    pub(crate) log_driver: Option<super::container::LogDriver>,
    #[garde(dive)]
    pub(crate) restart_policy: Option<super::deployment::RestartPolicy>,
    #[garde(dive)]
    pub(crate) update_config: Option<super::deployment::UpdateConfig>,
}

impl Changes {
    pub(crate) fn set_command(&mut self, command: Vec<String>) {
        match self.container_options.as_mut() {
            Some(Some(options)) => options.set_command(command),
            Some(slot @ None) => {
                if !command.is_empty() {
                    *slot = Some(super::container::ContainerOptions::from_command(command));
                }
            }
            None if !command.is_empty() => {
                self.container_options = Some(Some(
                    super::container::ContainerOptions::from_command(command),
                ));
            }
            None => {}
        }
    }
}

impl TryFrom<&Changes> for concept::app::Changes {
    type Error = Error;

    fn try_from(changes: &Changes) -> Result<Self> {
        Ok(Self::new(
            changes.description.clone(),
            changes.group.as_deref().map(str::parse).transpose()?,
            changes.image.as_deref().map(str::parse).transpose()?,
            tri(&changes.image_auth, |auth| {
                concept::image::ImageAuth::new(
                    auth.username.clone(),
                    auth.token.clone().unwrap_or_default(),
                )
            }),
            changes
                .replicas
                .as_ref()
                .map(TryInto::try_into)
                .transpose()?,
            each(&changes.ports, |p| (*p).into()),
            changes
                .domains
                .as_ref()
                .map(|d| d.iter().map(TryInto::try_into).collect())
                .transpose()?,
            None,
            each(&changes.mounts, |m| m.into()),
            tri(&changes.container_options, |o| o.into()),
            tri(&changes.resources, |r| r.into()),
            tri(&changes.placement, |p| p.into()),
            changes.log_driver.as_ref().map(Into::into),
            changes.restart_policy.as_ref().map(Into::into),
            changes.update_config.as_ref().map(Into::into),
        ))
    }
}

impl TryFrom<&concept::app::Changes> for Changes {
    type Error = Error;

    fn try_from(changes: &concept::app::Changes) -> Result<Self> {
        Ok(Self {
            description: changes.description().clone(),
            group: changes.group().as_ref().map(ToString::to_string),
            image: changes.image().as_ref().map(ToString::to_string),
            image_auth: tri(changes.image_auth(), |a| a.into()),
            replicas: changes.replicas().map(Into::into),
            ports: each(changes.ports(), |p| (*p).into()),
            domains: each(changes.domains(), |d| d.into()),
            xconfigs: each(changes.xconfigs(), |x| {
                super::xconfig::XConfigAction::InUse(x.into())
            }),
            mounts: each(changes.mounts(), |m| m.into()),
            container_options: tri(changes.container_options(), |o| o.into()),
            resources: try_tri(changes.resources(), |resources| resources.try_into())?,
            placement: tri(changes.placement(), |p| p.into()),
            log_driver: changes
                .log_driver()
                .as_ref()
                .map(super::container::LogDriver::try_from)
                .transpose()?,
            restart_policy: changes
                .restart_policy()
                .map(super::deployment::RestartPolicy::try_from)
                .transpose()?,
            update_config: changes
                .update_config()
                .map(super::deployment::UpdateConfig::try_from)
                .transpose()?,
        })
    }
}
