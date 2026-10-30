pub(crate) mod duty;
mod message;
mod prompt;
pub(crate) mod supervisor;
mod tool;
mod turn;

use jiff::Timestamp;

use crate::{concept, infra};

fn relay(
    sent: crate::error::Result<()>,
    timeline: concept::message::Timeline,
    author: &str,
    text: &str,
    what: &str,
) {
    match sent {
        Ok(()) => infra::message::MESSAGES.emit(&concept::message::Message::with_id(
            uuid::Uuid::new_v4(),
            timeline,
            insight_wire::message::Role::Agent,
            author.to_owned(),
            text.to_owned(),
            Timestamp::now(),
        )),
        Err(error) => tracing::warn!(error = ?error, "{what} failed"),
    }
}
