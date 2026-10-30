pub(crate) mod guardian;
pub(crate) mod logs;
mod runner;
pub(crate) mod runtime;
pub(crate) mod scaling;
pub(crate) mod unit;
pub(crate) mod xconfig;
pub(crate) mod xhealth;
pub(crate) mod xjob;

use std::collections::HashMap;

use cipher::{Domain, MasterKey};
use docker::types::{
    ContainerSpec, EndpointMode, EndpointPortConfig, EndpointSpec, Filter, MountType, NanoCpus,
    NanoDuration, NetworkAttachment, PortProtocol, PortPublishMode, RegistryAuth, ResourcesLimits,
    ResourcesReservations, RestartPolicyCondition, RollbackConfig, RollbackFailureAction, Service,
    ServiceMode, ServiceSpec, ServiceUpdateState, TaskTemplate,
};
use jiff::{SignedDuration, Timestamp};

use crate::error::{Error, ErrorKind, Result, SpecProblem};
use crate::{concept, config};

const LABEL_OPERATOR: &str = "dev.cdelivery.last_operator";
const LABEL_DESCRIPTION: &str = "dev.cdelivery.description";
const LABEL_IMAGE_AUTH: &str = "dev.cdelivery.image_auth";
const LABEL_SCALING: &str = "dev.cdelivery.scaling";
pub(super) const LABEL_DOMAINS: &str = "dev.cdelivery.domains";

pub(super) fn label<'a>(service: &'a Service, key: &str) -> Result<&'a str> {
    require(&service.spec.labels, key, &service.spec.name)
}

pub(super) fn bad_label(service: &str, key: &str) -> crate::error::Error {
    SpecProblem::BadLabel {
        service: service.to_owned(),
        label: key.to_owned(),
    }
    .into()
}

pub(super) fn json_label<T: serde::de::DeserializeOwned>(
    service: &Service,
    key: &str,
) -> Result<T> {
    serde_json::from_str(label(service, key)?).map_err(|_| bad_label(&service.spec.name, key))
}

pub(super) fn domains(service: &Service) -> Result<Vec<concept::domain::Rule>> {
    let rules: Vec<concept::domain::Rule> = json_label(service, LABEL_DOMAINS)?;
    if service.spec.name == config::SERVICE_NAME
        && !matches!(rules.as_slice(), [rule]
            if rule.route().target == config::PUBLIC_PORT
                && rule.route().path_prefix.is_none()
                && rule.route().header.is_none()
                && rule.route().cookie.is_none())
    {
        return Err(bad_label(&service.spec.name, LABEL_DOMAINS));
    }
    Ok(rules)
}

pub(super) async fn scan<T, Fut>(
    docker: &super::docker::Docker,
    filter: &Filter,
    observe: impl Fn(Service) -> Fut,
) -> Result<(Vec<T>, Vec<(String, Error)>)>
where
    Fut: Future<Output = Result<T>>,
{
    let services = docker.service_list(filter).await?;
    let mut entries = Vec::with_capacity(services.len());
    let mut errors = Vec::new();
    for service in services {
        let name = service.spec.name.clone();
        match observe(service).await {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                tracing::warn!(error = ?e, "skipping unreadable runner {name}");
                errors.push((name, e));
            }
        }
    }
    Ok((entries, errors))
}

pub(super) fn versioned<T: concept::versioned::Aggregate>(
    found: Option<Service>,
    decode: impl FnOnce(&Service) -> Result<T>,
) -> Result<Option<concept::versioned::Versioned<T>>> {
    match found {
        Some(service) => Ok(Some(concept::versioned::Versioned::new(
            service.version.index,
            decode(&service)?,
        ))),
        None => Ok(None),
    }
}

