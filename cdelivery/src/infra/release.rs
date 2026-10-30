use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Deserialize;

use crate::{concept, config, error};

pub(crate) struct Catalog {
    client: reqwest::Client,
    base_url: url::Url,
    snapshot: arc_swap::ArcSwapOption<concept::release::Snapshot>,
}

#[derive(Deserialize)]
struct Raw {
    image: String,
    changelog: String,
    #[serde(default)]
    important: bool,
}

impl Catalog {
    pub(crate) fn new(base_url: url::Url) -> error::Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(config::RELEASE_TIMEOUT.unsigned_abs())
                .build()?,
            base_url,
            snapshot: arc_swap::ArcSwapOption::empty(),
        })
    }

    pub(crate) async fn refresh(&self, docker: &super::docker::Docker) -> error::Result<()> {
        let service = super::instance::require_service(docker).await?;
        let image: concept::image::Reference =
            service.spec.task_template.container_spec.image.parse()?;
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .map_err(|()| error::ErrorKind::ReleaseUrlInvalid)?
            .pop_if_empty()
            .push("v1")
            .extend(image.prefix().split('/'))
            .push("latest.json");
        let mut query = vec![("client_version", env!("CARGO_PKG_VERSION").to_owned())];
        for (key, name) in [
            ("cdelivery_agent_version", config::VOLUME_AGENT_SERVICE_NAME),
            ("pingress_version", config::PINGRESS_SERVICE_NAME),
            ("insight_version", config::INSIGHT_SERVICE_NAME),
            ("insight_agent_version", config::INSIGHT_AGENT_SERVICE_NAME),
        ] {
            if let Some(service) = docker.service_inspect(name).await? {
                let image: concept::image::Reference =
                    service.spec.task_template.container_spec.image.parse()?;
                match concept::release::parse_version(image.tag()) {
                    Ok(version) => query.push((key, version.to_string())),
                    Err(error) => {
                        tracing::warn!(error = ?error, "release {key} current image tag omitted");
                    }
                }
            }
        }
        let mut response = self
            .client
            .get(url)
            .query(&query)
            .send()
            .await?
            .error_for_status()?;
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if chunk.len() > config::RELEASE_BODY_LIMIT.saturating_sub(body.len()) {
                tracing::warn!("release catalog response too large");
                return Err(error::ErrorKind::ReleaseUnavailable.into());
            }
            body.extend_from_slice(&chunk);
        }
        let values = serde_json::from_slice::<BTreeMap<String, serde_json::Value>>(&body)?;
        let mut snapshot = BTreeMap::new();
        for (key, value) in values {
            let Ok(component) = key.parse() else {
                continue;
            };
            let release = decode(value).inspect_err(|error| {
                tracing::warn!(error = ?error, "release {key} entry invalid");
            })?;
            snapshot.insert(component, release);
        }
        self.snapshot.store(Some(Arc::new(snapshot)));
        Ok(())
    }

    pub(crate) fn list(&self) -> Option<Arc<concept::release::Snapshot>> {
        self.snapshot.load_full()
    }
}

fn decode(value: serde_json::Value) -> error::Result<concept::release::Release> {
    let raw: Raw = serde_json::from_value(value)?;
    concept::release::Release::try_new(&raw.image, raw.changelog, raw.important)
}
