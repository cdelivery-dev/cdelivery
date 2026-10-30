use insight_wire::event::Kind;
use jiff::Timestamp;

use crate::error::Result;
use crate::infra;

pub(crate) fn conditions() -> Vec<insight_wire::event::Condition> {
    infra::event::HUB.observe()
}

pub(crate) async fn list(
    docker: &infra::docker::Docker,
    api: &infra::insight::query::Api,
    range: (Timestamp, Timestamp),
    skip: u32,
    limit: u16,
    kind: Option<Kind>,
    subject: Option<String>,
) -> Result<Vec<insight_wire::event::Event>> {
    super::insight::require_server(docker).await?;
    api.events(range, skip, limit, kind, subject.as_deref())
        .await
}
