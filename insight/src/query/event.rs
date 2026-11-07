use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::Response;
use axum_extra::extract::Query;
use datafusion::prelude::{Expr, lit};
use serde::Deserialize;

use super::Page;
use crate::error::Result;
use crate::storage::store::Store;
use insight_schema::event;

#[derive(Deserialize)]
pub(crate) struct Filter {
    kind: Option<insight_wire::event::Kind>,
    subject: Option<String>,
}

impl Filter {
    fn expr(&self) -> Option<Expr> {
        let fields = [
            (event::KIND, self.kind.map(|kind| lit(kind.as_str()))),
            (event::SUBJECT, self.subject.as_deref().map(lit)),
        ];
        super::eq_all(fields).reduce(Expr::and)
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
        insight_wire::event::INDEX_ID,
        range,
        filter.expr(),
        page,
    )
    .await
}
