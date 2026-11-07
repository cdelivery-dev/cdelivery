use super::{Client, filter_query};
use crate::error::Result;
use crate::types::{Container, ContainerDetail, Filter};

#[derive(Debug)]
pub struct Containers<'a>(pub(crate) &'a Client);

impl Containers<'_> {
    pub async fn list(
        &self,
        all: bool,
        tasks_only: bool,
        filter: &Filter,
    ) -> Result<Vec<Container>> {
        let mut filter = filter.clone();
        if tasks_only {
            filter = filter.add("is-task", "true");
        }
        let mut query = vec![("all", all.to_string())];
        query.extend(filter_query(&filter));
        self.0.list("/containers/json", &query).await
    }

    pub async fn inspect(&self, id: &str) -> Result<Option<ContainerDetail>> {
        self.0.get_opt(&format!("/containers/{id}/json"), &[]).await
    }
}
