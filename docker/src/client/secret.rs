use std::collections::HashMap;

use super::{Client, MAX_SECRET_DATA_SIZE, encode_immutable_data, filter_query, to_json_body};
use crate::error::Result;
use crate::types::{Filter, IdResponse, Secret, SecretSpec};

#[derive(Debug)]
pub struct Secrets<'a>(pub(crate) &'a Client);

impl Secrets<'_> {
    pub async fn inspect(&self, name: &str) -> Result<Option<Secret>> {
        self.0.get_opt(&format!("/secrets/{name}"), &[]).await
    }

    pub async fn list(&self, filter: &Filter) -> Result<Vec<Secret>> {
        self.0.list("/secrets", &filter_query(filter)).await
    }

    pub async fn create(
        &self,
        name: &str,
        labels: HashMap<String, String>,
        data: &[u8],
    ) -> Result<IdResponse> {
        let spec = SecretSpec {
            name: name.to_owned(),
            labels,
            data: Some(encode_immutable_data(data, MAX_SECRET_DATA_SIZE)?),
        };
        self.0
            .post_json("/secrets/create", &[], Some(to_json_body(&spec)), None)
            .await
    }

    pub async fn update_labels(
        &self,
        name: &str,
        version: u64,
        labels: HashMap<String, String>,
    ) -> Result<()> {
        let spec = SecretSpec {
            name: name.to_owned(),
            labels,
            data: None,
        };
        self.0
            .post_discard(
                &format!("/secrets/{name}/update"),
                &[("version", version.to_string())],
                Some(to_json_body(&spec)),
                None,
            )
            .await
    }

    pub async fn delete(&self, name: &str) -> Result<()> {
        self.0
            .delete_idempotent(&format!("/secrets/{name}"))
            .await?;
        Ok(())
    }
}
