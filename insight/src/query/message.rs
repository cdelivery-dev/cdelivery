use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::Response;
use axum_extra::extract::Query;
use datafusion::prelude::{Expr, col, lit};
use serde::Deserialize;

use super::Page;
use crate::error::Result;
use crate::storage::store::Store;
use insight_schema::message;

#[derive(Deserialize)]
pub(crate) struct Filter {
    timeline: Option<String>,
}

impl Filter {
    fn expr(&self) -> Option<Expr> {
        self.timeline
            .as_ref()
            .map(|timeline| col(message::TIMELINE).eq(lit(timeline)))
    }
}

pub(crate) async fn list(
    State(store): State<Arc<Store>>,
    Path(range): Path<(i64, i64)>,
    Query(page): Query<Page>,
    Query(filter): Query<Filter>,
) -> Result<Response> {
    super::search(
        &store,
        insight_wire::message::INDEX_ID,
        range,
        filter.expr(),
        page,
    )
    .await
}