fn encode_labels(key: &MasterKey, app: &concept::app::App) -> Result<HashMap<String, String>> {
    let mut labels = unit::labels(app.name(), unit::Kind::App);
    labels.extend([
        (
            super::group::LABEL_GROUP.to_owned(),
            app.group().to_string(),
        ),
        (
            LABEL_OPERATOR.to_owned(),
            super::label::json(&app.last_operator()),
        ),
    ]);
    if let Some(description) = app.description() {
        labels.insert(LABEL_DESCRIPTION.to_owned(), description.clone());
    }
    if let Some(auth) = app.image_auth() {
        let sealed = key.cipher_for(Domain::AppImageAuth).seal(auth)?;
        labels.insert(LABEL_IMAGE_AUTH.to_owned(), sealed);
    }
    if !app.domains().is_empty() {
        labels.insert(LABEL_DOMAINS.to_owned(), super::label::json(app.domains()));
    }
    if let Some(scaling) = app.scaling() {
        labels.insert(LABEL_SCALING.to_owned(), super::label::json(scaling));
    }
    Ok(labels)
}

fn require<'a>(labels: &'a HashMap<String, String>, key: &str, service: &str) -> Result<&'a str> {
    labels.get(key).map(String::as_str).ok_or_else(|| {
        SpecProblem::MissingLabel {
            service: service.to_owned(),
            label: key.to_owned(),
        }
        .into()
    })
}

fn decode_spec(
    key: &MasterKey,
    spec: &ServiceSpec,
    created_at: Timestamp,
) -> Result<concept::app::App> {
    let service = spec.name.as_str();
    let labels = &spec.labels;
    let (name, kind) = unit::decode(spec)?;
    if kind != unit::Kind::App {
        return Err(SpecProblem::NotApp(service.to_owned()).into());
    }
    let auth = labels
        .get(LABEL_IMAGE_AUTH)
        .map(|sealed| key.cipher_for(Domain::AppImageAuth).open(sealed))
        .transpose()?;
    let ServiceMode::Replicated { replicas } = spec.mode else {
        return Err(SpecProblem::NotReplicated(service.to_owned()).into());
    };
    let container = &spec.task_template.container_spec;
    let app = concept::app::App::hydrate(
        concept::app::Desired {
            name,
            description: labels.get(LABEL_DESCRIPTION).cloned(),
            group: require(labels, super::group::LABEL_GROUP, service)?
                .parse()
                .map_err(|_| SpecProblem::BadGroup(service.to_owned()))?,
            image: container.image.parse()?,
            image_auth: auth,
            replicas,
            scaling: labels
                .get(LABEL_SCALING)
                .map(|raw| serde_json::from_str(raw))
                .transpose()
                .map_err(|_| bad_label(service, LABEL_SCALING))?,
            ports: decode_ports(spec)?,
            domains: labels
                .get(LABEL_DOMAINS)
                .map(|raw| serde_json::from_str(raw))
                .transpose()
                .map_err(|_| bad_label(service, LABEL_DOMAINS))?
                .unwrap_or_default(),
            xconfigs: container.configs.iter().map(xconfig::to_ref).collect(),
            mounts: decode_mounts(spec)?,
            container_options: decode_container_options(container),
            resources: spec.task_template.resources.as_ref().map(decode_resources),
            placement: decode_placement(spec)?,
            log_driver: decode_log_driver(&spec.name, spec.task_template.log_driver.as_ref())?,
            restart_policy: decode_restart_policy(spec)?,
            update_config: decode_update_config(spec)?,
        },
        serde_json::from_str(require(labels, LABEL_OPERATOR, service)?)
            .map_err(|_| bad_label(service, LABEL_OPERATOR))?,
        created_at,
        None,
    );
    if app.scaling().is_some() {
        app.validate()
            .map_err(|_| bad_label(service, LABEL_SCALING))?;
    }
    Ok(app)
}

fn hydrate(key: &MasterKey, service: &Service) -> Result<concept::app::App> {
    let current = decode_spec(key, &service.spec, service.created_at)?;
    let rollback = derive_rollback(key, service, &current);
    Ok(current.with_rollback(rollback))
}

