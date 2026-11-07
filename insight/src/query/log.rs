use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::Response;
use axum_extra::extract::Query;
use datafusion::prelude::{Expr, col, contains, lit, lower};
use serde::Deserialize;

use super::Page;
use crate::error::{ErrorKind, Result};
use crate::storage::store::Store;
use insight_schema::{CONTAINER_ID, NODE_ID, log, log_index, valid_service_id};

#[derive(Deserialize)]
pub(crate) struct Filter {
    #[serde(default)]
    words: Vec<String>,
    source: Option<String>,
    node_id: Option<String>,
    container_id: Option<String>,
}

impl Filter {
    fn expr(&self) -> Option<Expr> {
        let fields = [
            (log::SOURCE, self.source.as_deref().map(lit)),
            (NODE_ID, self.node_id.as_deref().map(lit)),
            (CONTAINER_ID, self.container_id.as_deref().map(lit)),
        ];
        let text = lower(col(log::LOG));
        super::eq_all(fields)
            .chain(
                self.words
                    .iter()
                    .map(|word| contains(text.clone(), lit(word.to_lowercase()))),
            )
            .reduce(Expr::and)
    }
}

pub(crate) async fn list(
    State(store): State<Arc<Store>>,
    Path((service_id, start_time, end_time)): Path<(String, i64, i64)>,
    Query(page): Query<Page>,
    Query(filter): Query<Filter>,
) -> Result<Response> {
    if !valid_service_id(&service_id) {
        return Err(ErrorKind::BadRequest(format!("{service_id} is not a service id")).into());
    }
    super::search(
        &store,
        &log_index(&service_id),
        (start_time, end_time),
        filter.expr(),
        page,
    )
    .await
}
