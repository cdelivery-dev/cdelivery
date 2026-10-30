use super::{Client, filter_query};
use crate::error::Result;
use crate::types::{Filter, Task};

#[derive(Debug)]
pub struct Tasks<'a>(pub(crate) &'a Client);

impl Tasks<'_> {
    pub async fn list(&self, filter: &Filter) -> Result<Vec<Task>> {
        self.0.list("/tasks", &filter_query(filter)).await
    }
}
