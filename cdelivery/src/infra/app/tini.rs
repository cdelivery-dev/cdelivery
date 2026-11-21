use std::collections::HashMap;

use base64::Engine;
use docker::types::ServiceConfig;

use crate::{concept, error};

const PREFIX: &str = "cdelivery-tini-";

pub(super) fn internal_config(config: &ServiceConfig) -> bool {
    config.config_name.starts_with(PREFIX)
        && [
            concept::tini::AMD64_PATH,
            concept::tini::ARM64_PATH,
            concept::tini::INIT_PATH,
        ]
        .contains(&config.file.name.as_str())
}

pub(crate) async fn ensure(
    docker: &super::super::docker::Docker,
) -> error::Result<Vec<concept::app::XConfigRef>> {
    let version = concept::tini::VERSION.trim();
    let mut refs = Vec::with_capacity(3);
    for (arch, target) in [
        ("amd64", concept::tini::AMD64_PATH),
        ("arm64", concept::tini::ARM64_PATH),
        ("selector", concept::tini::INIT_PATH),
    ] {
        let name = format!("{PREFIX}{version}-{arch}");
        let data = if arch == "selector" {
            include_bytes!("../../../../tini/selector.tmpl").to_vec()
        } else {
            tokio::fs::read(format!("/usr/share/cdelivery/tini/{arch}")).await?
        };
        let id = if let Some(existing) = docker.config_inspect(&name).await? {
            verify(&existing, &data, arch == "selector")?;
            existing.id
        } else {
            let result = if arch == "selector" {
                docker
                    .config_create_template(&name, HashMap::new(), &data)
                    .await
            } else {
                docker.config_create(&name, HashMap::new(), &data).await
            };
            match result {
                Ok(id) => id,
                Err(error) if error.is_conflict() => {
                    let existing = docker
                        .config_inspect(&name)
                        .await?
                        .ok_or(error::ErrorKind::TiniUnavailable)?;
                    verify(&existing, &data, arch == "selector")?;
                    existing.id
                }
                Err(error) => return Err(error),
            }
        };
        refs.push(concept::app::XConfigRef::new(target.to_owned(), name, id));
    }
    Ok(refs)
}

fn verify(config: &docker::types::Config, data: &[u8], template: bool) -> error::Result<()> {
    let stored = config
        .spec
        .data
        .as_ref()
        .ok_or(error::ErrorKind::TiniUnavailable)?;
    if base64::engine::general_purpose::STANDARD.decode(stored)? != data
        || config
            .spec
            .templating
            .as_ref()
            .map(|driver| driver.name.as_str())
            != template.then_some("golang")
    {
        return Err(error::ErrorKind::TiniConfigConflict.into());
    }
    Ok(())
}