fn derive_rollback(
    key: &MasterKey,
    service: &Service,
    current: &concept::app::App,
) -> Option<concept::app::Rollback> {
    let state = service.update_status.as_ref().map(|status| &status.state);
    let change = |rolled_back| {
        let spec = service.previous_spec.as_ref()?;
        let previous = decode_spec(key, spec, service.created_at)
            .inspect_err(|error| {
                tracing::warn!(error = ?error, "app {} previous spec is undecodable; change unavailable", service.spec.name);
            })
            .ok()?;
        let (source, target) = if rolled_back {
            (current, &previous)
        } else {
            (&previous, current)
        };
        let mounts_supported = match (source.mounts(), target.mounts()) {
            (None, Some(_)) => false,
            (Some(source), Some(target)) => source.node_id() == target.node_id(),
            _ => true,
        };
        if !mounts_supported {
            tracing::warn!(
                "app {} previous edge contains an unsupported mount transition; change unavailable",
                service.spec.name
            );
            return None;
        }
        let changes = source.diff(target);
        (changes != concept::app::Changes::default()).then(|| {
            concept::app::Change::new(
                changes,
                target.last_operator(),
                (!rolled_back).then_some(service.updated_at),
            )
        })
    };
    match state {
        Some(ServiceUpdateState::RollbackStarted) => {
            Some(concept::app::Rollback::InProgress(change(true)))
        }
        Some(ServiceUpdateState::RollbackPaused) => {
            Some(concept::app::Rollback::Paused(change(true)))
        }
        Some(ServiceUpdateState::RollbackCompleted) => {
            Some(concept::app::Rollback::Completed(change(true)))
        }
        Some(ServiceUpdateState::Unknown(state)) => {
            tracing::warn!(
                "app {} has unknown update state {state}; rollback unavailable",
                service.spec.name
            );
            None
        }
        None
        | Some(
            ServiceUpdateState::Updating
            | ServiceUpdateState::Paused
            | ServiceUpdateState::Completed,
        ) => change(false).map(concept::app::Rollback::Available),
    }
}

pub(crate) fn build_spec(key: &MasterKey, app: &concept::app::App) -> Result<ServiceSpec> {
    Ok(ServiceSpec {
        extra: serde_json::Map::new(),
        name: app.name().to_string(),
        labels: encode_labels(key, app)?,
        task_template: TaskTemplate {
            container_spec: ContainerSpec {
                image: app.image().to_string(),
                command: app
                    .container_options()
                    .as_ref()
                    .map(|o| o.command().clone())
                    .unwrap_or_default(),
                env: runner_env(app),
                mounts: app.mounts().as_ref().map(encode_mounts).unwrap_or_default(),
                configs: app
                    .xconfigs()
                    .iter()
                    .map(xconfig::to_service_config)
                    .collect(),
                ..ContainerSpec::default()
            },
            resources: app.resources().as_ref().map(encode_resources),
            restart_policy: Some(docker::types::RestartPolicy {
                condition: RestartPolicyCondition::Any,
                delay: Some(to_nano(app.restart_policy().delay())?),
                max_attempts: None,
                window: None,
            }),
            placement: encode_placement(app),
            networks: Some(encode_networks(app)),
            log_driver: encode_log_driver(app.log_driver()),
            force_update: 0,
            extra: serde_json::Map::new(),
        },
        mode: ServiceMode::Replicated {
            replicas: app.replicas(),
        },
        update_config: Some(encode_update_config(app.update_config())?),
        rollback_config: Some(encode_rollback_config(app.update_config())?),
        endpoint_spec: Some(encode_endpoint(app.ports())),
    })
}

fn encode_networks(app: &concept::app::App) -> Vec<NetworkAttachment> {
    let mut targets = vec![super::group::resource_name(app.group())];
    if !app.domains().is_empty() {
        targets.push(crate::config::PINGRESS_NETWORK.to_owned());
    }
    targets
        .into_iter()
        .map(|target| NetworkAttachment {
            target,
            aliases: None,
            driver_opts: None,
        })
        .collect()
}

pub(crate) async fn list_domains(
    docker: &super::docker::Docker,
) -> Result<Vec<concept::app::Routing>> {
    let mut apps = Vec::new();
    for service in docker
        .service_list(&Filter::new().label(LABEL_DOMAINS))
        .await?
    {
        let rules = domains(&service)?;
        if !rules.is_empty() {
            apps.push(concept::app::Routing::new(
                service.spec.name,
                service.id,
                rules,
            ));
        }
    }
    Ok(apps)
}

