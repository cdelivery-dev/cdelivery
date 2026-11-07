use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::LazyLock;

use parking_lot::Mutex;

use crate::concept;
use crate::config;

pub(crate) static MESSAGES: LazyLock<Hub> = LazyLock::new(Hub::default);

#[derive(Default)]
pub(crate) struct Hub {
    state: Mutex<State>,
    wake: tokio::sync::Notify,
}

#[derive(Default)]
struct State {
    rings: HashMap<concept::message::Timeline, VecDeque<concept::message::Message>>,
    dirty: HashSet<concept::message::Timeline>,
    running: HashSet<concept::message::Timeline>,
}

impl Hub {
    pub(crate) fn emit(&self, message: &concept::message::Message) {
        let key = message.timeline().clone();
        let cutoff = jiff::Timestamp::now()
            .saturating_sub(config::AGENT_CHAT_WINDOW)
            .unwrap_or(jiff::Timestamp::MIN);
        let mut state = self.state.lock();
        state.rings.retain(|_, ring| {
            ring.back()
                .is_some_and(|stored| stored.timestamp() >= cutoff)
        });
        let ring = state.rings.entry(key.clone()).or_default();
        if ring.iter().any(|stored| stored.id() == message.id()) {
            return;
        }
        while ring.len() >= usize::from(config::AGENT_MESSAGE_RING) {
            ring.pop_front();
        }
        ring.push_back(message.clone());
        if matches!(message.role(), insight_wire::message::Role::User) {
            state.dirty.insert(key);
        }
        drop(state);

        super::record(
            insight_wire::message::INDEX_ID,
            insight_wire::message::Message {
                id: message.id(),
                timestamp: message.timestamp(),
                timeline: message.timeline().to_string(),
                role: message.role(),
                author: message.author().clone(),
                text: message.text().clone(),
            },
        );
        if matches!(message.role(), insight_wire::message::Role::User) {
            self.wake.notify_one();
        }
    }

    pub(crate) fn take_recent(
        &self,
        timeline: &concept::message::Timeline,
    ) -> Vec<concept::message::Message> {
        let mut state = self.state.lock();
        state.dirty.remove(timeline);
        state
            .rings
            .get(timeline)
            .map(|ring| ring.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub(crate) fn claim(&self, limit: usize) -> Vec<concept::message::Timeline> {
        let State { dirty, running, .. } = &mut *self.state.lock();
        let claimed: Vec<_> = dirty
            .extract_if(|timeline| !running.contains(timeline))
            .take(limit)
            .collect();
        running.extend(claimed.iter().cloned());
        claimed
    }

    pub(crate) fn release(&self, timeline: &concept::message::Timeline) {
        let mut state = self.state.lock();
        state.running.remove(timeline);
        if state.dirty.contains(timeline) {
            self.wake.notify_one();
        }
    }

    pub(crate) fn wake(&self) -> &tokio::sync::Notify {
        &self.wake
    }
}
