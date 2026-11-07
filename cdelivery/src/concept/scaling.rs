use std::collections::{HashMap, VecDeque};

use getset::{CopyGetters, Getters};
use insight_wire::cpu::{ContainerSample, NodeSample, Response};
use jiff::{SignedDuration, Timestamp};

use crate::config;

#[derive(Debug, Getters, CopyGetters, macros_derive::Ctor)]
pub(crate) struct Observation {
    #[getset(get = "pub(crate)")]
    service_id: String,
    #[getset(get = "pub(crate)")]
    app: super::app::App,
    #[getset(get_copy = "pub(crate)")]
    version: u64,
    containers: Option<HashMap<String, String>>,
    occupied: HashMap<String, u64>,
    blocked: bool,
}

impl Observation {
    fn ready(&self) -> bool {
        self.containers
            .as_ref()
            .is_some_and(|ids| ids.len() as u64 == self.app.replicas() && !ids.is_empty())
    }
}

#[derive(Debug)]
pub(crate) enum Decision {
    Hold(Option<&'static str>),
    Scale { replicas: u64, utilization: f64 },
}

#[derive(Debug)]
struct AppState {
    version: u64,
    since: Option<Timestamp>,
    waiting: bool,
    low: Option<(Timestamp, u64)>,
}

#[derive(Debug, Default)]
pub(crate) struct Controller {
    apps: HashMap<String, AppState>,
    history: History,
}

#[derive(Debug, Default)]
struct History {
    samples: VecDeque<(Timestamp, Response)>,
    interval: u16,
}

impl Controller {
    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&str) -> bool) {
        self.apps.retain(|name, _| keep(name));
    }

    pub(crate) fn reset(&mut self) {
        self.history.samples.clear();
        for app in self.apps.values_mut() {
            app.low = None;
        }
    }

    pub(crate) fn invalidate(&mut self, name: &str) {
        if let Some(app) = self.apps.get_mut(name) {
            app.since = None;
            app.low = None;
        }
    }

    pub(crate) fn observe(
        &mut self,
        observations: &[Observation],
        mut samples: Response,
        interval: u16,
        now: Timestamp,
    ) {
        if self.history.interval != interval {
            self.reset();
            self.history.interval = interval;
        }
        let freshness = self.history.freshness();
        for observation in observations {
            let ready = observation.ready();
            let state = self
                .apps
                .entry(observation.app.name().to_string())
                .or_insert(AppState {
                    version: observation.version,
                    since: None,
                    waiting: !ready,
                    low: None,
                });
            if state.version != observation.version || !ready || observation.blocked {
                state.version = observation.version;
                state.since = None;
                state.low = None;
            }
            state.waiting |= !ready;
            if ready && !observation.blocked {
                state.since.get_or_insert(now);
            }
            if let Some(containers) = samples.services.get_mut(&observation.service_id) {
                containers.retain(|id, sample| {
                    observation
                        .containers
                        .as_ref()
                        .is_some_and(|ids| ids.contains_key(id))
                        && now.duration_since(sample.observed_at).abs() <= freshness
                });
            }
        }
        self.history.record(samples, now);
    }

    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    pub(crate) fn decide(
        &mut self,
        observation: &Observation,
        nodes: &[super::node::Node],
        now: Timestamp,
    ) -> Decision {
        let name = observation.app.name().as_str();
        let expanding = self.apps.values().any(|app| app.waiting);
        let Some(state) = self.apps.get_mut(name) else {
            return Decision::Hold(None);
        };
        if !observation.ready() || observation.blocked {
            return Decision::Hold(Some("Waiting for deployment and running replicas"));
        }
        let history = &self.history;
        let app = &observation.app;
        let Some(policy) = app.scaling() else {
            return Decision::Hold(None);
        };
        let Some(containers) = &observation.containers else {
            return Decision::Hold(None);
        };
        let Some(since) = state.since else {
            return Decision::Hold(None);
        };
        let limit = app
            .resources()
            .as_ref()
            .and_then(|r| r.limits().as_ref())
            .and_then(super::app::ResourceSpec::cpus);
        let utilization = containers
            .iter()
            .try_fold(0.0, |total, (id, node)| {
                let used = history.average(
                    since,
                    |cpu| {
                        let sample = cpu.services.get(&observation.service_id)?.get(id)?;
                        Some((sample, sample.observed_at, sample.node_uptime_ms))
                    },
                    container_usage,
                )?;
                let baseline =
                    limit.or_else(|| history.node(node, since).map(|(_, cores)| cores))?;
                (baseline.is_finite() && baseline > 0.0).then_some(total + used / baseline)
            })
            .map(|sum| sum / containers.len() as f64);
        let Some(utilization) = utilization else {
            state.low = None;
            return Decision::Hold(Some("Waiting for complete, fresh CPU samples"));
        };
        if state.waiting {
            state.waiting = containers
                .values()
                .any(|id| history.node(id, since).is_none());
            return Decision::Hold(
                state
                    .waiting
                    .then_some("Waiting for CPU samples after expansion"),
            );
        }
        let target = policy.clamp(
            (containers.len() as f64 * utilization / config::SCALING_CPU_TARGET).ceil() as u64,
        );
        if utilization > config::SCALING_CPU_UP && target == app.replicas() {
            state.low = None;
            return Decision::Hold(Some(
                "Automatic scaling has reached the total replica limit",
            ));
        }
        let replicas =
            if utilization > config::SCALING_CPU_UP && target > app.replicas() && !expanding {
                state.low = None;
                target.min(
                    app.replicas()
                        .saturating_add(quota(observation, history, nodes, limit)),
                )
            } else if utilization < config::SCALING_CPU_DOWN && target < app.replicas() {
                let (since, highest) = state.low.get_or_insert((now, target));
                *highest = (*highest).max(target);
                if now.duration_since(*since) < config::SCALING_DOWNSCALE_WAIT {
                    return Decision::Hold(None);
                }
                *highest
            } else {
                state.low = None;
                return Decision::Hold(
                    expanding.then_some("Waiting for another application to finish expanding"),
                );
            };
        if replicas == app.replicas() {
            return Decision::Hold(Some("No CPU capacity for additional replicas"));
        }
        state.waiting = replicas > app.replicas();
        state.since = Some(now);
        state.low = None;
        Decision::Scale {
            replicas,
            utilization,
        }
    }
}

