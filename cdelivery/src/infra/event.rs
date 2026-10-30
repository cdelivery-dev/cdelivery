use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::LazyLock;

use docker::types::{Filter, Node, NodeState, Service, Task, TaskState};
use insight_wire::event::{Condition, Event, Kind};
use jiff::{SignedDuration, Timestamp};
use parking_lot::Mutex;

use crate::concept;
use crate::config;
use crate::error::Result;
use crate::locale;

tokio::task_local! {
    pub(crate) static ACTOR: concept::actor::Actor;
}

pub(crate) static HUB: LazyLock<Hub> = LazyLock::new(Hub::default);
pub(crate) static DUTY_RAW: LazyLock<DutyQueue> = LazyLock::new(DutyQueue::default);
pub(crate) static DUTY_ANALYSIS: LazyLock<DutyQueue> = LazyLock::new(DutyQueue::default);

#[derive(Default)]
pub(crate) struct DutyQueue {
    state: Mutex<DutyState>,
    wake: tokio::sync::Notify,
}

#[derive(Default)]
struct DutyState {
    events: VecDeque<Event>,
    dropped: usize,
}

impl DutyQueue {
    pub(crate) fn push(&self, records: impl IntoIterator<Item = Event>) {
        let mut state = self.state.lock();
        for record in records {
            if record.kind == Kind::Patrol && !state.events.is_empty() {
                continue;
            }
            if state.events.len() >= config::AGENT_DUTY_QUEUE {
                state.events.pop_front();
                state.dropped = state.dropped.saturating_add(1);
            }
            state.events.push_back(record);
        }
        self.wake.notify_one();
    }

    pub(crate) fn drain(&self) -> (Vec<Event>, usize) {
        let mut state = self.state.lock();
        let dropped = std::mem::take(&mut state.dropped);
        (state.events.drain(..).collect(), dropped)
    }

    pub(crate) fn wake(&self) -> &tokio::sync::Notify {
        &self.wake
    }
}

fn agent_marked() -> bool {
    ACTOR
        .try_with(|actor| actor.origin() == concept::actor::Origin::Agent)
        .unwrap_or(false)
}

#[derive(Default)]
pub(crate) struct Hub {
    conditions: Mutex<HashMap<(Kind, String), Recorded>>,
}

#[derive(Debug)]
struct Recorded {
    condition: Condition,
    announced: bool,
}

fn actor() -> String {
    ACTOR
        .try_with(ToString::to_string)
        .unwrap_or_else(|_| "system".to_owned())
}

pub(crate) fn truncate_bytes(s: &str, max: usize) -> &str {
    s.get(..s.floor_char_boundary(max)).unwrap_or_default()
}

impl Hub {
    pub(crate) fn raise(
        &self,
        kind: Kind,
        subject: &str,
        detail: &str,
        announce_after: Option<SignedDuration>,
    ) {
        let observed_at = Timestamp::now();
        let announce = {
            let mut conditions = self.conditions.lock();
            let state = conditions
                .entry((kind, subject.to_owned()))
                .or_insert_with(|| Recorded {
                    condition: Condition {
                        kind,
                        subject: subject.to_owned(),
                        since: observed_at,
                        detail: String::new(),
                        observed_at,
                    },
                    announced: false,
                });
            detail.clone_into(&mut state.condition.detail);
            state.condition.observed_at = observed_at;
            let announce = !state.announced
                && announce_after.is_some_and(|delay| {
                    delay <= SignedDuration::ZERO
                        || observed_at.duration_since(state.condition.since) >= delay
                });
            state.announced |= announce;
            announce
        };
        if announce {
            tracing::warn!("event {kind:?} raised for {subject}: {detail}");
            emit(kind, subject, detail);
        }
    }

    pub(crate) fn clear(&self, kind: Kind, subject: &str) -> bool {
        let removed = self.conditions.lock().remove(&(kind, subject.to_owned()));
        if removed.as_ref().is_some_and(|state| state.announced)
            && let Some(recovery) = concept::event::recovery(kind)
        {
            emit(recovery, subject, "");
        }
        removed.is_some()
    }

    pub(crate) fn dismiss(&self, kind: Kind, subject: &str) {
        self.conditions.lock().remove(&(kind, subject.to_owned()));
    }

    pub(crate) fn retain(&self, kind: Kind, keep: &HashSet<String>) {
        self.conditions
            .lock()
            .retain(|(k, subject), _| *k != kind || keep.contains(subject));
    }

    pub(crate) fn replace(&self, kind: Kind, errors: Vec<(String, String)>) {
        let keep = errors.iter().map(|(subject, _)| subject.clone()).collect();
        for (subject, detail) in errors {
            self.raise(kind, &subject, &detail, None);
        }
        self.retain(kind, &keep);
    }

    pub(crate) fn observe(&self) -> Vec<Condition> {
        let mut all: Vec<_> = self
            .conditions
            .lock()
            .values()
            .map(|state| state.condition.clone())
            .collect();
        all.sort_by(|a, b| {
            b.since
                .cmp(&a.since)
                .then_with(|| a.subject.cmp(&b.subject))
                .then_with(|| a.kind.as_str().cmp(b.kind.as_str()))
        });
        all
    }
}

