pub(crate) const SERVICE_NAME: &str = insight_wire::PLATFORM_SERVICE_NAME;
pub(crate) const SECRET_NAME: &str = "cdelivery";
pub(crate) const NETWORK_NAME: &str = "cdelivery";
pub(crate) const PUBLIC_PORT: std::num::NonZeroU16 = std::num::NonZeroU16::new(9000).unwrap();
pub(crate) const LISTEN_ADDR: (&str, u16) = ("0.0.0.0", PUBLIC_PORT.get());
pub(crate) const INTERNAL_LISTEN_ADDR: (&str, u16) = ("0.0.0.0", pingress_wire::CONFIG_API_PORT);
pub(crate) const JSON_BODY_LIMIT: usize = 6 * 1024 * 1024;
pub(crate) const MCP_CACHE_TTL: SignedDuration = SignedDuration::from_mins(5);
pub(crate) const PUBLIC_BODY_LIMIT: usize = 64 * 1024;
pub(crate) const SETUP_TIMEOUT: SignedDuration = SignedDuration::from_secs(45);
pub(crate) const RECORD_QUEUE: usize = 1024;
pub(crate) const SHUTDOWN_TIMEOUT: SignedDuration = SignedDuration::from_secs(45);
pub(crate) const SERVICE_UPDATE_MONITOR: SignedDuration = SignedDuration::from_secs(60);

pub(crate) const VOLUME_AGENT_SERVICE_NAME: &str = "cdelivery-agent";

pub(crate) const SECRET_PATH_ENV: &str = "CDELIVERY_SECRET_PATH";
pub(crate) const RELEASES_URL_ENV: &str = "CDELIVERY_RELEASES_URL";
const DEFAULT_RELEASES_URL: &str = "https://releases.cdelivery.dev/";
pub(crate) const RELEASE_TIMEOUT: SignedDuration = SignedDuration::from_secs(10);
pub(crate) const RELEASE_BODY_LIMIT: usize = 128 * 1024;

pub(crate) const SESSION_COOKIE: &str = "cdelivery_session";
pub(crate) const SESSION_TTL: SignedDuration = SignedDuration::from_hours(7 * 24);
pub(crate) const NODE_JOIN_TTL: SignedDuration = SignedDuration::from_mins(30);
pub(crate) const SESSION_RENEW_MIN_EXTENSION: SignedDuration = SignedDuration::from_hours(24);
pub(crate) const TOTP_STEP_SECS: u64 = 30;
pub(crate) const TOTP_COOLDOWN: SignedDuration = SignedDuration::from_mins(5);

pub(crate) const MAINTENANCE_INTERVAL: SignedDuration = SignedDuration::from_hours(1);
pub(crate) const XCONFIG_GC_GRACE: SignedDuration = SignedDuration::from_hours(1);
pub(crate) const XCONFIG_DATA_MAX: usize = docker::MAX_CONFIG_DATA_SIZE;
pub(crate) const GROUP_DELETE_WAIT: SignedDuration = SignedDuration::from_secs(10);

pub(crate) const ACME_DNS_ENV: &str = "CDELIVERY_ACME_DNS";
pub(crate) const ACME_ISSUE_INTERVAL: SignedDuration = SignedDuration::from_mins(5);
pub(crate) const ACME_RENEW_INTERVAL: SignedDuration = SignedDuration::from_hours(8);
pub(crate) const ACME_PREFLIGHT_TIMEOUT: SignedDuration = SignedDuration::from_secs(10);
pub(crate) const ACME_POLL_READY_TIMEOUT: SignedDuration = SignedDuration::from_secs(60);
pub(crate) const ACME_POLL_CERT_TIMEOUT: SignedDuration = SignedDuration::from_secs(20);
pub(crate) const ACME_ISSUE_TIMEOUT: SignedDuration = SignedDuration::from_mins(5);
pub(crate) const ACME_ARI_TIMEOUT: SignedDuration = SignedDuration::from_secs(30);

pub(crate) const PINGRESS_SERVICE_NAME: &str = pingress_wire::access::SERVICE_NAME;
pub(crate) const PINGRESS_NETWORK: &str = "cdelivery-pingress";

pub(crate) const INSIGHT_SERVICE_NAME: &str = insight_wire::SERVICE_NAME;
pub(crate) const INSIGHT_AGENT_SERVICE_NAME: &str = "cdelivery-insight-agent";
pub(crate) const INSIGHT_VOLUME: &str = "cdelivery-insight-data";
pub(crate) const INSIGHT_SAMPLE_INTERVAL_DEFAULT_SECS: u16 = 5;
pub(crate) const INSIGHT_WRITER_FLUSH_INTERVAL_DEFAULT: SignedDuration =
    SignedDuration::from_mins(3);