#[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn quota(
    observation: &Observation,
    history: &History,
    nodes: &[super::node::Node],
    limit: Option<f64>,
) -> u64 {
    let placement = observation.app.placement().as_ref();
    let label = placement.and_then(|p| p.label().as_ref());
    let mut eligible = 0_u64;
    let quota = nodes
        .iter()
        .filter(|node| {
            matches!(node.state(), super::node::State::Ready)
                && matches!(node.availability(), super::node::Availability::Active)
                && label.is_none_or(|label| node.labels().contains(label))
        })
        .map(|node| {
            eligible += 1;
            let occupied = observation.occupied.get(node.id()).copied().unwrap_or(0);
            let slots = placement
                .and_then(super::app::Placement::max_replicas_per_node)
                .unwrap_or(super::app::MAX_REPLICAS)
                .saturating_sub(occupied);
            history
                .node(node.id(), Timestamp::MIN)
                .map_or(0, |(idle, cores)| {
                    limit
                        .map_or(
                            u64::from(occupied == 0 && idle >= config::SCALING_CPU_TARGET),
                            |limit| (idle * cores / limit).floor() as u64,
                        )
                        .min(slots)
                })
        })
        .fold(0_u64, u64::saturating_add);
    if limit.is_none() {
        quota.min(eligible.saturating_sub(observation.app.replicas()))
    } else {
        quota
    }
}

impl History {
    fn record(&mut self, mut samples: Response, now: Timestamp) {
        let freshness = self.freshness();
        samples
            .nodes
            .retain(|_, sample| now.duration_since(sample.observed_at).abs() <= freshness);
        let retained = self.window().saturating_add(freshness.saturating_mul(2));
        self.samples.push_back((now, samples));
        while self
            .samples
            .get(1)
            .is_some_and(|(at, _)| now.duration_since(*at) >= retained)
        {
            self.samples.pop_front();
        }
    }

    fn window(&self) -> SignedDuration {
        config::SCALING_WINDOW.max(config::agent_freshness(self.interval))
    }

    fn freshness(&self) -> SignedDuration {
        config::SCALING_FRESHNESS.max(config::agent_freshness(self.interval))
    }

    fn node(&self, id: &str, since: Timestamp) -> Option<(f64, f64)> {
        let idle = self.average(
            since,
            |cpu| {
                let sample = cpu.nodes.get(id)?;
                Some((sample, sample.observed_at, sample.node_uptime_ms))
            },
            node_idle,
        )?;
        Some((
            idle,
            f64::from(self.samples.back()?.1.nodes.get(id)?.cpu_cores),
        ))
    }

    fn average<'a, T: 'a>(
        &'a self,
        since: Timestamp,
        read: impl Fn(&'a Response) -> Option<(&'a T, Timestamp, u64)>,
        measure: impl Fn(&T, &T) -> Option<f64>,
    ) -> Option<f64> {
        let mut values = self
            .samples
            .iter()
            .rev()
            .filter(|(at, _)| *at >= since)
            .map(|(_, cpu)| read(cpu));
        let (last, at, end) = values.next()??;
        if at < since {
            return None;
        }
        let mut previous = (last, end);
        for value in values {
            let (first, at, start) = value?;
            if at < since || i128::from(previous.1.checked_sub(start)?) > self.window().as_millis()
            {
                return None;
            }
            measure(first, previous.0)?;
            if i128::from(end.checked_sub(start)?) >= self.window().as_millis() {
                return measure(first, last);
            }
            previous = (first, start);
        }
        None
    }
}

#[expect(clippy::cast_precision_loss)]
fn container_usage(first: &ContainerSample, last: &ContainerSample) -> Option<f64> {
    if first.container_started_at != last.container_started_at {
        return None;
    }
    let used = last
        .cpu_usage_usec_total?
        .checked_sub(first.cpu_usage_usec_total?)?;
    let elapsed = last.node_uptime_ms.checked_sub(first.node_uptime_ms)?;
    if elapsed == 0 {
        return (used == 0).then_some(0.0);
    }
    Some(used as f64 / elapsed as f64 / 1000.0)
}

#[expect(clippy::cast_precision_loss)]
fn node_idle(first: &NodeSample, last: &NodeSample) -> Option<f64> {
    if first.boot_id != last.boot_id || first.cpu_cores != last.cpu_cores || last.cpu_cores == 0 {
        return None;
    }
    let ticks = |sample: &NodeSample| {
        [
            sample.cpu_user_ticks,
            sample.cpu_system_ticks,
            sample.cpu_steal_ticks,
            sample.cpu_idle_ticks,
            sample.cpu_iowait_ticks,
        ]
    };
    let mut total = 0.0;
    let mut idle = 0.0;
    for (index, (start, end)) in ticks(first).into_iter().zip(ticks(last)).enumerate() {
        let delta = end.checked_sub(start)? as f64;
        total += delta;
        if index >= 3 {
            idle += delta;
        }
    }
    if first.node_uptime_ms == last.node_uptime_ms {
        return (total == 0.0).then_some(0.0);
    }
    (total > 0.0).then_some(idle / total)
}
