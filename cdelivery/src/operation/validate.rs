use std::borrow::Cow;
use std::fmt::Display;

use crate::concept;
use crate::locale;

pub(super) struct Tr;

pub(crate) fn report<T: garde::Validate<Context = ()>>(value: &T) -> garde::Report {
    let result = garde::i18n::with_i18n(Tr, || value.validate());
    match result {
        Ok(()) => garde::Report::new(),
        Err(report) => report,
    }
}

pub(crate) fn append(report: &mut garde::Report, path: garde::Path, result: garde::Result) {
    if let Err(error) = result {
        report.append(path, error);
    }
}

macro_rules! tr {
    (@message $key:literal) => {
        locale::tt!($key)
    };
    (@message $key:literal, _) => {
        locale::tt!($key)
    };
    (@message $key:literal, $p:ident) => {
        locale::tt!($key, $p = $p)
    };
    ($($method:ident($($p:tt: $t:ty)?) => $key:literal),+ $(,)?) => {
        impl garde::i18n::I18n for Tr {
            $(fn $method(&self $(, $p: $t)?) -> Cow<'static, str> {
                tr!(@message $key $(, $p)?)
            })+
        }
    };
}

tr! {
    length_lower_than(min: usize) => "validation.length_lower_than",
    length_greater_than(max: usize) => "validation.length_greater_than",
    range_lower_than(min: &dyn Display) => "validation.range_lower_than",
    range_greater_than(max: &dyn Display) => "validation.range_greater_than",
    credit_card_invalid(_: garde::i18n::InvalidCreditCard) => "validation.credit_card_invalid",
    pattern_no_match(_: &dyn Display) => "validation.pattern_no_match",
    contains_missing(pattern: &dyn Display) => "validation.contains_missing",
    url_invalid(_: garde::i18n::InvalidUrl) => "validation.url_invalid",
    prefix_missing(prefix: &dyn Display) => "validation.prefix_missing",
    suffix_missing(suffix: &dyn Display) => "validation.suffix_missing",
    phone_number_invalid(_: garde::i18n::InvalidPhoneNumber) => "validation.phone_number_invalid",
    ip_invalid(_: garde::i18n::IpKind) => "validation.ip_invalid",
    matches_field_mismatch(field: &dyn Display) => "validation.matches_field_mismatch",
    email_invalid(_: garde::i18n::InvalidEmail) => "validation.email_invalid",
    ascii_invalid() => "validation.ascii_invalid",
    alphanumeric_invalid() => "validation.alphanumeric_invalid",
    required_not_set() => "validation.required_not_set",
}

pub(super) fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

macro_rules! check {
    ($($name:ident($v:ident: $t:ty) $key:literal: $ok:expr;)+) => {
        $(
            pub(crate) fn $name($v: &$t, _ctx: &()) -> garde::Result {
                if $ok {
                    Ok(())
                } else {
                    Err(garde::Error::new(locale::tt!($key).into_owned()))
                }
            }

            pub(crate) mod $name {
                pub(crate) fn rule() -> std::borrow::Cow<'static, str> {
                    crate::locale::tt!(locale = crate::locale::Locale::En, $key)
                }
            }
        )+
    };
}

