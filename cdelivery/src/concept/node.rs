pub(crate) mod join;

use getset::{CopyGetters, Getters};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Selector {
    NodeId(String),
    Label(String),
}

pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Role {
    Manager,
    Worker,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Availability {
    Active,
    Pause,
    Drain,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum State {
    Ready,
    Down,
    Disconnected,
    Unknown,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Reachability {
    Reachable,
    Unreachable,
    Unknown,
}

#[derive(Debug, Getters, CopyGetters, macros_derive::Ctor)]
pub(crate) struct Node {
    #[getset(get = "pub(crate)")]
    id: String,
    #[getset(get = "pub(crate)")]
    hostname: String,
    #[getset(get_copy = "pub(crate)")]
    role: Role,
    #[getset(get_copy = "pub(crate)")]
    availability: Availability,
    #[getset(get_copy = "pub(crate)")]
    state: State,
    #[getset(get_copy = "pub(crate)")]
    reachability: Option<Reachability>,
    #[getset(get_copy = "pub(crate)")]
    cpus: f64,
    #[getset(get_copy = "pub(crate)")]
    memory_bytes: u64,
    #[getset(get = "pub(crate)")]
    engine_version: String,
    #[getset(get = "pub(crate)")]
    addr: String,
    #[getset(get = "pub(crate)")]
    labels: Vec<String>,
    #[getset(get_copy = "pub(crate)")]
    created_at: jiff::Timestamp,
    #[getset(get = "pub(crate)")]
    os: Option<String>,
    #[getset(get = "pub(crate)")]
    architecture: Option<String>,
    #[getset(get_copy = "pub(crate)")]
    leader: Option<bool>,
    #[getset(get = "pub(crate)")]
    status_message: String,
}

#[derive(Debug, Getters, macros_derive::Ctor)]
#[getset(get = "pub(crate)")]
pub(crate) struct JoinInfo {
    manager_addr: std::net::SocketAddr,
    token: cipher::Secret,
}
