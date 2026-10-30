use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::num::NonZeroU16;
use std::sync::{Arc, LazyLock};

use arc_swap::ArcSwap;
use pingora::upstreams::peer::HttpPeer;

use crate::config;

#[derive(Debug, Clone)]
pub(crate) struct Candidate {
    pub(crate) app_name: String,
    pub(crate) target: NonZeroU16,
    pub(crate) service_id: String,
}

#[derive(Debug)]
pub(crate) struct RouteEntry {
    path_prefix: Option<String>,
    match_header: Option<pingress_wire::MatchKV>,
    match_cookie: Option<pingress_wire::MatchKV>,
    upstreams: Vec<(String, Candidate)>,
}

type KVRef<'a> = Option<(&'a str, Option<&'a str>)>;

fn kv_ref(kv: Option<&pingress_wire::MatchKV>) -> KVRef<'_> {
    kv.map(|kv| (kv.key.as_str(), kv.value.as_deref()))
}

impl RouteEntry {
    fn from_route(app_name: &str, service_id: &str, route: &pingress_wire::Route) -> Self {
        Self {
            path_prefix: route.path_prefix.clone(),
            match_header: route.header.clone(),
            match_cookie: route.cookie.clone(),
            upstreams: vec![(
                format!("{app_name}:{}", route.target),
                Candidate {
                    app_name: app_name.to_owned(),
                    target: route.target,
                    service_id: service_id.to_owned(),
                },
            )],
        }
    }

    fn score(&self) -> (usize, usize) {
        let set_count = usize::from(self.path_prefix.is_some())
            + usize::from(self.match_header.is_some())
            + usize::from(self.match_cookie.is_some());
        let prefix_len = self.path_prefix.as_deref().map_or(0, str::len);
        (set_count, prefix_len)
    }

    fn tiebreak(&self) -> (Option<&str>, KVRef<'_>, KVRef<'_>) {
        (
            self.path_prefix.as_deref(),
            kv_ref(self.match_header.as_ref()),
            kv_ref(self.match_cookie.as_ref()),
        )
    }

    fn ordering(a: &Self, b: &Self) -> std::cmp::Ordering {
        b.score()
            .cmp(&a.score())
            .then_with(|| a.tiebreak().cmp(&b.tiebreak()))
    }

    fn matches(
        &self,
        path: &str,
        header_val: impl Fn(&str, Option<&str>) -> bool,
        cookie_val: impl Fn(&str, Option<&str>) -> bool,
    ) -> bool {
        if let Some(prefix) = &self.path_prefix {
            let bare = prefix.trim_end_matches('/');
            match path.strip_prefix(bare) {
                Some(rest) if rest.is_empty() || rest.starts_with('/') => {}
                _ => return false,
            }
        }
        if let Some(kv) = &self.match_header
            && !header_val(&kv.key, kv.value.as_deref())
        {
            return false;
        }
        if let Some(kv) = &self.match_cookie
            && !cookie_val(&kv.key, kv.value.as_deref())
        {
            return false;
        }
        true
    }

    fn is_same_route(&self, other: &Self) -> bool {
        self.path_prefix == other.path_prefix
            && self.match_header == other.match_header
            && self.match_cookie == other.match_cookie
    }

    fn merge_upstreams(&mut self, other: &Self) {
        self.upstreams.extend(other.upstreams.iter().cloned());
        self.upstreams.sort_by(|a, b| a.0.cmp(&b.0));
        self.upstreams.dedup_by(|a, b| a.0 == b.0);
    }
}

pub(crate) enum RouteOutcome {
    Peer {
        candidate: Candidate,
        peer: Box<HttpPeer>,
    },
    NoRoute,
    NoUpstream,
}

#[derive(Debug, Default)]
pub(crate) struct Routes {
    entries: Vec<RouteEntry>,
    https: bool,
}

impl Routes {
    fn resolve(
        &self,
        path: &str,
        header_val: impl Fn(&str, Option<&str>) -> bool,
        cookie_val: impl Fn(&str, Option<&str>) -> bool,
        resolved: &HashMap<String, SocketAddr>,
    ) -> RouteOutcome {
        match self
            .entries
            .iter()
            .find(|e| e.matches(path, &header_val, &cookie_val))
        {
            None => RouteOutcome::NoRoute,
            Some(entry) => entry
                .upstreams
                .iter()
                .find_map(|(dns_key, candidate)| {
                    resolved.get(dns_key).map(|address| {
                        let mut peer = HttpPeer::new(*address, false, String::new());
                        peer.options.connection_timeout = Some(config::CONNECT_TIMEOUT);
                        RouteOutcome::Peer {
                            candidate: candidate.clone(),
                            peer: Box::new(peer),
                        }
                    })
                })
                .unwrap_or(RouteOutcome::NoUpstream),
        }
    }

