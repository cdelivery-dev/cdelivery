use getset::{CopyGetters, Getters};
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{ErrorKind, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum PortProtocol {
    Tcp,
    Udp,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, CopyGetters, macros_derive::Ctor,
)]
#[getset(get_copy = "pub(crate)")]
pub(crate) struct Port {
    target: u16,
    published: u16,
    protocol: PortProtocol,
}

#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters, macros_derive::Ctor,
)]
pub(crate) struct Mount {
    #[getset(get = "pub(crate)")]
    target: String,
    #[getset(get = "pub(crate)")]
    volume: String,
    #[getset(get_copy = "pub(crate)")]
    read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Getters, macros_derive::Ctor)]
#[getset(get = "pub(crate)")]
pub(crate) struct NodeMounts {
    node_id: String,
    mounts: Vec<Mount>,
}

#[derive(Debug, Getters, macros_derive::Ctor)]
#[getset(get = "pub(crate)")]
pub(crate) struct Routing {
    name: String,
    service_id: String,
    rules: Vec<super::domain::Rule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, macros_derive::Ctor)]
#[getset(get = "pub(crate)")]
pub(crate) struct ContainerOptions {
    command: Vec<String>,
    env: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, macros_derive::Ctor)]
#[getset(get = "pub(crate)")]
pub(crate) struct Resources {
    limits: Option<ResourceSpec>,
    reservations: Option<ResourceSpec>,
}

impl Resources {
    fn normalized(mut self) -> Option<Self> {
        let configured = |spec: &ResourceSpec| spec.cpus.is_some() || spec.memory_bytes.is_some();
        self.limits = self.limits.filter(configured);
        self.reservations = self.reservations.filter(configured);
        (self.limits.is_some() || self.reservations.is_some()).then_some(self)
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Serialize, Deserialize, CopyGetters, macros_derive::Ctor,
)]
#[getset(get_copy = "pub(crate)")]
pub(crate) struct ResourceSpec {
    cpus: Option<f64>,
    memory_bytes: Option<u64>,
}

impl Eq for ResourceSpec {}

#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters, macros_derive::Ctor,
)]
pub(crate) struct Placement {
    #[getset(get = "pub(crate)")]
    label: Option<String>,
    #[getset(get_copy = "pub(crate)")]
    max_replicas_per_node: Option<u64>,
}

pub(crate) const MIB: u64 = 1 << 20;
pub(crate) const MAX_REPLICAS: u64 = 65_536;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, CopyGetters)]
#[getset(get_copy = "pub(crate)")]
#[serde(deny_unknown_fields)]
pub(crate) struct Dynamic {
    min: u64,
    max: Option<u64>,
}

impl Dynamic {
    pub(crate) fn new(min: u64, max: Option<u64>) -> Result<Self> {
        if !(1..=MAX_REPLICAS).contains(&min)
            || max.is_some_and(|max| !(min..=MAX_REPLICAS).contains(&max))
        {
            return Err(ErrorKind::ReplicaBoundsInvalid.into());
        }
        Ok(Self { min, max })
    }

