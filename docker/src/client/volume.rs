use std::collections::HashMap;

use http::Method;
use serde::Deserialize;

use super::Client;
use crate::error::Result;
use crate::types::Volume;

#[derive(Debug)]
pub struct Volumes<'a>(pub(crate) &'a Client);

impl Volumes<'_> {
    pub async fn inspect(&self, name: &str) -> Result<Option<Volume>> {
        self.0.get_opt(&format!("/volumes/{name}"), &[]).await
    }

    pub async fn list(&self, sizes: bool) -> Result<Vec<Volume>> {
        let mut usage = HashMap::new();
        if sizes {
            for volume in self
                .list_at("/system/df", &[("type", "volume".to_owned())])
                .await?
            {
                usage.insert(volume.name, volume.usage_data);
            }
        }
        let mut volumes = self.list_at("/volumes", &[]).await?;
        for volume in &mut volumes {
            volume.usage_data = usage.remove(&volume.name).flatten();
        }
        Ok(volumes)
    }

    pub async fn create(&self, name: &str) -> Result<()> {
        let body = serde_json::json!({"Name": name, "Driver": "local"}).to_string();
        self.0
            .post_discard("/volumes/create", &[], Some(body), None)
            .await
    }

    pub async fn delete(&self, name: &str) -> Result<()> {
        self.0
            .delete_not_found_ok(&format!("/volumes/{name}"), &[])
            .await
    }

    async fn list_at(&self, path: &str, query: &[(&str, String)]) -> Result<Vec<Volume>> {
        let bytes = self.0.request(Method::GET, path, query, None, None).await?;
        let body: VolumeList = serde_json::from_slice(&bytes)?;
        Ok(body.volumes.unwrap_or_default())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct VolumeList {
    volumes: Option<Vec<Volume>>,
}
