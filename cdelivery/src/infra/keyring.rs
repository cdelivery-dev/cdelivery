use std::collections::HashMap;

use cipher::{Domain, MasterKey};
use docker::types::{ServiceFileTarget, ServiceSecret, ServiceSpec};
use zeroize::Zeroizing;

use crate::config::{self, SECRET_NAME, SERVICE_NAME};
use crate::error::Result;

pub(crate) async fn ensure_subkey_secret(
    docker: &super::docker::Docker,
    key: &MasterKey,
    name: &str,
    domain: Domain,
) -> Result<String> {
    if let Some(existing) = docker.secret_inspect(name).await? {
        return Ok(existing.id);
    }
    let subkey = key.subkey(domain);
    match docker.secret_create(name, HashMap::new(), &*subkey).await {
        Ok(id) => Ok(id),
        Err(e) if e.is_conflict() => {
            tracing::warn!("key secret {name} already exists; reusing");
            docker.secret_inspect(name).await?.map(|s| s.id).ok_or(e)
        }
        Err(e) => Err(e),
    }
}

pub(crate) fn load() -> Result<Option<MasterKey>> {
    match std::fs::read(config::secret_path()) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&Zeroizing::new(bytes))?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub(crate) async fn create(docker: &super::docker::Docker, key: &MasterKey) -> Result<String> {
    let data = Zeroizing::new(serde_json::to_vec(key)?);
    docker
        .secret_create(SECRET_NAME, HashMap::new(), &data)
        .await
}

pub(crate) async fn find_id(docker: &super::docker::Docker) -> Result<Option<String>> {
    Ok(docker.secret_inspect(SECRET_NAME).await?.map(|s| s.id))
}

pub(crate) async fn observe(docker: &super::docker::Docker) -> Result<Option<(String, bool)>> {
    let service = super::instance::verified_service(docker).await?;
    Ok(find_id(docker)
        .await?
        .map(|id| (id, has_secret_ref(&service.spec))))
}

pub(crate) async fn attach(docker: &super::docker::Docker, secret_id: &str) -> Result<()> {
    let service = super::instance::require_service(docker).await?;
    let mut spec = service.spec.clone();
    if with_secret_ref(&mut spec, secret_id) {
        super::instance::prepare_replacement(&mut spec);
        docker
            .service_update(SERVICE_NAME, service.version.index, &spec, None)
            .await?;
    }
    Ok(())
}

fn with_secret_ref(spec: &mut ServiceSpec, secret_id: &str) -> bool {
    if has_secret_ref(spec) {
        return false;
    }
    let secrets = &mut spec.task_template.container_spec.secrets;
    secrets.push(ServiceSecret {
        file: ServiceFileTarget {
            name: SECRET_NAME.to_owned(),
            ..ServiceFileTarget::default()
        },
        secret_id: secret_id.to_owned(),
        secret_name: SECRET_NAME.to_owned(),
    });
    true
}

fn has_secret_ref(spec: &ServiceSpec) -> bool {
    spec.task_template
        .container_spec
        .secrets
        .iter()
        .any(|secret| secret.secret_name == SECRET_NAME)
}
