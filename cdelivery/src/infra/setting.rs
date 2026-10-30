use std::collections::HashMap;

use cipher::{Domain, MasterKey};

use crate::concept;
use crate::error::Result;

const DATA: &str = "dev.cdelivery.setting.data";

fn decode(labels: &HashMap<String, String>, key: &MasterKey) -> Result<concept::setting::Settings> {
    match labels.get(DATA) {
        Some(data) => Ok(key.cipher_for(Domain::Setting).open(data)?),
        None => Ok(concept::setting::Settings::default()),
    }
}

pub(crate) async fn load(
    docker: &super::docker::Docker,
    key: &MasterKey,
) -> Result<concept::versioned::Versioned<concept::setting::Settings>> {
    let service = super::instance::require_service(docker).await?;
    Ok(concept::versioned::Versioned::new(
        service.version.index,
        decode(&service.spec.labels, key)?,
    ))
}

pub(crate) async fn save(
    docker: &super::docker::Docker,
    key: &MasterKey,
    settings: &concept::versioned::Versioned<concept::setting::Settings>,
) -> Result<()> {
    if !settings.changed() {
        return Ok(());
    }
    let service = super::instance::require_service(docker).await?;
    let data = key.cipher_for(Domain::Setting).seal(settings.get())?;
    docker
        .service_update_labels(&service, settings.version(), None, |labels| {
            labels.insert(DATA.to_owned(), data);
        })
        .await
}
