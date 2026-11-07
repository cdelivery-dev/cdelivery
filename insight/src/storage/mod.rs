pub mod meta;
pub mod store;

use std::sync::{Arc, LazyLock};

use datafusion::arrow::compute::SortOptions;
use datafusion::arrow::datatypes::Schema;
use datafusion::logical_expr::SortExpr;
use datafusion::prelude::col;
use insight_schema::{ID, IndexKind, TIMESTAMP};
use parquet::file::metadata::SortingColumn;

use crate::error::{ErrorKind, Result};

const SORT_TO_INDICES_OPTIONS: SortOptions = SortOptions {
    descending: true,
    nulls_first: true,
};
static SORT_FIELD: LazyLock<SortExpr> = LazyLock::new(|| col(TIMESTAMP).sort(false, true));
const PARQUET_WRITER_SORT_FIELD: SortingColumn = SortingColumn {
    column_idx: 0,
    descending: true,
    nulls_first: true,
};

fn current_schema(index_id: &str) -> Result<Arc<Schema>> {
    IndexKind::of(index_id)
        .map(IndexKind::schema)
        .ok_or_else(|| ErrorKind::BadRequest(format!("unsupported index {index_id}")).into())
}

fn query_sort(index_id: &str) -> Vec<SortExpr> {
    let ties: &[&str] = match IndexKind::of(index_id) {
        Some(IndexKind::Message) => &[ID],
        _ => &[],
    };
    std::iter::once(TIMESTAMP)
        .chain(ties.iter().copied())
        .map(|name| col(name).sort(false, true))
        .collect()
}
