mod account;
mod app;
mod asset;
mod auth;
mod command;
mod conditions;
mod dashboard;
mod events;
mod group;
mod insight;
mod metrics;
mod node;
mod overview;
mod pingress;
mod range;
mod response;
pub(crate) mod router;
mod settings;
mod setup;
mod target;
mod volume;

fn im_title(platform: crate::operation::typ::Im) -> std::borrow::Cow<'static, str> {
    match platform {
        crate::operation::typ::Im::Feishu => crate::locale::tt!("ui.channel.feishu.title"),
        crate::operation::typ::Im::Lark => crate::locale::tt!("ui.channel.lark.title"),
        crate::operation::typ::Im::Slack => crate::locale::tt!("ui.channel.slack.title"),
        crate::operation::typ::Im::Telegram => crate::locale::tt!("ui.channel.telegram.title"),
        crate::operation::typ::Im::Discord => crate::locale::tt!("ui.channel.discord.title"),
    }
}

fn im_slug(platform: crate::operation::typ::Im) -> &'static str {
    crate::concept::message::Im::from(platform).key()
}

async fn read<O>(
    app: &crate::boot::App,
    access: &crate::concept::permission::Access,
) -> crate::error::Result<O::Output>
where
    O: crate::operation::catalog::Catalogued<Input = crate::operation::input::Empty>,
{
    crate::operation::execute::<O>(
        app,
        access,
        crate::operation::input::Empty {},
        jiff::Timestamp::now(),
    )
    .await
}

fn read_error(error: &crate::error::Error, context: &str) -> String {
    if !matches!(error.kind(), crate::error::ErrorKind::Validation(_)) {
        tracing::warn!(error = ?error, "{context}");
    }
    error.detail(crate::locale::current()).0
}

fn token<T: serde::de::DeserializeOwned>(value: Option<&str>) -> crate::error::Result<Option<T>> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| serde_json::from_value(serde_json::Value::String(value.to_owned())))
        .transpose()
        .map_err(|error| crate::error::ErrorKind::InvalidBody(error.to_string()).into())
}

fn pages(
    skip: u32,
    len: usize,
    limit: u16,
    url: impl Fn(u32) -> String,
) -> (Option<String>, Option<String>) {
    let previous = (skip > 0).then(|| url(skip.saturating_sub(u32::from(limit))));
    let next = (len == usize::from(limit))
        .then(|| skip.checked_add(u32::from(limit)).map(&url))
        .flatten();
    (previous, next)
}

async fn load_nodes(
    app: &crate::boot::App,
    access: &crate::concept::permission::Access,
    needed: bool,
) -> crate::error::Result<crate::operation::node::typ::Nodes> {
    if !needed {
        return Ok(crate::operation::node::typ::Nodes { nodes: Vec::new() });
    }
    read::<crate::operation::node::List>(app, access).await
}

async fn latest_versions<const N: usize>(
    app: &crate::boot::App,
    access: &crate::concept::permission::Access,
    needed: bool,
    components: [crate::concept::release::Component; N],
) -> [Option<crate::operation::instance::typ::Release>; N] {
    if !needed {
        return components.map(|_| None);
    }
    match read::<crate::operation::instance::VersionList>(app, access).await {
        Ok(mut versions) => components.map(|component| versions.0.remove(component.as_ref())),
        Err(error) => {
            tracing::warn!(error = ?error, "release metadata unavailable");
            components.map(|_| None)
        }
    }
}

fn target_version(
    current: Option<&str>,
    latest: Option<crate::operation::instance::typ::Release>,
) -> crate::error::Result<Option<String>> {
    let Some(latest) = latest else {
        return Ok(None);
    };
    let latest_version = crate::concept::release::parse_version(&latest.version)?;
    let is_newer = match current.map(crate::concept::release::parse_version) {
        Some(Ok(current)) => latest_version > current,
        Some(Err(_)) | None => true,
    };
    Ok(is_newer.then_some(latest.version))
}

#[expect(clippy::cast_precision_loss)]
fn bytes_text(bytes: u64) -> String {
    if bytes >= 1 << 30 {
        format!("{:.1} GiB", bytes as f64 / f64::from(1 << 30))
    } else if bytes >= 1 << 20 {
        format!("{:.1} MiB", bytes as f64 / f64::from(1 << 20))
    } else if bytes >= 1 << 10 {
        format!("{:.1} KiB", bytes as f64 / f64::from(1 << 10))
    } else {
        format!("{bytes} B")
    }
}

fn page_url(
    path: &str,
    range: (jiff::Timestamp, jiff::Timestamp),
    extra: &[(&str, &str)],
    skip: u32,
) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query
        .append_pair("start", &range.0.to_string())
        .append_pair("end", &range.1.to_string());
    for &(key, value) in extra {
        if !value.is_empty() {
            query.append_pair(key, value);
        }
    }
    if skip > 0 {
        query.append_pair("skip", &skip.to_string());
    }
    format!("{path}?{}", query.finish())
}
