use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use cdelivery_wire::postgres::PlanetScaleSku;
use serde::Deserialize;

use crate::config;
use crate::error::{ErrorKind, Result};

static SIZES: LazyLock<arc_swap::ArcSwapOption<Vec<PlanetScaleSku>>> =
    LazyLock::new(arc_swap::ArcSwapOption::empty);

pub(crate) async fn refresh() -> Result<()> {
    #[derive(Deserialize)]
    struct Catalog {
        sizes: Vec<PlanetScaleSku>,
    }

    let mut url = config::releases_url()?;
    url.path_segments_mut()
        .map_err(|()| ErrorKind::ReleaseUrlInvalid)?
        .pop_if_empty()
        .extend(["v1", "planetscale-sizes.json"]);
    let mut response = reqwest::Client::builder()
        .timeout(config::RELEASE_TIMEOUT.unsigned_abs())
        .build()?
        .get(url)
        .send()
        .await?
        .error_for_status()?;
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if chunk.len() > config::RELEASE_BODY_LIMIT.saturating_sub(body.len()) {
            return Err(
                ErrorKind::InvalidBody("PlanetScale size catalog is too large".to_owned()).into(),
            );
        }
        body.extend_from_slice(&chunk);
    }
    let catalog: Catalog = serde_json::from_slice(&body)?;
    let mut names = HashSet::new();
    for sku in &catalog.sizes {
        if [&sku.name, &sku.size, &sku.provider, &sku.architecture]
            .iter()
            .any(|value| value.trim().is_empty())
            || sku.size.len() > 128
            || sku.memory_bytes == 0
            || !names.insert(&sku.name)
        {
            return Err(ErrorKind::InvalidBody(
                "PlanetScale size catalog contains an invalid or duplicate SKU".to_owned(),
            )
            .into());
        }
    }
    SIZES.store(Some(Arc::new(catalog.sizes)));
    Ok(())
}

pub(crate) fn list() -> Option<Vec<PlanetScaleSku>> {
    SIZES.load().as_deref().cloned()
}
