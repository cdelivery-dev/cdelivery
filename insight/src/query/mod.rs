use axum::response::Response;
use datafusion::prelude::{Expr, col};
use serde::Deserialize;

use crate::error::{ErrorKind, Result};
use crate::storage::store::Store;

pub(crate) mod access;
pub(crate) mod cpu;
pub(crate) mod event;
pub(crate) mod log;
pub(crate) mod message;
pub(crate) mod metric;
pub(crate) mod overview;

fn default_limit() -> u16 {
    100
}

const MAX_LIMIT: u16 = 1000;

fn eq_all<const N: usize>(fields: [(&str, Option<Expr>); N]) -> impl Iterator<Item = Expr> {
    fields
        .into_iter()
        .filter_map(|(column, value)| Some(col(column).eq(value?)))
}

#[derive(Deserialize)]
pub(crate) struct Page {
    #[serde(default)]
    skip: u32,
    #[serde(default = "default_limit")]
    limit: u16,
}

pub(crate) async fn search(
    store: &Store,
    index_id: &str,
    range: (i64, i64),
    filter: Option<Expr>,
    page: Page,
) -> Result<Response> {
    if page.limit == 0 || page.limit > MAX_LIMIT {
        return Err(
            ErrorKind::BadRequest(format!("limit must be between 1 and {MAX_LIMIT}")).into(),
        );
    }
    let batches = store
        .search(
            index_id,
            range,
            filter,
            page.skip as usize,
            Some(usize::from(page.limit)),
            |batches: &mut Vec<_>, batch| {
                batches.push(batch);
                Ok(())
            },
        )
        .await?;
    crate::http::batches_to_json_response(&batches)
}
