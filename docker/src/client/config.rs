use std::collections::HashMap;

use super::{Client, MAX_CONFIG_DATA_SIZE, encode_immutable_data, filter_query, to_json_body};
use crate::error::Result;
use crate::types::{Config, ConfigSpec, Filter, IdResponse};

#[derive(Debug)]
pub struct Configs<'a>(pub(crate) &'a Client);

impl Configs<'_> {
    pub async fn inspect(&self, name: &str) -> Result<Option<Config>> {
        self.0.get_opt(&format!("/configs/{name}"), &[]).await
    }

    pub async fn list(&self, filter: &Filter) -> Result<Vec<Config>> {
        self.0.list("/configs", &filter_query(filter)).await
    }

    pub async fn create(
        &self,
        name: &str,
        labels: HashMap<String, String>,
        data: &[u8],
    ) -> Result<IdResponse> {
        let spec = ConfigSpec {
            name: name.to_owned(),
            labels,
            data: Some(encode_immutable_data(data, MAX_CONFIG_DATA_SIZE)?),
        };
        self.0
            .post_json("/configs/create", &[], Some(to_json_body(&spec)), None)
            .await
    }

    pub async fn update_labels(
        &self,
        name: &str,
        version: u64,
        labels: HashMap<String, String>,
    ) -> Result<()> {
        let spec = ConfigSpec {
            name: name.to_owned(),
            labels,
            data: None,
        };
        self.0
            .post_discard(
                &format!("/configs/{name}/update"),
                &[("version", version.to_string())],
                Some(to_json_body(&spec)),
                None,
            )
            .await
    }

    pub async fn delete(&self, name: &str) -> Result<()> {
        self.0
            .delete_idempotent(&format!("/configs/{name}"))
            .await?;
        Ok(())
    }
}
