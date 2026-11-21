use std::collections::BTreeMap;

use cdelivery_wire::postgres;
use serde_json::json;

use crate::error::{ErrorKind, Result};

const BASE: &str = "https://api.planetscale.com/v1";

fn request(
    client: &reqwest::Client,
    fields: &BTreeMap<String, String>,
    method: reqwest::Method,
    branch: &str,
    suffix: &[&str],
) -> Result<reqwest::RequestBuilder> {
    let mut segments = vec![
        "organizations",
        crate::field(fields, "organization")?,
        "databases",
        crate::field(fields, "database_name")?,
        "branches",
        branch,
    ];
    segments.extend_from_slice(suffix);
    let request = client.request(method, crate::endpoint(BASE, &segments)?);
    let token = crate::field(fields, "api_token")?;
    Ok(request.bearer_auth(token))
}

#[expect(clippy::too_many_lines)]
pub(crate) async fn prepare(
    client: &reqwest::Client,
    fields: &BTreeMap<String, String>,
    name: &str,
    size: Option<&str>,
    sizes: Option<&[postgres::PlanetScaleSku]>,
) -> Result<cipher::Secret> {
    let parent = crate::field(fields, "branch_name")?;
    let source = crate::json(request(client, fields, reqwest::Method::GET, parent, &[])?).await?;
    let source_size = crate::string(&source, "cluster_name")?;
    let region = crate::string(crate::member(&source, "region")?, "slug")?;
    let metal = source
        .get("metal")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| ErrorKind::Invalid("source storage type is missing".to_owned()))?;
    let selected = size.or(if metal { Some("PS-5") } else { None });
    let cluster_size = if let Some(selected) = selected {
        let sizes = sizes.ok_or_else(|| {
            ErrorKind::Invalid("PlanetScale size catalog is unavailable".to_owned())
        })?;
        let source = sizes
            .iter()
            .find(|sku| sku.name == source_size)
            .ok_or_else(|| {
                ErrorKind::Invalid(format!(
                    "source SKU {source_size} is missing from the PlanetScale size catalog"
                ))
            })?;
        let mut targets = sizes.iter().filter(|sku| {
            !sku.metal
                && sku.size == selected
                && sku.provider == source.provider
                && sku.architecture == source.architecture
        });
        let target = targets.next().ok_or_else(|| {
            ErrorKind::Invalid(format!(
                "PlanetScale rehearsal size {selected} has no matching SKU for {source_size}"
            ))
        })?;
        if targets.next().is_some() {
            return Err(ErrorKind::Invalid(format!(
                "PlanetScale rehearsal size {selected} has multiple matching SKUs for {source_size}"
            ))
            .into());
        }
        target.name.clone()
    } else {
        source_size.to_owned()
    };
    let restore = jiff::Timestamp::now()
        .checked_sub(jiff::SignedDuration::from_secs(300))
        .map_err(|error| ErrorKind::Invalid(error.to_string()))?;
    let mut page = 1;
    let mut backup: Option<(jiff::Timestamp, String)> = None;
    loop {
        let response = crate::json(
            request(client, fields, reqwest::Method::GET, parent, &["backups"])?.query(&[
                ("state", "success".to_owned()),
                ("to", restore.to_string()),
                ("per_page", "100".to_owned()),
                ("page", page.to_string()),
            ]),
        )
        .await?;
        let backups = response
            .get("data")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| ErrorKind::Invalid("backups response is missing".to_owned()))?;
        for candidate in backups {
            let Some(completed) = candidate["completed_at"].as_str() else {
                continue;
            };
            let completed = completed
                .parse::<jiff::Timestamp>()
                .map_err(|error| ErrorKind::Invalid(error.to_string()))?;
            if completed <= restore
                && backup
                    .as_ref()
                    .is_none_or(|(previous, _)| completed > *previous)
            {
                backup = Some((completed, crate::string(candidate, "id")?.to_owned()));
            }
        }
        let Some(next) = response
            .get("next_page")
            .and_then(serde_json::Value::as_u64)
        else {
            break;
        };
        page = next;
    }
    let (_, backup_id) = backup.ok_or_else(|| {
        ErrorKind::Invalid("no completed backup is available before the recovery point".to_owned())
    })?;
    let extensions = crate::json(request(
        client,
        fields,
        reqwest::Method::GET,
        parent,
        &["extensions"],
    )?)
    .await?;
    let extensions = extensions
        .as_array()
        .ok_or_else(|| ErrorKind::Invalid("source extensions response is missing".to_owned()))?;
    let mut enabled = Vec::new();
    let mut parameters = serde_json::Map::new();
    for extension in extensions {
        if extension["enabled"].as_bool() == Some(false) {
            continue;
        }
        if extension["can_enable"].as_bool() == Some(true)
            && extension["enabled"].as_bool() == Some(true)
        {
            enabled.push(crate::string(extension, "name")?.to_owned());
        }
        if let Some(settings) = extension["parameters"].as_array() {
            for setting in settings {
                if setting["internal"].as_bool() == Some(true)
                    || setting["disabled"].as_bool() == Some(true)
                {
                    continue;
                }
                let Some(value) = setting["value"].as_str() else {
                    continue;
                };
                let namespace = crate::string(setting, "namespace")?;
                let name = crate::string(setting, "name")?;
                let entry = parameters.entry(namespace).or_insert_with(|| json!({}));
                entry[name] = json!(value);
            }
        }
    }
    let token = crate::field(fields, "api_token")?;
    let url = crate::endpoint(
        BASE,
        &[
            "organizations",
            crate::field(fields, "organization")?,
            "databases",
            crate::field(fields, "database_name")?,
            "branches",
        ],
    )?;
    let create = client.post(url).bearer_auth(token);
    crate::json(create.json(&json!({"name": name, "parent_branch": parent, "backup_id": backup_id, "restore_point": restore.to_string(), "cluster_size": cluster_size, "region": region, "replicas": 0}))).await?;
    wait_ready(client, fields, name).await?;
    let change = crate::json(
        request(client, fields, reqwest::Method::PATCH, name, &["changes"])?
            .json(&json!({"extensions": enabled, "parameters": parameters})),
    )
    .await?;
    if !change.is_null() {
        let id = crate::string(&change, "id")?;
        let started = jiff::Timestamp::now();
        loop {
            let change = crate::json(request(
                client,
                fields,
                reqwest::Method::GET,
                name,
                &["changes", id],
            )?)
            .await?;
            match crate::string(&change, "state")? {
                "completed" => break,
                "canceled" => {
                    return Err(ErrorKind::Invalid(
                        "PlanetScale extension changes were canceled".to_owned(),
                    )
                    .into());
                }
                _ => {}
            }
            if jiff::Timestamp::now().duration_since(started).as_secs() >= 1200 {
                return Err(ErrorKind::Invalid(
                    "PlanetScale extension configuration timed out".to_owned(),
                )
                .into());
            }
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    }
    let base_username = crate::field(fields, "base_username")?;
    let mut page = 1;
    let role_id = loop {
        let response = crate::json(
            request(client, fields, reqwest::Method::GET, name, &["roles"])?.query(&[
                ("q", base_username.to_owned()),
                ("per_page", "100".to_owned()),
                ("page", page.to_string()),
            ]),
        )
        .await?;
        let roles = response
            .get("data")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| ErrorKind::Invalid("roles response is missing".to_owned()))?;
        if let Some(role) = roles
            .iter()
            .find(|role| role["base_username"].as_str() == Some(base_username))
        {
            break crate::string(role, "id")?.to_owned();
        }
        let Some(next) = response
            .get("next_page")
            .and_then(serde_json::Value::as_u64)
        else {
            return Err(ErrorKind::Invalid(
                "restored branch does not contain the configured database role".to_owned(),
            )
            .into());
        };
        page = next;
    };
    let role = crate::json(request(
        client,
        fields,
        reqwest::Method::POST,
        name,
        &["roles", &role_id, "reset"],
    )?)
    .await?;
    let original = crate::field(fields, "dsn")?;
    let mut target = url::Url::parse(original)?;
    target.set_host(Some(crate::string(&role, "access_host_url")?))?;
    target
        .set_username(crate::string(&role, "username")?)
        .map_err(|()| ErrorKind::Invalid("invalid role username".to_owned()))?;
    target
        .set_password(Some(crate::string(&role, "password")?))
        .map_err(|()| ErrorKind::Invalid("invalid role password".to_owned()))?;
    Ok(cipher::Secret::new(postgres::rewrite_dsn(
        original,
        Some(target.as_str()),
        None,
    )?))
}

async fn wait_ready(
    client: &reqwest::Client,
    fields: &BTreeMap<String, String>,
    name: &str,
) -> Result<()> {
    let started = jiff::Timestamp::now();
    loop {
        let branch = crate::json(request(client, fields, reqwest::Method::GET, name, &[])?).await?;
        if branch.get("ready").and_then(serde_json::Value::as_bool) == Some(true) {
            return Ok(());
        }
        if jiff::Timestamp::now().duration_since(started).as_secs() >= 1200 {
            return Err(
                ErrorKind::Invalid("PlanetScale branch preparation timed out".to_owned()).into(),
            );
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
}

pub(crate) async fn cleanup(
    client: &reqwest::Client,
    fields: &BTreeMap<String, String>,
    name: &str,
) -> Result<()> {
    if name == crate::field(fields, "branch_name")? {
        return Err(ErrorKind::Invalid("cannot delete the source branch".to_owned()).into());
    }
    let response = request(client, fields, reqwest::Method::DELETE, name, &[])?
        .send()
        .await?;
    if response.status().is_success() || response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }
    Err(ErrorKind::Provider {
        status: response.status().as_u16(),
        message: response.text().await?,
    }
    .into())
}