pub(crate) const INSIGHT_QUERY_TIMEOUT: SignedDuration = SignedDuration::from_secs(60);
pub(crate) const SCALING_INTERVAL: SignedDuration = SignedDuration::from_secs(30);
pub(crate) const SCALING_QUERY_TIMEOUT: SignedDuration = SignedDuration::from_secs(5);
pub(crate) const SCALING_QUERY_CONCURRENCY: usize = 8;
pub(crate) const SCALING_WINDOW: SignedDuration = SignedDuration::from_secs(60);
pub(crate) const SCALING_FRESHNESS: SignedDuration = SignedDuration::from_secs(15);
pub(crate) const SCALING_DOWNSCALE_WAIT: SignedDuration = SignedDuration::from_mins(5);
pub(crate) const SCALING_CPU_TARGET: f64 = 0.6;
pub(crate) const SCALING_CPU_UP: f64 = 0.7;
pub(crate) const SCALING_CPU_DOWN: f64 = 0.5;
pub(crate) const INSIGHT_QUERY_MAX_DAYS: i64 = 31;
pub(crate) const INSIGHT_QUERY_MAX_RANGE: SignedDuration =
    SignedDuration::from_hours(INSIGHT_QUERY_MAX_DAYS * 24);
pub(crate) const METRIC_DEFAULT_BUCKETS: u16 = 20;
pub(crate) const UI_BUCKETS: u16 = 270;
pub(crate) const AGENT_REPORT_WINDOW_SECS: u32 = 15 * 60;

pub(crate) fn step_floor(step_secs: u32, sample_interval_secs: u16) -> u32 {
    step_secs.max(u32::from(sample_interval_secs))
}

pub(crate) fn agent_freshness(sample_interval_secs: u16) -> SignedDuration {
    SignedDuration::from_secs(i64::from(sample_interval_secs) * 2)
}

pub(crate) fn metric_step_secs(
    (start, end): (jiff::Timestamp, jiff::Timestamp),
    max_buckets: Option<u16>,
) -> u32 {
    let duration = end.duration_since(start);
    let span_secs = u32::try_from(
        duration
            .as_secs()
            .saturating_add(i64::from(duration.subsec_nanos() > 0)),
    )
    .unwrap_or(u32::MAX);
    span_secs
        .div_ceil(u32::from(max_buckets.unwrap_or(METRIC_DEFAULT_BUCKETS)))
        .clamp(
            u32::from(insight_wire::metric::SAMPLE_INTERVAL_MIN_SECS),
            insight_wire::metric::STEP_SECS_MAX,
        )
}

pub(crate) const EVENT_MESSAGE_MAX: usize = 500;
pub(crate) const EVENT_DETAIL_MAX: usize = 65_536;
pub(crate) const TASK_CRASH_WINDOW: SignedDuration = SignedDuration::from_mins(10);
pub(crate) const INSTANCE_ALERT_DELAY: SignedDuration = SignedDuration::from_mins(2);

pub(crate) const HURL_FILE_PATH: &str = "/xhealth.hurl";
pub(crate) const HURL_TARGET_URL_ENV: &str = "HURL_VARIABLE_TARGET_URL";
pub(crate) const TARGET_HOST_ENV: &str = "TARGET_HOST";
pub(crate) const TARGET_PORT_ENV: &str = "TARGET_PORT";
pub(crate) const TASK_SETTLE: SignedDuration = SignedDuration::from_secs(90);
pub(crate) const GUARDIAN_DEPLOY_TIMEOUT: SignedDuration = SignedDuration::from_mins(10);
pub(crate) const GUARDIAN_PROMOTE_SETTLE: SignedDuration = SignedDuration::from_secs(90);
pub(crate) const XHEALTH_MIN_INTERVAL: SignedDuration = SignedDuration::from_mins(1);
pub(crate) const RECONCILE_INTERVAL: SignedDuration = SignedDuration::from_secs(30);

pub(crate) const XJOB_MAX_PER_APP: usize = 16;
pub(crate) const XJOB_MAX_DURATION: SignedDuration = SignedDuration::from_hours(24);