pub(crate) fn emit(kind: Kind, subject: &str, detail: &str) {
    let record = Event {
        timestamp: Timestamp::now(),
        kind,
        subject: subject.to_owned(),
        actor: actor(),
        detail: (!detail.is_empty())
            .then(|| truncate_bytes(detail, config::EVENT_MESSAGE_MAX).to_owned()),
    };
    super::record(insight_wire::event::INDEX_ID, &record);
    if !agent_marked() {
        if concept::event::attention(record.kind) {
            DUTY_RAW.push([record.clone()]);
        }
        DUTY_ANALYSIS.push([record]);
    }
}

pub(crate) async fn sweep(docker: &super::docker::Docker, now: Timestamp) -> Vec<(String, String)> {
    let mut errors = Vec::new();
    if let Some(nodes) = queried(docker.node_list().await, "Docker nodes", &mut errors) {
        watch_nodes(nodes);
    }
    let services = queried(
        docker.service_list(&Filter::new()).await,
        "Docker services",
        &mut errors,
    );
    let tasks = queried(docker.task_list_all().await, "Docker tasks", &mut errors);
    if let (Some(services), Some(tasks)) = (services, tasks) {
        watch_tasks(services, &tasks, now, &mut errors);
    }
    errors
}

fn queried<T>(result: Result<T>, subject: &str, errors: &mut Vec<(String, String)>) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::warn!(error = ?error, "event sweep cannot list {subject}");
            errors.push((format!("/{subject}"), error.chain(locale::Locale::En)));
            None
        }
    }
}

fn watch_nodes(nodes: Vec<Node>) {
    let mut seen = HashSet::new();
    for node in nodes {
        match &node.status.state {
            NodeState::Ready => {
                HUB.clear(Kind::NodeOffline, &node.id);
            }
            state => {
                let message = format!("{}: {state:?}", node.description.hostname);
                HUB.raise(
                    Kind::NodeOffline,
                    &node.id,
                    &message,
                    Some(SignedDuration::ZERO),
                );
            }
        }
        seen.insert(node.id);
    }
    HUB.retain(Kind::NodeOffline, &seen);
}

fn watch_tasks(
    services: Vec<Service>,
    tasks: &[Task],
    now: Timestamp,
    errors: &mut Vec<(String, String)>,
) {
    let platform = [
        config::SERVICE_NAME,
        config::VOLUME_AGENT_SERVICE_NAME,
        config::PINGRESS_SERVICE_NAME,
        config::INSIGHT_SERVICE_NAME,
        config::INSIGHT_AGENT_SERVICE_NAME,
    ];
    let mut tasks_by_service: HashMap<&str, Vec<&Task>> = HashMap::new();
    for task in tasks {
        tasks_by_service
            .entry(&task.service_id)
            .or_default()
            .push(task);
    }
    let mut seen = HashSet::new();
    for service in services.into_iter().filter(|service| {
        platform.contains(&service.spec.name.as_str())
            || service
                .spec
                .labels
                .get(super::app::unit::LABEL_KIND)
                .is_some_and(|kind| kind == "App")
    }) {
        let tasks = tasks_by_service
            .remove(service.id.as_str())
            .unwrap_or_default();
        observe_service(&service, &tasks, now, errors);
        seen.insert(service.spec.name);
    }
    HUB.retain(Kind::RuntimeIssue, &seen);
}

fn observe_service(
    service: &Service,
    tasks: &[&Task],
    now: Timestamp,
    errors: &mut Vec<(String, String)>,
) {
    let name = &service.spec.name;
    let mut evidence: Vec<_> = tasks
        .iter()
        .filter_map(|task| task_diagnostic(task, now).map(|detail| (task.status.timestamp, detail)))
        .collect();
    evidence.extend(super::app::runtime::update_diagnostic(service));
    let announce_after = if evidence.is_empty() {
        config::INSTANCE_ALERT_DELAY
    } else {
        SignedDuration::ZERO
    };
    let ready = if let Some(status) = &service.status {
        let instances = super::app::runtime::instances(tasks, status.desired);
        if !instances.ready() {
            evidence.push((
                now,
                format!(
                    "Active instances running {}, desired {}",
                    instances.running(),
                    status.desired
                ),
            ));
        }
        instances.ready()
    } else {
        let detail = format!("Service {name} response is missing instance counts");
        tracing::warn!("{detail}");
        errors.push((format!("{name}/status"), detail));
        false
    };
    evidence.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let message = evidence
        .into_iter()
        .map(|(_, detail)| detail)
        .collect::<Vec<_>>()
        .join("\n");
    if message.is_empty() {
        if ready {
            HUB.clear(Kind::RuntimeIssue, name);
        }
    } else {
        HUB.raise(Kind::RuntimeIssue, name, &message, Some(announce_after));
    }
}

fn task_diagnostic(task: &Task, now: Timestamp) -> Option<String> {
    let status = &task.status;
    let current = super::app::runtime::demanded(task)
        && (status.error().is_some()
            || matches!(task.desired_state, TaskState::Unknown(_))
            || matches!(status.state, TaskState::Unknown(_)));
    let recent = matches!(status.state, TaskState::Failed | TaskState::Rejected)
        && now.duration_since(status.timestamp) <= config::TASK_CRASH_WINDOW;
    (current || recent).then(|| super::app::runtime::task_diagnostic(task))
}