    pub(crate) fn clamp(self, replicas: u64) -> u64 {
        replicas.clamp(self.min, self.max.unwrap_or(MAX_REPLICAS))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Replicas {
    Fixed(u64),
    Dynamic(Dynamic),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum LogDriver {
    #[default]
    FollowHost,
    JsonFile {
        max_size: u64,
        max_file: u64,
    },
    Insight,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum LogSource {
    App,
    XHealth,
    XJob { job: super::name::Name },
    GuardianApp,
    GuardianCheck,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, CopyGetters, macros_derive::Ctor,
)]
#[getset(get_copy = "pub(crate)")]
pub(crate) struct RestartPolicy {
    delay: SignedDuration,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            delay: SignedDuration::from_secs(60),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum UpdateFailureAction {
    Continue,
    Pause,
    Rollback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum UpdateOrder {
    StopFirst,
    StartFirst,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, CopyGetters, macros_derive::Ctor,
)]
#[getset(get_copy = "pub(crate)")]
pub(crate) struct UpdateConfig {
    parallelism: Option<u64>,
    failure_action: UpdateFailureAction,
    monitor: SignedDuration,
    max_failure_percent: u8,
    order: UpdateOrder,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            parallelism: None,
            failure_action: UpdateFailureAction::Rollback,
            monitor: crate::config::SERVICE_UPDATE_MONITOR,
            max_failure_percent: 0,
            order: UpdateOrder::StopFirst,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Operator {
    User { id: Uuid },
    ApiKey { user_id: Uuid },
    Agent { user_id: Uuid },
    Duty,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, macros_derive::Ctor)]
#[getset(get = "pub(crate)")]
pub(crate) struct XConfigRef {
    target: String,
    config_name: String,
    config_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Getters, macros_derive::Ctor)]
#[getset(get = "pub(crate)")]
pub(crate) struct Change {
    changes: Changes,
    operator: Operator,
    changed_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Rollback {
    Available(Change),
    InProgress(Option<Change>),
    Paused(Option<Change>),
    Completed(Option<Change>),
}

#[expect(clippy::option_option)]
#[serde_with::skip_serializing_none]
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Default,
    Serialize,
    Deserialize,
    Getters,
    CopyGetters,
    macros_derive::Ctor,
)]
#[serde(default)]
pub(crate) struct Changes {
    #[serde(with = "serde_with::rust::double_option")]
    #[getset(get = "pub(crate)")]
    description: Option<Option<String>>,
    #[getset(get = "pub(crate)")]
    group: Option<super::name::Name>,
    #[getset(get = "pub(crate)")]
    image: Option<super::image::Reference>,
    #[serde(with = "serde_with::rust::double_option")]
    #[getset(get = "pub(crate)")]
    image_auth: Option<Option<super::image::ImageAuth>>,
    #[getset(get_copy = "pub(crate)")]
    replicas: Option<Replicas>,
    #[getset(get = "pub(crate)")]
    ports: Option<Vec<Port>>,
    #[getset(get = "pub(crate)")]
    domains: Option<Vec<super::domain::Rule>>,
    #[getset(get = "pub(crate)")]
    xconfigs: Option<Vec<XConfigRef>>,
    #[getset(get = "pub(crate)")]
    mounts: Option<Vec<Mount>>,
    #[serde(with = "serde_with::rust::double_option")]
    #[getset(get = "pub(crate)")]
    container_options: Option<Option<ContainerOptions>>,
    #[serde(with = "serde_with::rust::double_option")]
    #[getset(get = "pub(crate)")]
    resources: Option<Option<Resources>>,
    #[serde(with = "serde_with::rust::double_option")]
    #[getset(get = "pub(crate)")]
    placement: Option<Option<Placement>>,
    #[getset(get = "pub(crate)")]
    log_driver: Option<LogDriver>,
    #[getset(get_copy = "pub(crate)")]
    restart_policy: Option<RestartPolicy>,
    #[getset(get_copy = "pub(crate)")]
    update_config: Option<UpdateConfig>,
}

impl Changes {
    pub(crate) fn replace_xconfigs(&mut self, xconfigs: Vec<XConfigRef>) {
        self.xconfigs = Some(xconfigs);
    }

    pub(crate) fn guardian_relevant(&self) -> bool {
        self.group.is_some()
            || self.image.is_some()
            || self.container_options.is_some()
            || self.xconfigs.is_some()
    }

    pub(crate) fn health_target_changed(&self) -> bool {
        self.ports.is_some() || self.domains.is_some()
    }

    pub(crate) fn apply(&self, app: &mut App) -> Result<()> {
        let mut updated = app.clone();
        self.apply_to(&mut updated)?;
        updated.validate()?;
        *app = updated;
        Ok(())
    }

    fn apply_to(&self, app: &mut App) -> Result<()> {
        if let Some(d) = &self.description {
            app.description = d.clone().filter(|s| !s.is_empty());
        }
        if let Some(g) = &self.group {
            app.group = g.clone();
        }
        let same_registry = self
            .image
            .as_ref()
            .is_none_or(|image| image.same_registry(&app.image));
        if let Some(image) = &self.image {
            app.image.clone_from(image);
        }
        let current = app.image_auth.take().filter(|_| same_registry);
        app.image_auth = match &self.image_auth {
            None => current,
            Some(None) => None,
            Some(Some(auth)) => Some(
                auth.inherit(current)
                    .ok_or_else(|| ErrorKind::SecretRequired("image_auth.token".into()))?,
            ),
        };
        if let Some(replicas) = self.replicas {
            match replicas {
                Replicas::Fixed(count) => {
                    app.replicas = count;
                    app.scaling = None;
                }
                Replicas::Dynamic(scaling) => {
                    let scaling = Dynamic::new(scaling.min, scaling.max)?;
                    if app.scaling != Some(scaling) {
                        app.replicas = scaling.clamp(app.replicas);
                        app.scaling = Some(scaling);
                    }
                }
            }
        }
        if let Some(p) = &self.ports {
            app.ports.clone_from(p);
        }
        if let Some(d) = &self.domains {
            app.domains.clone_from(d);
        }
        if let Some(x) = &self.xconfigs {
            if app.mounts.is_some() && !x.is_empty() {
                return Err(ErrorKind::XConfigMountsConflict.into());
            }
            app.xconfigs.clone_from(x);
        }
        if let Some(m) = &self.mounts {
            let Some(mounts) = &mut app.mounts else {
                return Err(ErrorKind::MountsRequireStateful.into());
            };
            mounts.mounts.clone_from(m);
        }
        if let Some(c) = &self.container_options {
            app.container_options.clone_from(c);
        }
        if let Some(r) = &self.resources {
            app.resources = r.clone().and_then(Resources::normalized);
        }
        if let Some(p) = &self.placement {
            if p.is_some() && app.mounts.is_some() {
                return Err(ErrorKind::MountsPlacementConflict.into());
            }
            app.placement.clone_from(p);
        }
        if let Some(l) = &self.log_driver {
            app.log_driver.clone_from(l);
        }
        if let Some(r) = self.restart_policy {
            app.restart_policy = r;
        }
        if let Some(u) = self.update_config {
            app.update_config = u;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct Desired {
    pub(crate) name: super::name::Name,
    pub(crate) description: Option<String>,
    pub(crate) group: super::name::Name,
    pub(crate) image: super::image::Reference,
    pub(crate) image_auth: Option<super::image::ImageAuth>,
    pub(crate) replicas: u64,
    pub(crate) scaling: Option<Dynamic>,
    pub(crate) ports: Vec<Port>,
    pub(crate) domains: Vec<super::domain::Rule>,
    pub(crate) xconfigs: Vec<XConfigRef>,
    pub(crate) mounts: Option<NodeMounts>,
    pub(crate) container_options: Option<ContainerOptions>,
    pub(crate) resources: Option<Resources>,
    pub(crate) placement: Option<Placement>,
    pub(crate) log_driver: LogDriver,
    pub(crate) restart_policy: RestartPolicy,
    pub(crate) update_config: UpdateConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Getters, CopyGetters)]
pub(crate) struct App {
    #[getset(get = "pub(crate)")]
    name: super::name::Name,
    #[getset(get = "pub(crate)")]
    description: Option<String>,
    #[getset(get = "pub(crate)")]
    group: super::name::Name,
    #[getset(get = "pub(crate)")]
    image: super::image::Reference,
    #[getset(get = "pub(crate)")]
    image_auth: Option<super::image::ImageAuth>,
    #[getset(get_copy = "pub(crate)")]
    replicas: u64,
    #[getset(get = "pub(crate)")]
    scaling: Option<Dynamic>,
    #[getset(get = "pub(crate)")]
    ports: Vec<Port>,
    #[getset(get = "pub(crate)")]
    domains: Vec<super::domain::Rule>,
    #[getset(get = "pub(crate)")]
    xconfigs: Vec<XConfigRef>,
    #[getset(get = "pub(crate)")]
    mounts: Option<NodeMounts>,
    #[getset(get = "pub(crate)")]
    container_options: Option<ContainerOptions>,
    #[getset(get = "pub(crate)")]
    resources: Option<Resources>,
    #[getset(get = "pub(crate)")]
    placement: Option<Placement>,
    #[getset(get = "pub(crate)")]
    log_driver: LogDriver,
    #[getset(get_copy = "pub(crate)")]
    restart_policy: RestartPolicy,
    #[getset(get_copy = "pub(crate)")]
    update_config: UpdateConfig,
    #[getset(get_copy = "pub(crate)")]
    last_operator: Operator,
    #[getset(get_copy = "pub(crate)")]
    created_at: Timestamp,
    #[getset(get = "pub(crate)")]
    rollback: Option<Rollback>,
}

#[derive(Debug)]
pub(crate) enum UpdateOutcome {
    Applied(Box<App>),
    Guarded,
}

impl App {
    pub(crate) fn new(desired: Desired, operator: Operator, now: Timestamp) -> Result<Self> {
        if desired.mounts.is_some() && desired.placement.is_some() {
            return Err(ErrorKind::MountsPlacementConflict.into());
        }
        if desired.mounts.is_some() && !desired.xconfigs.is_empty() {
            return Err(ErrorKind::XConfigMountsConflict.into());
        }
        let app = Self::hydrate(desired, operator, now, None);
        app.validate()?;
        Ok(app)
    }

    pub(crate) fn hydrate(
        desired: Desired,
        operator: Operator,
        created_at: Timestamp,
        rollback: Option<Rollback>,
    ) -> Self {
        Self {
            name: desired.name,
            description: desired.description.filter(|s| !s.is_empty()),
            group: desired.group,
            image: desired.image,
            image_auth: desired.image_auth,
            replicas: desired.replicas,
            scaling: desired.scaling,
            ports: desired.ports,
            domains: desired.domains,
            xconfigs: desired.xconfigs,
            mounts: desired.mounts,
            container_options: desired.container_options,
            resources: desired.resources.and_then(Resources::normalized),
            placement: desired.placement,
            log_driver: desired.log_driver,
            restart_policy: desired.restart_policy,
            update_config: desired.update_config,
            last_operator: operator,
            created_at,
            rollback,
        }
    }

    pub(crate) fn record_operator(&mut self, operator: Operator) {
        self.last_operator = operator;
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.mounts.is_some() && (self.scaling.is_some() || self.replicas != 1) {
            return Err(ErrorKind::StatefulReplicasPinned.into());
        }
        if let Some(scaling) = self.scaling {
            Dynamic::new(scaling.min, scaling.max)?;
        }
        Ok(())
    }

    pub(crate) fn with_rollback(mut self, rollback: Option<Rollback>) -> Self {
        self.rollback = rollback;
        self
    }

    pub(crate) fn replica_policy(&self) -> Replicas {
        self.scaling
            .map_or(Replicas::Fixed(self.replicas), Replicas::Dynamic)
    }

    pub(crate) fn diff(&self, target: &App) -> Changes {
        Changes {
            description: (self.description != target.description)
                .then(|| target.description.clone()),
            group: (self.group != target.group).then(|| target.group.clone()),
            image: (self.image != target.image).then(|| target.image.clone()),
            image_auth: (self.image_auth != target.image_auth
                || (!self.image.same_registry(&target.image) && target.image_auth.is_some()))
            .then(|| target.image_auth.clone()),
            replicas: (self.replica_policy() != target.replica_policy())
                .then(|| target.replica_policy()),
            ports: (self.ports != target.ports).then(|| target.ports.clone()),
            domains: (self.domains != target.domains).then(|| target.domains.clone()),
            xconfigs: (self.xconfigs != target.xconfigs).then(|| target.xconfigs.clone()),
            mounts: (self.mounts != target.mounts).then(|| {
                target
                    .mounts
                    .as_ref()
                    .map(|m| m.mounts().clone())
                    .unwrap_or_default()
            }),
            container_options: (self.container_options != target.container_options)
                .then(|| target.container_options.clone()),
            resources: (self.resources != target.resources).then(|| target.resources.clone()),
            placement: (self.placement != target.placement).then(|| target.placement.clone()),
            log_driver: (self.log_driver != target.log_driver).then(|| target.log_driver.clone()),
            restart_policy: (self.restart_policy != target.restart_policy)
                .then_some(target.restart_policy),
            update_config: (self.update_config != target.update_config)
                .then_some(target.update_config),
        }
    }
}

impl super::versioned::Aggregate for App {}