pub(crate) const DUTY_OPERATOR: &str = "duty-agent";
pub(crate) const AGENT_MESSAGE_RING: u16 = 50;
pub(crate) const AGENT_CHAT_WINDOW: SignedDuration = SignedDuration::from_hours(168);
pub(crate) const AGENT_PATROL_INTERVAL_DEFAULT: SignedDuration = SignedDuration::from_mins(30);
pub(crate) const AGENT_MAX_TURNS: usize = 16;
pub(crate) const AGENT_TOOL_OUTPUT_MAX: usize = 65_536;
pub(crate) const AGENT_TOOL_OUTPUT_MIN: usize = 1_024;
pub(crate) const AGENT_PROMPT_CHAR_MAX: usize = 300_000;
pub(crate) const AGENT_CONCURRENCY: usize = 2;
pub(crate) const AGENT_INBOUND_TEXT_MAX: usize = 4_000;
pub(crate) const AGENT_TEXT_MAX: usize = 10_000;
pub(crate) const AGENT_IMAP_POLL: SignedDuration = SignedDuration::from_mins(1);
pub(crate) const AGENT_IMAP_DEADLINE: SignedDuration = SignedDuration::from_mins(5);
pub(crate) const AGENT_LLM_TIMEOUT: SignedDuration = SignedDuration::from_mins(5);
pub(crate) const AGENT_RETRY: SignedDuration = SignedDuration::from_secs(60);
pub(crate) const AGENT_CHANNEL_TIMEOUT: SignedDuration = SignedDuration::from_secs(30);
pub(crate) const AGENT_SEND_TIMEOUT: SignedDuration = SignedDuration::from_secs(60);
pub(crate) const AGENT_INBOUND_DRAIN: SignedDuration = SignedDuration::from_secs(20);
pub(crate) const AGENT_INBOUND_RETRY: SignedDuration = SignedDuration::from_secs(1);
pub(crate) const OPERATION_STANDARD_TIMEOUT: SignedDuration = SignedDuration::from_secs(30);
pub(crate) const OPERATION_EXTENDED_TIMEOUT: SignedDuration = SignedDuration::from_secs(190);
pub(crate) const PUBLIC_TRANSPORT_TIMEOUT: SignedDuration = SignedDuration::from_secs(35);
pub(crate) const PROTECTED_TRANSPORT_TIMEOUT: SignedDuration = SignedDuration::from_secs(210);
pub(crate) const AGENT_FEISHU_TOKEN_MARGIN: SignedDuration = SignedDuration::from_mins(5);
pub(crate) const AGENT_FEISHU_FRAGMENT_TTL: SignedDuration = SignedDuration::from_secs(5);
pub(crate) const AGENT_FEISHU_FRAGMENT_MAX: usize = 64;
pub(crate) const AGENT_SLACK_PING: SignedDuration = SignedDuration::from_secs(10);
pub(crate) const AGENT_SLACK_STALL: SignedDuration = SignedDuration::from_secs(30);
pub(crate) const AGENT_SLACK_RECONNECT: SignedDuration = SignedDuration::from_secs(30);
pub(crate) const AGENT_SLACK_RECONNECT_NONCE: u64 = 5;
pub(crate) const AGENT_EVENT_DEDUP: usize = 256;
pub(crate) const AGENT_IM_INBOUND_QUEUE: usize = 64;
pub(crate) const AGENT_IM_INBOUND_WAIT: SignedDuration = SignedDuration::from_secs(5);
pub(crate) const AGENT_DUTY_QUEUE: usize = 256;
pub(crate) const AGENT_ANALYSIS_DEBOUNCE: SignedDuration = SignedDuration::from_secs(3);

use jiff::{SignedDuration, Timestamp};

use crate::error::{ErrorKind, Result};

pub(crate) fn expiry(now: Timestamp, ttl: SignedDuration) -> Timestamp {
    now.saturating_add(ttl).unwrap_or(Timestamp::MAX)
}

pub(crate) fn secret_path() -> String {
    std::env::var(SECRET_PATH_ENV).unwrap_or_else(|_| "/run/secrets/cdelivery".into())
}

pub(crate) fn releases_url() -> Result<url::Url> {
    let raw = match std::env::var(RELEASES_URL_ENV) {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => DEFAULT_RELEASES_URL.to_owned(),
        Err(std::env::VarError::NotUnicode(_)) => return Err(ErrorKind::ReleaseUrlInvalid.into()),
    };
    let url = url::Url::parse(raw.trim()).map_err(|_| ErrorKind::ReleaseUrlInvalid)?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(ErrorKind::ReleaseUrlInvalid.into());
    }
    if url.scheme() == "http" {
        tracing::warn!("release catalog uses unencrypted HTTP transport");
    }
    Ok(url)
}
