use std::collections::{HashSet, VecDeque};

use crate::concept;

pub(crate) struct Dedup {
    seen: HashSet<String>,
    order: VecDeque<String>,
    cap: usize,
}

impl Dedup {
    pub(crate) fn new(cap: usize) -> Self {
        Self {
            seen: HashSet::new(),
            order: VecDeque::new(),
            cap,
        }
    }

    pub(crate) async fn admit(
        &mut self,
        inbound: &tokio::sync::mpsc::Sender<concept::message::Inbound>,
        message: concept::message::Inbound,
        platform: &str,
    ) -> std::io::Result<()> {
        let event_id = message.event_id().clone();
        if self.seen.contains(&event_id) {
            tracing::debug!("{platform} event {event_id} duplicate, dropped");
            return Ok(());
        }
        inbound.send(message).await.map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "im inbound queue closed")
        })?;
        self.insert(&event_id);
        tracing::info!("{platform} event {event_id} accepted");
        Ok(())
    }

    fn insert(&mut self, id: &str) {
        if !self.seen.insert(id.to_owned()) {
            return;
        }
        self.order.push_back(id.to_owned());
        if self.order.len() > self.cap
            && let Some(oldest) = self.order.pop_front()
        {
            self.seen.remove(&oldest);
        }
    }
}
