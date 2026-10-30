use std::time::Duration;

pub(crate) fn endpoint() -> String {
    format!(
        "{}:{}",
        insight_wire::PLATFORM_SERVICE_NAME,
        pingress_wire::CONFIG_API_PORT
    )
}

pub(crate) fn http_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(FETCH_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
}

pub(crate) const HTTP_ADDR: &str = "[::]:80";
pub(crate) const HTTPS_ADDR: &str = "[::]:443";
pub(crate) const HTTP_ADDR_V4: &str = "0.0.0.0:80";
pub(crate) const HTTPS_ADDR_V4: &str = "0.0.0.0:443";

pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const BOOT_BACKOFF_START: Duration = Duration::from_secs(1);
pub(crate) const BOOT_BACKOFF_MAX: Duration = Duration::from_secs(15);
pub(crate) const HEALTHZ_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
pub(crate) const CHALLENGE_INFLIGHT_CAP: usize = 16;
pub(crate) const CHALLENGE_BODY_CAP: u64 = 256;
pub(crate) const POLL_INTERVAL: Duration = Duration::from_mins(1);
