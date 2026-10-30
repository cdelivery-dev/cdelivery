use std::sync::Arc;

use axum::response::{IntoResponse, Response};
use axum::{
    Router,
    routing::{get, post},
};
use datafusion::arrow::array::RecordBatch;
use datafusion::arrow::json::WriterBuilder;
use datafusion::arrow::json::writer::JsonArray;

use crate::error::{Error, ErrorKind, Result};
use crate::storage::store::Store;

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match self.kind() {
            ErrorKind::BadRequest(_) => http::StatusCode::BAD_REQUEST,
            _ => http::StatusCode::INTERNAL_SERVER_ERROR,
        };
        if status.is_server_error() {
            tracing::error!(error = ?self, "query request failed");
        }
        (status, self.to_string()).into_response()
    }
}

pub(crate) fn batches_to_json_response(batches: &[RecordBatch]) -> Result<Response> {
    let buf = Vec::new();
    let mut writer = WriterBuilder::new()
        .with_timestamp_format("%Y-%m-%dT%H:%M:%S%.fZ".to_owned())
        .build::<_, JsonArray>(buf);
    for batch in batches {
        writer.write(batch)?;
    }
    writer.finish()?;
    Ok((
        [(
            http::header::CONTENT_TYPE,
            http::header::HeaderValue::from_static("application/json"),
        )],
        writer.into_inner(),
    )
        .into_response())
}

pub async fn serve(
    store: Arc<Store>,
    listener: tokio::net::TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<()> {
    let router = Router::new()
        .route("/health", get(|| async { "OK" }))
        .route("/metrics/cpu/latest", post(crate::query::cpu::latest))
        .route(
            "/logs/{service_id}/{start_time}/{end_time}",
            get(crate::query::log::list),
        )
        .route(
            "/metrics/host/{start_time}/{end_time}",
            get(crate::query::metric::host),
        )
        .route(
            "/metrics/filesystem/{start_time}/{end_time}",
            get(crate::query::metric::filesystem),
        )
        .route(
            "/metrics/node-latest/{start_time}/{end_time}",
            get(crate::query::metric::node_latest),
        )
        .route(
            "/metrics/overview/{start_time}/{end_time}",
            get(crate::query::overview::get),
        )
        .route(
            "/metrics/container/{start_time}/{end_time}",
            get(crate::query::metric::container),
        )
        .route(
            "/metrics/container/instances/{start_time}/{end_time}",
            get(crate::query::metric::instances),
        )
        .route(
            "/pingress/access/{start_time}/{end_time}",
            get(crate::query::access::list),
        )
        .route(
            "/pingress/access/stats/{start_time}/{end_time}",
            get(crate::query::access::stats),
        )
        .route(
            "/pingress/access/stats/status/{start_time}/{end_time}",
            get(crate::query::access::status_stats),
        )
        .route(
            "/events/{start_time}/{end_time}",
            get(crate::query::event::list),
        )
        .route(
            "/messages/{start_time}/{end_time}",
            get(crate::query::message::list),
        )
        .with_state(store);
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown)
        .await?;
    Ok(())
}
