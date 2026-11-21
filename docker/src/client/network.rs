use std::collections::HashMap;

use http::{Method, StatusCode};

use super::{Client, filter_query, to_json_body};
use crate::error::Result;
use crate::types::{Filter, Network, NetworkCreateResponse};

#[derive(Debug)]
pub struct Networks<'a>(pub(crate) &'a Client);

impl Networks<'_> {
    pub async fn create(
        &self,
        name: &str,
        labels: HashMap<String, String>,
        enable_ipv6: bool,
    ) -> Result<NetworkCreateResponse> {
        let body = serde_json::json!({
            "Name": name,
            "Driver": "overlay",
            "Scope": "swarm",
            "EnableIPv6": enable_ipv6,
            "Labels": labels,
        });
        self.0
            .post_json("/networks/create", &[], Some(to_json_body(&body)), None)
            .await
    }

    pub async fn inspect(&self, name: &str) -> Result<Option<Network>> {
        self.0.get_opt(&format!("/networks/{name}"), &[]).await
    }

    pub async fn list(&self, filter: &Filter) -> Result<Vec<Network>> {
        self.0.list("/networks", &filter_query(filter)).await
    }

    pub async fn delete(&self, name: &str) -> Result<bool> {
        match self
            .0
            .request(
                Method::DELETE,
                &format!("/networks/{name}"),
                &[],
                None,
                None,
            )
            .await
        {
            Ok(_) => Ok(true),
            Err(e) if e.is_not_found() || missing_network_500(&e) => Ok(false),
            Err(e) => Err(e),
        }
    }
}

fn missing_network_500(error: &crate::Error) -> bool {
    error.as_docker().is_some_and(|(code, message)| {
        code == StatusCode::INTERNAL_SERVER_ERROR
            && message
                .strip_prefix("network ")
                .and_then(|value| value.strip_suffix(" not found"))
                .is_some_and(|value| !value.is_empty())
    })
}