fn runner_env(app: &concept::app::App) -> Vec<String> {
    app.container_options()
        .as_ref()
        .map(|o| o.env().iter().map(|(k, v)| format!("{k}={v}")).collect())
        .unwrap_or_default()
}

pub(crate) fn registry_auth(app: &concept::app::App) -> Option<RegistryAuth> {
    app.image_auth().as_ref().map(|auth| RegistryAuth {
        username: Some(auth.username().clone()),
        password: Some(auth.token().clone()),
        identity_token: None,
        server_address: app.image().registry().to_owned(),
    })
}

fn encode_placement(app: &concept::app::App) -> Option<docker::types::Placement> {
    if let Some(mounts) = app.mounts() {
        return Some(docker::types::Placement {
            constraints: Some(vec![super::node::id_constraint(mounts.node_id())]),
            max_replicas: Some(1),
            extra: serde_json::Map::new(),
        });
    }
    app.placement().as_ref().map(|p| docker::types::Placement {
        constraints: p
            .label()
            .as_ref()
            .map(|label| vec![super::node::label_constraint(label)]),
        max_replicas: p.max_replicas_per_node(),
        extra: serde_json::Map::new(),
    })
}

fn decode_placement(spec: &ServiceSpec) -> Result<Option<concept::app::Placement>> {
    let Some(placement) = spec.task_template.placement.as_ref() else {
        return Ok(None);
    };
    let label = match placement.constraints {
        None => None,
        Some(_) => match super::node::decode_selector(Some(placement), &spec.name)? {
            concept::node::Selector::NodeId(_) => return Ok(None),
            concept::node::Selector::Label(label) => Some(label),
        },
    };
    Ok((label.is_some() || placement.max_replicas.is_some())
        .then(|| concept::app::Placement::new(label, placement.max_replicas)))
}

fn encode_mounts(mounts: &concept::app::NodeMounts) -> Vec<docker::types::Mount> {
    mounts
        .mounts()
        .iter()
        .map(|m| docker::types::Mount {
            kind: MountType::Volume,
            target: m.target().clone(),
            source: format!("{}{}", cdelivery_wire::VOLUME_PREFIX, m.volume()),
            read_only: m.read_only(),
            bind_options: None,
        })
        .collect()
}