check! {
    sha256_hex(value: str) "validation.sha256_hex": is_sha256_hex(value);
    totp_code(value: str) "validation.totp_code": value.len() == 6
        && value.bytes().all(|byte| byte.is_ascii_digit());
    totp_secret(value: str) "validation.totp_secret": value.len() == 32
        && value
            .bytes()
            .all(|byte| matches!(byte, b'A'..=b'Z' | b'2'..=b'7'));
    valid_name(value: str) "validation.name": value.parse::<concept::name::Name>().is_ok();
    available_volume_name(value: str) "validation.volume_name_reserved":
        !cdelivery_wire::name::volume_reserved(value);
    valid_node_id(value: str) "validation.node_id": concept::node::valid_id(value);
    valid_xconfig_name(value: str) "validation.xconfig_name": value
        .rsplit_once('-')
        .is_some_and(|(app, id)| {
            app.parse::<concept::name::Name>().is_ok()
                && id.len() == 32
                && id.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        });
    valid_xconfig_target(value: str) "validation.xconfig_target":
        value.starts_with('/') && !value.ends_with('/');
    valid_absolute_path(value: str) "validation.absolute_path": value.starts_with('/');
    valid_domain(value: str) "validation.domain":
        value.parse::<concept::domain::DomainName>().is_ok();
    valid_host(value: str) "validation.host":
        pingress_wire::access::Host::parse(value).is_some();
    image_reference(value: str) "validation.image_reference":
        !value.contains('@') && value.parse::<concept::image::Reference>().is_ok();
    settings_host(value: str) "validation.settings_host": !value.is_empty()
        && value.len() <= 255
        && !value.contains("://")
        && !value.contains(['/', '@', '?', '#'])
        && !value.chars().any(|c| c.is_whitespace() || c.is_control());
    settings_endpoint(value: str) "validation.settings_endpoint": value.len() <= 2048
        && url::Url::parse(value).is_ok_and(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
        });
    settings_patrol_interval(value: super::typ::Duration) "validation.settings_patrol_interval":
        matches!(
            value,
            super::typ::Duration::Minutes(15..=1440) | super::typ::Duration::Hours(1..=24)
        );
    key_pem_length(value: cipher::Secret) "validation.key_pem_length":
        (1..=16_384).contains(&value.reveal().len());
    valid_path_prefix(value: str) "validation.path_prefix":
        value.starts_with('/') && value.len() <= 1024 && !value.contains(['?', '#']);
    log_max_size(value: super::typ::ByteSize) "validation.log_max_size":
        (concept::app::MIB..=1 << 30).contains(&u64::from(*value));
    xjob_duration(value: super::typ::Duration) "validation.xjob_duration":
        matches!(value, super::typ::Duration::Minutes(1..=1440) | super::typ::Duration::Hours(1..=24));
    xjob_argument(value: str) "validation.xjob_argument":
        concept::xjob::Spec::valid_argument(value);
    feishu_open_id(value: str) "validation.feishu_open_id": value.strip_prefix("ou_")
        .is_some_and(|rest| {
            !rest.is_empty() && rest.len() <= 64 && rest.bytes().all(|b| b.is_ascii_alphanumeric())
        });
    cpu_cores(value: f64) "validation.cpu_cores": {
        let thousandths = value * 1000.0;
        (thousandths - thousandths.round()).abs() < 1e-6
            && thousandths.round() >= 1.0
            && thousandths.round() <= 10_000_000.0
    };
    slack_user_id(value: str) "validation.slack_user_id": value.len() >= 2
        && value.len() <= 64
        && (value.starts_with('U') || value.starts_with('W'))
        && value.bytes().all(|b| b.is_ascii_alphanumeric());
    telegram_user_id(value: str) "validation.telegram_user_id": value
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_digit() && b != b'0')
        && value.bytes().all(|b| b.is_ascii_digit())
        && value
            .parse::<i64>()
            .is_ok_and(|id| id > 0 && id < (1_i64 << 52));
    discord_user_id(value: str) "validation.discord_user_id": value
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_digit() && b != b'0')
        && value.bytes().all(|b| b.is_ascii_digit())
        && value.parse::<u64>().is_ok_and(|id| id != 0);
}

pub(crate) fn im_id(platform: super::typ::Im) -> impl FnOnce(&str, &()) -> garde::Result {
    move |value, ctx| match platform {
        super::typ::Im::Feishu | super::typ::Im::Lark => feishu_open_id(value, ctx),
        super::typ::Im::Slack => slack_user_id(value, ctx),
        super::typ::Im::Telegram => telegram_user_id(value, ctx),
        super::typ::Im::Discord => discord_user_id(value, ctx),
    }
}

pub(crate) mod im_id {
    pub(crate) fn rule() -> std::borrow::Cow<'static, str> {
        [
            super::feishu_open_id::rule(),
            super::slack_user_id::rule(),
            super::telegram_user_id::rule(),
            super::discord_user_id::rule(),
        ]
        .join(". ")
        .into()
    }
}

pub(crate) fn query_range(
    start: Option<jiff::Timestamp>,
    end: Option<jiff::Timestamp>,
    now: jiff::Timestamp,
) -> (jiff::Timestamp, jiff::Timestamp) {
    let end = end.unwrap_or(now);
    let start = start.unwrap_or_else(|| {
        end.saturating_sub(jiff::SignedDuration::from_hours(1))
            .unwrap_or(jiff::Timestamp::MIN)
    });
    (start, end)
}

fn validate_query_range((start, end): (jiff::Timestamp, jiff::Timestamp)) -> garde::Result {
    if start >= end {
        return Err(garde::Error::new(
            locale::tt!("validation.query_time_range").into_owned(),
        ));
    }
    if end.duration_since(start) > crate::config::INSIGHT_QUERY_MAX_RANGE {
        return Err(garde::Error::new(
            locale::tt!(
                "validation.query_time_range_max",
                days = crate::config::INSIGHT_QUERY_MAX_DAYS
            )
            .into_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn query_time_range(
    start: Option<jiff::Timestamp>,
    end: Option<jiff::Timestamp>,
    now: jiff::Timestamp,
) -> garde::Result {
    validate_query_range(query_range(start, end, now))
}
