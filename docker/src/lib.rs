mod client;
mod error;
pub mod types;

pub use client::*;
pub use error::{Error, Result};

pub const DEFAULT_SOCKET_PATH: &str = "/var/run/docker.sock";
