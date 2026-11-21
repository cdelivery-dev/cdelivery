use hickory_resolver::Resolver;
use hickory_resolver::config::{ConnectionConfig, NameServerConfig, ResolverConfig};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use jiff::Timestamp;

use crate::config;
use crate::error::Result;

pub(super) async fn check_dns_persist(account_uri: &str, domain: &str) -> bool {
    let built = match std::env::var(config::ACME_DNS_ENV) {
        Ok(addr) => {
            let Ok(socket) = addr.parse::<std::net::SocketAddr>() else {
                return false;
            };
            let mut conn = ConnectionConfig::udp();
            conn.port = socket.port();
            let resolver_config = ResolverConfig::from_parts(
                None,
                vec![],
                vec![NameServerConfig::new(socket.ip(), true, vec![conn])],
            );
            Resolver::builder_with_config(resolver_config, TokioRuntimeProvider::default()).build()
        }
        Err(_) => match Resolver::builder_tokio() {
            Ok(builder) => builder.build(),
            Err(_) => return false,
        },
    };
    let Ok(resolver) = built else {
        return false;
    };
    let domain = domain.trim_end_matches('.');
    let labels: Vec<&str> = domain.split('.').collect();
    for start in 0..labels.len().saturating_sub(1) {
        let Some(slice) = labels.get(start..) else {
            continue;
        };
        let query_domain = slice.join(".");
        let Ok(lookup) = resolver
            .txt_lookup(format!("_validation-persist.{query_domain}"))
            .await
        else {
            continue;
        };
        for record in lookup.answers() {
            let hickory_resolver::proto::rr::RData::TXT(txt) = &record.data else {
                continue;
            };
            if txt_grants(
                &txt.to_string(),
                account_uri,
                query_domain == domain,
                Timestamp::now(),
            ) {
                return true;
            }
        }
    }
    false
}

fn txt_grants(txt: &str, account_uri: &str, exact: bool, now: Timestamp) -> bool {
    let mut account_match = false;
    let mut wildcard = false;
    let mut expired = false;
    for part in txt.split(';') {
        let part = part.trim().trim_matches('"').trim();
        if let Some(rest) = part.strip_prefix("accounturi=") {
            account_match = rest.trim() == account_uri;
        } else if part == "policy=wildcard" {
            wildcard = true;
        } else if let Some(rest) = part.strip_prefix("persistUntil=")
            && let Ok(secs) = rest.trim().parse::<i64>()
            && now.as_second() >= secs
        {
            expired = true;
        }
    }
    account_match && !expired && (exact || wildcard)
}

pub(super) async fn healthz_ok(domain: &str) -> bool {
    let url = format!("http://{domain}{}", pingress_wire::HTTP01_HEALTHZ_PATH);
    let Ok(client) = probe_client() else {
        return false;
    };
    match client.get(&url).send().await {
        Ok(resp) if resp.status() == http::StatusCode::OK => true,
        Ok(resp) => {
            tracing::warn!("HTTP-01 preflight for {domain} returned {}", resp.status());
            false
        }
        Err(error) => {
            tracing::warn!(error = ?crate::error::Error::from(error), "HTTP-01 preflight for {domain} failed");
            false
        }
    }
}

fn probe_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(config::ACME_PREFLIGHT_TIMEOUT.unsigned_abs())
        .build()?)
}