fn decode_mounts(spec: &ServiceSpec) -> Result<Option<concept::app::NodeMounts>> {
    let mounts = &spec.task_template.container_spec.mounts;
    if mounts.is_empty() {
        return Ok(None);
    }
    let list = mounts
        .iter()
        .map(|m| {
            let volume = m
                .source
                .strip_prefix(cdelivery_wire::VOLUME_PREFIX)
                .filter(|name| {
                    matches!(m.kind, MountType::Volume)
                        && name.parse::<concept::name::Name>().is_ok()
                })
                .ok_or_else(|| SpecProblem::UnsupportedMount(spec.name.clone()))?;
            Ok(concept::app::Mount::new(
                m.target.clone(),
                volume.to_owned(),
                m.read_only,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let placement = spec.task_template.placement.as_ref();
    let node_id = match super::node::decode_selector(placement, &spec.name)? {
        concept::node::Selector::NodeId(id) => id,
        concept::node::Selector::Label(_) => {
            return Err(SpecProblem::NotPinned(spec.name.clone()).into());
        }
    };
    Ok(Some(concept::app::NodeMounts::new(node_id, list)))
}

fn decode_container_options(container: &ContainerSpec) -> Option<concept::app::ContainerOptions> {
    if container.command.is_empty() && container.env.is_empty() {
        return None;
    }
    let env = container
        .env
        .iter()
        .map(|kv| match kv.split_once('=') {
            Some((k, v)) => (k.to_owned(), v.to_owned()),
            None => (kv.clone(), String::new()),
        })
        .collect();
    Some(concept::app::ContainerOptions::new(
        container.command.clone(),
        env,
    ))
}

fn encode_resources(resources: &concept::app::Resources) -> docker::types::Resources {
    let cpus = |spec: &concept::app::ResourceSpec| spec.cpus().map(NanoCpus::from_cores);
    docker::types::Resources {
        limits: resources.limits().as_ref().map(|l| ResourcesLimits {
            nano_cpus: cpus(l),
            memory_bytes: l.memory_bytes(),
            pids: None,
        }),
        reservations: resources
            .reservations()
            .as_ref()
            .map(|r| ResourcesReservations {
                nano_cpus: cpus(r),
                memory_bytes: r.memory_bytes(),
            }),
    }
}

fn decode_resources(resources: &docker::types::Resources) -> concept::app::Resources {
    concept::app::Resources::new(
        resources.limits.as_ref().map(|l| {
            concept::app::ResourceSpec::new(
                l.nano_cpus.as_ref().map(NanoCpus::cores),
                l.memory_bytes,
            )
        }),
        resources.reservations.as_ref().map(|r| {
            concept::app::ResourceSpec::new(
                r.nano_cpus.as_ref().map(NanoCpus::cores),
                r.memory_bytes,
            )
        }),
    )
}

fn encode_log_driver(log_driver: &concept::app::LogDriver) -> Option<docker::types::LogDriver> {
    match log_driver {
        concept::app::LogDriver::FollowHost => None,
        concept::app::LogDriver::JsonFile { max_size, max_file } => {
            Some(docker::types::LogDriver {
                name: "json-file".to_owned(),
                options: Some(HashMap::from([
                    (
                        "max-size".to_owned(),
                        format!("{}m", max_size / concept::app::MIB),
                    ),
                    ("max-file".to_owned(), max_file.to_string()),
                ])),
            })
        }
        concept::app::LogDriver::Insight => Some(super::insight::fluentd_log_driver()),
    }
}

fn decode_log_driver(
    service: &str,
    log_driver: Option<&docker::types::LogDriver>,
) -> Result<concept::app::LogDriver> {
    let Some(driver) = log_driver else {
        return Ok(concept::app::LogDriver::FollowHost);
    };
    if super::insight::is_fluentd(Some(driver)) {
        return Ok(concept::app::LogDriver::Insight);
    }
    match driver.name.as_str() {
        "json-file" => {}
        other => {
            return Err(SpecProblem::UnknownLogDriver {
                service: service.to_owned(),
                driver: other.to_owned(),
            }
            .into());
        }
    }
    let options = driver.options.clone().unwrap_or_default();
    let max_size = match options.get("max-size").and_then(|s| s.strip_suffix('m')) {
        Some(size) => size
            .parse::<u64>()
            .map_err(|_| SpecProblem::BadLogOption("max-size"))?
            .saturating_mul(concept::app::MIB),
        None => 10 * concept::app::MIB,
    };
    let max_file = match options.get("max-file") {
        Some(count) => count
            .parse()
            .map_err(|_| SpecProblem::BadLogOption("max-file"))?,
        None => 1,
    };
    Ok(concept::app::LogDriver::JsonFile { max_size, max_file })
}

pub(crate) async fn list_insight_consumers(docker: &super::docker::Docker) -> Result<Vec<String>> {
    let mut names: Vec<String> = docker
        .service_list(&Filter::new().label(unit::LABEL_APP))
        .await?
        .into_iter()
        .filter(|service| {
            super::insight::is_fluentd(service.spec.task_template.log_driver.as_ref())
                || service.spec.labels.contains_key(LABEL_SCALING)
        })
        .map(|service| service.spec.name)
        .collect();
    names.sort();
    Ok(names)
}

fn decode_restart_policy(spec: &ServiceSpec) -> Result<concept::app::RestartPolicy> {
    let policy = spec
        .task_template
        .restart_policy
        .as_ref()
        .ok_or_else(|| SpecProblem::NoRestartPolicy(spec.name.clone()))?;
    Ok(concept::app::RestartPolicy::new(
        policy
            .delay
            .as_ref()
            .map_or(SignedDuration::ZERO, from_nano),
    ))
}

fn encode_update_config(config: concept::app::UpdateConfig) -> Result<docker::types::UpdateConfig> {
    Ok(docker::types::UpdateConfig {
        parallelism: config.parallelism(),
        delay: None,
        failure_action: Some(match config.failure_action() {
            concept::app::UpdateFailureAction::Continue => {
                docker::types::UpdateFailureAction::Continue
            }
            concept::app::UpdateFailureAction::Pause => docker::types::UpdateFailureAction::Pause,
            concept::app::UpdateFailureAction::Rollback => {
                docker::types::UpdateFailureAction::Rollback
            }
        }),
        monitor: Some(to_nano(config.monitor())?),
        max_failure_ratio: Some(f32::from(config.max_failure_percent()) / 100.0),
        order: Some(config.order().into()),
    })
}

fn encode_rollback_config(config: concept::app::UpdateConfig) -> Result<RollbackConfig> {
    Ok(RollbackConfig {
        parallelism: None,
        delay: None,
        failure_action: Some(RollbackFailureAction::Continue),
        monitor: Some(to_nano(config.monitor())?),
        max_failure_ratio: Some(f32::from(config.max_failure_percent()) / 100.0),
        order: Some(config.order().into()),
    })
}

macros::mirror_enum! {
    concept::app::UpdateOrder <-> docker::types::UpdateOrder { StopFirst, StartFirst }
}
macros::mirror_enum! {
    docker::types::UpdateFailureAction => concept::app::UpdateFailureAction {
        Continue, Pause, Rollback
    }
}

fn decode_update_config(spec: &ServiceSpec) -> Result<concept::app::UpdateConfig> {
    let bad = || SpecProblem::BadUpdateConfig(spec.name.clone());
    let config = spec.update_config.as_ref().ok_or_else(bad)?;
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let percent = (config.max_failure_ratio.unwrap_or(0.0) * 100.0).round() as u8;
    Ok(concept::app::UpdateConfig::new(
        config.parallelism.filter(|p| *p != 0),
        config.failure_action.clone().ok_or_else(bad)?.into(),
        config
            .monitor
            .as_ref()
            .map_or(SignedDuration::ZERO, from_nano),
        percent,
        config.order.clone().ok_or_else(bad)?.into(),
    ))
}

fn encode_endpoint(ports: &[concept::app::Port]) -> EndpointSpec {
    EndpointSpec {
        mode: EndpointMode::Vip,
        ports: (!ports.is_empty()).then(|| {
            ports
                .iter()
                .map(|p| EndpointPortConfig {
                    protocol: match p.protocol() {
                        concept::app::PortProtocol::Tcp => PortProtocol::Tcp,
                        concept::app::PortProtocol::Udp => PortProtocol::Udp,
                    },
                    target_port: p.target(),
                    published_port: Some(p.published()),
                    publish_mode: PortPublishMode::Ingress,
                })
                .collect()
        }),
    }
}

fn decode_ports(spec: &ServiceSpec) -> Result<Vec<concept::app::Port>> {
    let Some(ports) = spec.endpoint_spec.as_ref().and_then(|e| e.ports.as_ref()) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for port in ports {
        let Some(published) = port.published_port else {
            continue;
        };
        let protocol = match port.protocol {
            PortProtocol::Tcp => concept::app::PortProtocol::Tcp,
            PortProtocol::Udp => concept::app::PortProtocol::Udp,
            PortProtocol::Sctp => {
                return Err(SpecProblem::BadPortProtocol(spec.name.clone()).into());
            }
        };
        out.push(concept::app::Port::new(
            port.target_port,
            published,
            protocol,
        ));
    }
    Ok(out)
}

fn to_nano(duration: SignedDuration) -> Result<NanoDuration> {
    let nanos = u64::try_from(duration.as_nanos())
        .map_err(|_| ErrorKind::UnprocessableBody("app duration is out of range".to_owned()))?;
    Ok(NanoDuration(nanos))
}

fn from_nano(nano: &NanoDuration) -> SignedDuration {
    SignedDuration::from_nanos_i128(i128::from(nano.0))
}

pub(crate) async fn create(
    docker: &super::docker::Docker,
    key: &MasterKey,
    app: &concept::app::App,
) -> Result<()> {
    let spec = build_spec(key, app)?;
    docker
        .service_create_unique(&spec, registry_auth(app).as_ref(), ErrorKind::AppExists)
        .await
}

pub(crate) async fn load(
    docker: &super::docker::Docker,
    key: &MasterKey,
    name: &concept::name::Name,
) -> Result<Option<concept::versioned::Versioned<concept::app::App>>> {
    let found = docker
        .service_find(unit::Kind::App.for_app(name), name.as_str())
        .await?;
    versioned(found, |service| hydrate(key, service))
}

pub(crate) async fn service_id(
    docker: &super::docker::Docker,
    name: &concept::name::Name,
) -> Result<Option<String>> {
    Ok(docker
        .service_find(unit::Kind::App.for_app(name), name.as_str())
        .await?
        .map(|service| service.id))
}

pub(crate) async fn units(
    docker: &super::docker::Docker,
) -> Result<Vec<(concept::name::Name, String, u64, u64)>> {
    let services = docker.service_list(&unit::Kind::App.filter()).await?;
    let mut units = Vec::with_capacity(services.len());
    for service in services {
        let name = match unit::owner(&service.spec, unit::Kind::App) {
            Ok(name) => name,
            Err(error) => {
                tracing::warn!(error = ?error, "skipping undecodable app unit {}", service.spec.name);
                continue;
            }
        };
        let Some(status) = service.status else {
            tracing::warn!(
                "app unit {} listed without service status",
                service.spec.name
            );
            continue;
        };
        units.push((name, service.id, status.running, status.desired));
    }
    units.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
    Ok(units)
}

pub(crate) async fn list(
    docker: &super::docker::Docker,
) -> Result<Vec<(concept::name::Name, concept::name::Name)>> {
    let services = docker.service_list(&unit::Kind::App.filter()).await?;
    let mut apps = Vec::with_capacity(services.len());
    for service in services {
        let summary = (|| {
            let name = unit::owner(&service.spec, unit::Kind::App)?;
            let group: concept::name::Name = require(
                &service.spec.labels,
                super::group::LABEL_GROUP,
                &service.spec.name,
            )?
            .parse()
            .map_err(|_| SpecProblem::BadGroup(service.spec.name.clone()))?;
            Ok::<_, crate::error::Error>((name, group))
        })();
        match summary {
            Ok(app) => apps.push(app),
            Err(error) => {
                tracing::warn!(error = ?error, "skipping undecodable app summary {}", service.spec.name);
            }
        }
    }
    apps.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
    Ok(apps)
}

pub(crate) async fn save(
    docker: &super::docker::Docker,
    key: &MasterKey,
    app: &concept::versioned::Versioned<concept::app::App>,
) -> Result<()> {
    if !app.changed() {
        return Ok(());
    }
    let spec = build_spec(key, app.get())?;
    docker
        .service_update(
            app.get().name().as_str(),
            app.version(),
            &spec,
            registry_auth(app.get()).as_ref(),
        )
        .await
}

pub(crate) async fn delete(
    docker: &super::docker::Docker,
    name: &concept::name::Name,
) -> Result<bool> {
    if docker
        .service_find(unit::Kind::App.for_app(name), name.as_str())
        .await?
        .is_none()
    {
        return Ok(false);
    }
    docker.service_delete(name.as_str()).await
}

pub(crate) async fn rollback(
    docker: &super::docker::Docker,
    name: &concept::name::Name,
    version: u64,
) -> Result<()> {
    docker.service_rollback(name.as_str(), version).await
}

pub(crate) async fn restart(
    docker: &super::docker::Docker,
    name: &concept::name::Name,
) -> Result<()> {
    if docker
        .service_find(unit::Kind::App.for_app(name), name.as_str())
        .await?
        .is_none()
    {
        return Err(ErrorKind::AppNotFound.into());
    }
    docker.service_restart(name.as_str()).await
}