    fn add_entry(&mut self, entry: RouteEntry) {
        if let Some(existing) = self.entries.iter_mut().find(|e| e.is_same_route(&entry)) {
            existing.merge_upstreams(&entry);
        } else {
            self.entries.push(entry);
        }
    }
}

fn upstream_hosts(table: &HashMap<String, Routes>) -> Vec<String> {
    let mut hosts: Vec<String> = table
        .values()
        .flat_map(|routes| routes.entries.iter())
        .flat_map(|entry| entry.upstreams.iter())
        .map(|(dns_key, _)| dns_key.clone())
        .collect();
    hosts.sort();
    hosts.dedup();
    hosts
}

pub(crate) async fn apply(apps: &BTreeMap<String, pingress_wire::AppRoutes>) {
    ROUTING_TABLE.store(Arc::new(build(apps)));
    refresh().await;
}

pub(crate) async fn refresh() {
    let hosts = upstream_hosts(&ROUTING_TABLE.load());
    RESOLVED.store(Arc::new(resolve(&hosts).await));
}

fn build(apps: &BTreeMap<String, pingress_wire::AppRoutes>) -> HashMap<String, Routes> {
    let mut table: HashMap<String, Routes> = HashMap::new();
    for (app_name, app_routes) in apps {
        for spec in &app_routes.routes {
            let entry = table.entry(spec.domain.clone()).or_default();
            entry.https |= spec.https;
            entry.add_entry(RouteEntry::from_route(
                app_name,
                &app_routes.service_id,
                spec,
            ));
        }
    }
    for routes in table.values_mut() {
        routes.entries.sort_by(RouteEntry::ordering);
    }
    table
}

async fn resolve(hosts: &[String]) -> HashMap<String, SocketAddr> {
    let mut lookups = tokio::task::JoinSet::new();
    for host in hosts {
        let host = host.clone();
        lookups.spawn(async move {
            let result = tokio::time::timeout(
                config::LOOKUP_TIMEOUT,
                tokio::net::lookup_host(host.clone()),
            )
            .await;
            (host, result)
        });
    }
    let mut fresh = HashMap::new();
    while let Some(joined) = lookups.join_next().await {
        match joined {
            Ok((host, Ok(Ok(mut addresses)))) => {
                if let Some(address) = addresses.next() {
                    fresh.insert(host, address);
                }
            }
            Ok((host, Ok(Err(error)))) => {
                tracing::warn!(error = ?error, "upstream {host} DNS lookup failed");
            }
            Ok((host, Err(error))) => {
                tracing::warn!(error = ?error, "upstream {host} DNS lookup timed out");
            }
            Err(error) => {
                tracing::error!(error = ?error, "upstream DNS lookup task {:?} failed", error.id());
            }
        }
    }
    merge(hosts, &fresh, &RESOLVED.load())
}

fn merge(
    hosts: &[String],
    fresh: &HashMap<String, SocketAddr>,
    previous: &HashMap<String, SocketAddr>,
) -> HashMap<String, SocketAddr> {
    let mut next = HashMap::new();
    for host in hosts {
        match fresh.get(host).or_else(|| previous.get(host)) {
            Some(addr) => {
                next.insert(host.clone(), *addr);
            }
            None => {
                tracing::warn!("upstream {host} does not resolve and has no cached address");
            }
        }
    }
    next
}

pub(crate) fn is_managed(domain: &str) -> bool {
    ROUTING_TABLE.load().contains_key(domain)
}

pub(crate) fn decide(
    domain: &str,
    path: &str,
    header_val: impl Fn(&str, Option<&str>) -> bool,
    cookie_val: impl Fn(&str, Option<&str>) -> bool,
) -> Option<(bool, RouteOutcome)> {
    let table = ROUTING_TABLE.load();
    let routes = table.get(domain)?;
    Some((
        routes.https,
        routes.resolve(path, header_val, cookie_val, &RESOLVED.load()),
    ))
}

static ROUTING_TABLE: LazyLock<ArcSwap<HashMap<String, Routes>>> =
    LazyLock::new(|| ArcSwap::from_pointee(HashMap::new()));

static RESOLVED: LazyLock<ArcSwap<HashMap<String, SocketAddr>>> =
    LazyLock::new(|| ArcSwap::from_pointee(HashMap::new()));
