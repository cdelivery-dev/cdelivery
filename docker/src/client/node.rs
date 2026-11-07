use super::{Client, filter_query, to_json_body};
use crate::error::Result;
use crate::types::{Filter, Node, NodeSpec};

#[derive(Debug)]
pub struct Nodes<'a>(pub(crate) &'a Client);

impl Nodes<'_> {
    pub async fn list(&self, filter: &Filter) -> Result<Vec<Node>> {
        self.0.list("/nodes", &filter_query(filter)).await
    }

    pub async fn inspect(&self, id: &str) -> Result<Option<Node>> {
        self.0.get_opt(&format!("/nodes/{id}"), &[]).await
    }

    pub async fn update(&self, id: &str, version: u64, spec: &NodeSpec) -> Result<()> {
        self.0
            .post_discard(
                &format!("/nodes/{id}/update"),
                &[("version", version.to_string())],
                Some(to_json_body(spec)),
                None,
            )
            .await
    }

    pub async fn remove(&self, id: &str, force: bool) -> Result<()> {
        self.0
            .delete_not_found_ok(&format!("/nodes/{id}"), &[("force", force.to_string())])
            .await
    }
}
