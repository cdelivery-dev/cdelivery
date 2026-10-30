use std::collections::HashMap;

use docker::Client;
use docker::types::{
    Config, Filter, InspectSwarm, Network, Node, NodeSpec, RegistryAuth, Secret, Service,
    ServiceSpec, SystemInfo, Task,
};

use crate::error::{ErrorKind, Result};

pub(crate) const PLACEHOLDER: &[u8] = b"x";

fn managed_spec(spec: &ServiceSpec) -> ServiceSpec {
    let mut spec = spec.clone();
    super::label::managed(&mut spec.labels);
    super::label::managed(&mut spec.task_template.container_spec.labels);
    spec
}

fn conflict<T>(result: docker::Result<T>, hit: fn(&docker::Error) -> bool) -> Result<T> {
    match result {
        Ok(value) => Ok(value),
        Err(e) if hit(&e) => Err(ErrorKind::Conflict.into()),
        Err(e) => Err(e.into()),
    }
}

fn relay(name: &str, op: &str, result: docker::Result<Vec<String>>) -> Result<()> {
    for warning in conflict(result, docker::Error::is_version_conflict)? {
        tracing::warn!("service {name} {op} warning: {warning}");
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) struct Docker(Client);

impl Docker {
    pub(crate) async fn connect(socket: &str) -> Result<Self> {
        Ok(Self(Client::connect(socket).await?))
    }
}

impl Docker {
    pub(crate) async fn secret_inspect(&self, name: &str) -> Result<Option<Secret>> {
        Ok(self.0.secrets().inspect(name).await?)
    }

    pub(crate) async fn secret_create(
        &self,
        name: &str,
        mut labels: HashMap<String, String>,
        data: &[u8],
    ) -> Result<String> {
        super::label::managed(&mut labels);
        let result = self.0.secrets().create(name, labels, data).await;
        Ok(conflict(result, docker::Error::is_conflict)?.id)
    }

    pub(crate) async fn secret_list(&self, name_prefix: &str) -> Result<Vec<Secret>> {
        let filter = Filter::new().name(name_prefix);
        let mut secrets = self.0.secrets().list(&filter).await?;
        secrets.retain(|s| s.spec.name.starts_with(name_prefix));
        secrets.sort_by_key(|s| s.created_at);
        Ok(secrets)
    }

    pub(crate) async fn secret_find(&self, label: &str, value: &str) -> Result<Option<Secret>> {
        let filter = Filter::new().label(format!("{label}={value}"));
        Ok(self
            .0
            .secrets()
            .list(&filter)
            .await?
            .into_iter()
            .min_by_key(|s| s.created_at))
    }

    pub(crate) async fn secret_delete(&self, name: &str) -> Result<()> {
        self.0.secrets().delete(name).await?;
        Ok(())
    }

    pub(crate) async fn secret_update_labels(
        &self,
        name: &str,
        version: u64,
        mut labels: HashMap<String, String>,
    ) -> Result<()> {
        if self.secret_inspect(name).await?.is_none_or(|secret| {
            secret
                .spec
                .labels
                .get(insight_wire::MANAGED_LABEL)
                .is_none_or(|value| value != "true")
        }) {
            return Err(ErrorKind::Conflict.into());
        }
        super::label::managed(&mut labels);
        let result = self.0.secrets().update_labels(name, version, labels).await;
        conflict(result, docker::Error::is_version_conflict)
    }

    pub(crate) async fn service_inspect(&self, name: &str) -> Result<Option<Service>> {
        Ok(self.0.services().inspect(name).await?)
    }

    pub(crate) async fn service_status(&self, name: &str) -> Result<Option<(u64, u64)>> {
        Ok(self
            .0
            .services()
            .status(name)
            .await?
            .map(|status| (status.running, status.desired)))
    }

    pub(crate) async fn service_exists(&self, name: &str) -> Result<bool> {
        Ok(self.service_inspect(name).await?.is_some())
    }

    pub(crate) async fn service_create(
        &self,
        spec: &ServiceSpec,
        auth: Option<&RegistryAuth>,
    ) -> Result<String> {
        let spec = managed_spec(spec);
        let result = self.0.services().create(&spec, auth).await;
        let response = conflict(result, docker::Error::is_conflict)?;
        for warning in response.warnings.into_iter().flatten() {
            if !warning.is_empty() {
                tracing::warn!("service {} create warning: {warning}", spec.name);
            }
        }
        Ok(response.id)
    }

    pub(crate) async fn service_create_unique(
        &self,
        spec: &ServiceSpec,
        auth: Option<&RegistryAuth>,
        exists: ErrorKind,
    ) -> Result<()> {
        match self.service_create(spec, auth).await {
            Ok(_) => Ok(()),
            Err(e) if e.is_conflict() => Err(exists.into()),
            Err(e) => Err(e),
        }
    }

    pub(crate) async fn service_list(&self, filter: &Filter) -> Result<Vec<Service>> {
        Ok(self.0.services().list(filter).await?)
    }

    pub(crate) async fn service_find(&self, filter: Filter, name: &str) -> Result<Option<Service>> {
        let services = self.service_list(&filter.name(name)).await?;
        Ok(services.into_iter().find(|s| s.spec.name == name))
    }

    pub(crate) async fn service_update(
        &self,
        name: &str,
        version: u64,
        spec: &ServiceSpec,
        auth: Option<&RegistryAuth>,
    ) -> Result<()> {
        if self.service_inspect(name).await?.is_none_or(|service| {
            service
                .spec
                .labels
                .get(insight_wire::MANAGED_LABEL)
                .is_none_or(|value| value != "true")
        }) {
            return Err(ErrorKind::Conflict.into());
        }
        let spec = managed_spec(spec);
        let result = self.0.services().update(name, version, &spec, auth).await;
        relay(name, "update", result)
    }

    pub(crate) async fn service_update_labels(
        &self,
        current: &Service,
        version: u64,
        auth: Option<&RegistryAuth>,
        patch: impl FnOnce(&mut HashMap<String, String>),
    ) -> Result<()> {
        let mut spec = current.spec.clone();
        patch(&mut spec.labels);
        self.service_update(&current.spec.name, version, &spec, auth)
            .await
    }

    pub(crate) async fn service_rollback(&self, name: &str, version: u64) -> Result<()> {
        relay(
            name,
            "rollback",
            self.0.services().rollback(name, version).await,
        )
    }

    pub(crate) async fn service_restart(&self, name: &str) -> Result<()> {
        relay(name, "restart", self.0.services().restart(name, None).await)
    }

    pub(crate) async fn service_delete(&self, name: &str) -> Result<bool> {
        Ok(self.0.services().delete(name).await?)
    }

    pub(crate) async fn task_list(&self, service: &str) -> Result<Vec<Task>> {
        let filter = Filter::new().add("service", service);
        Ok(self.0.tasks().list(&filter).await?)
    }

    pub(crate) async fn task_list_all(&self) -> Result<Vec<Task>> {
        Ok(self.0.tasks().list(&Filter::new()).await?)
    }

    pub(crate) async fn config_inspect(&self, name: &str) -> Result<Option<Config>> {
        Ok(self.0.configs().inspect(name).await?)
    }

    pub(crate) async fn config_create(
        &self,
        name: &str,
        mut labels: HashMap<String, String>,
        data: &[u8],
    ) -> Result<String> {
        super::label::managed(&mut labels);
        let result = self.0.configs().create(name, labels, data).await;
        Ok(conflict(result, docker::Error::is_conflict)?.id)
    }

    pub(crate) async fn config_list(&self, label: &str) -> Result<Vec<Config>> {
        let filter = Filter::new().label(label);
        Ok(self.0.configs().list(&filter).await?)
    }

    pub(crate) async fn config_update_labels(
        &self,
        name: &str,
        version: u64,
        mut labels: HashMap<String, String>,
    ) -> Result<()> {
        if self.config_inspect(name).await?.is_none_or(|config| {
            config
                .spec
                .labels
                .get(insight_wire::MANAGED_LABEL)
                .is_none_or(|value| value != "true")
        }) {
            return Err(ErrorKind::Conflict.into());
        }
        super::label::managed(&mut labels);
        let result = self.0.configs().update_labels(name, version, labels).await;
        conflict(result, |e| e.is_version_conflict() || e.is_not_found())
    }

    pub(crate) async fn config_delete(&self, name: &str) -> Result<()> {
        conflict(
            self.0.configs().delete(name).await,
            docker::Error::is_in_use,
        )
    }

    pub(crate) async fn network_inspect(&self, name: &str) -> Result<Option<Network>> {
        Ok(self.0.networks().inspect(name).await?)
    }

    pub(crate) async fn network_create(
        &self,
        name: &str,
        mut labels: HashMap<String, String>,
    ) -> Result<String> {
        super::label::managed(&mut labels);
        let result = self.0.networks().create(name, labels, false).await;
        let response = conflict(result, docker::Error::is_conflict)?;
        if !response.warning.is_empty() {
            tracing::warn!("network {name} create warning: {}", response.warning);
        }
        Ok(response.id)
    }

    pub(crate) async fn network_list(&self, label: &str) -> Result<Vec<Network>> {
        let filter = Filter::new().label(label);
        Ok(self.0.networks().list(&filter).await?)
    }

    pub(crate) async fn network_delete(&self, name: &str) -> Result<bool> {
        conflict(
            self.0.networks().delete(name).await,
            docker::Error::is_in_use,
        )
    }

    pub(crate) async fn node_list(&self) -> Result<Vec<Node>> {
        Ok(self.0.nodes().list(&Filter::new()).await?)
    }

    pub(crate) async fn node_inspect(&self, id: &str) -> Result<Option<Node>> {
        Ok(self.0.nodes().inspect(id).await?)
    }

    pub(crate) async fn node_update(&self, id: &str, version: u64, spec: &NodeSpec) -> Result<()> {
        let result = self.0.nodes().update(id, version, spec).await;
        conflict(result, docker::Error::is_version_conflict)
    }

    pub(crate) async fn node_delete(&self, id: &str) -> Result<()> {
        self.0.nodes().remove(id, true).await?;
        Ok(())
    }

    pub(crate) async fn system_info(&self) -> Result<SystemInfo> {
        Ok(self.0.system().info().await?)
    }

    pub(crate) async fn swarm_inspect(&self) -> Result<InspectSwarm> {
        Ok(self.0.system().swarm().await?)
    }
}
