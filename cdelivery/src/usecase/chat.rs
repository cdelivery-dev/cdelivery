use crate::concept;
use crate::config;
use crate::error::Result;
use crate::infra;

pub(crate) async fn history(
    docker: &infra::docker::Docker,
    api: &infra::insight::query::Api,
    timeline: &concept::message::Timeline,
    range: (jiff::Timestamp, jiff::Timestamp),
) -> Result<Vec<concept::message::Message>> {
    let messages = if infra::insight::server::exists(docker).await? {
        api.messages(
            range,
            config::AGENT_MESSAGE_RING.saturating_add(1),
            timeline.to_string(),
        )
        .await?
        .into_iter()
        .map(|message| {
            concept::message::Message::with_id(
                message.id,
                timeline.clone(),
                message.role,
                message.author,
                message.text,
                message.timestamp,
            )
        })
        .collect()
    } else {
        Vec::new()
    };
    Ok(messages)
}
