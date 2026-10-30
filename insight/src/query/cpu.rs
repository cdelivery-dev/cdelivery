use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use insight_wire::cpu::{MAX_SERVICES, Request, Response};

use crate::error::{ErrorKind, Result};
use crate::storage::store::Store;

pub(crate) async fn latest(
    State(store): State<Arc<Store>>,
    Json(mut request): Json<Request>,
) -> Result<Json<Response>> {
    if !(1..=MAX_SERVICES).contains(&request.service_ids.len()) {
        return Err(ErrorKind::BadRequest(format!(
            "service_ids must contain between 1 and {MAX_SERVICES} IDs"
        ))
        .into());
    }
    if request
        .service_ids
        .iter()
        .any(|id| !insight_schema::valid_service_id(id))
    {
        return Err(ErrorKind::BadRequest(
            "service_ids must contain 1 to 64 ASCII alphanumeric characters".to_owned(),
        )
        .into());
    }
    request.service_ids.sort_unstable();
    request.service_ids.dedup();
    Ok(Json(store.cpu.select(&request.service_ids)))
}
