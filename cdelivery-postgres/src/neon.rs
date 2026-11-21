use std::collections::BTreeMap;

use cdelivery_wire::postgres;
use serde_json::json;

use crate::error::{ErrorKind, Result};

const BASE: &str = "https://console.neon.tech/api/v2";

pub(crate) async fn prepare(
    client: &reqwest::Client,
    fields: &BTreeMap<String, String>,
    name: &str,
    size: Option<&str>,
) -> Result<cipher::Secret> {
    let project = crate::field(fields, "project_id")?;
    let token = crate::field(fields, "api_key")?;
    let parent = crate::field(fields, "branch_id")?;
    let cu = if let Some(size) = size {
        size.parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && *value > 0.0)
            .ok_or_else(|| {
                ErrorKind::Invalid("Neon compute size must be a positive number".to_owned())
            })?
    } else {
        let source = crate::json(
            client
                .get(crate::endpoint(
                    BASE,
                    &[
                        "projects",
                        project,
                        "endpoints",
                        crate::field(fields, "endpoint_id")?,
                    ],
                )?)
                .bearer_auth(token),
        )
        .await?;
        source
            .pointer("/endpoint/autoscaling_limit_min_cu")
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| {
                ErrorKind::Invalid("source endpoint compute size is missing".to_owned())
            })?
    };
    let created = crate::json(client.post(crate::endpoint(BASE, &["projects", project, "branches"])?).bearer_auth(token)
        .json(&json!({
            "branch": {"name": name, "parent_id": parent},
            "endpoints": [{"type": "read_write", "autoscaling_limit_min_cu": cu, "autoscaling_limit_max_cu": cu}]
        }))).await?;
    let branch = crate::string(crate::member(&created, "branch")?, "id")?.to_owned();
    wait(client, project, token, &created).await?;
    let original = postgres::Dsn::parse(crate::field(fields, "dsn")?)?;
    let pooled = crate::field(fields, "pooled")?;
    let connection = crate::json(
        client
            .get(crate::endpoint(
                BASE,
                &["projects", project, "connection_uri"],
            )?)
            .bearer_auth(token)
            .query(&[
                ("branch_id", branch.as_str()),
                ("database_name", original.database()),
                ("role_name", original.username()),
                ("pooled", pooled),
            ]),
    )
    .await?;
    Ok(cipher::Secret::new(postgres::rewrite_dsn(
        original.dsn(),
        Some(crate::string(&connection, "uri")?),
        None,
    )?))
}

pub(crate) async fn cleanup(
    client: &reqwest::Client,
    fields: &BTreeMap<String, String>,
    name: &str,
) -> Result<()> {
    let project = crate::field(fields, "project_id")?;
    let token = crate::field(fields, "api_key")?;
    let mut cursor = String::new();
    let branch = loop {
        let mut request = client
            .get(crate::endpoint(BASE, &["projects", project, "branches"])?)
            .bearer_auth(token)
            .query(&[("search", name), ("limit", "100")]);
        if !cursor.is_empty() {
            request = request.query(&[("cursor", &cursor)]);
        }
        let response = crate::json(request).await?;
        let branches = response
            .get("branches")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| ErrorKind::Invalid("Neon branches response is missing".to_owned()))?;
        if let Some(branch) = branches
            .iter()
            .find(|branch| branch["name"].as_str() == Some(name))
        {
            break crate::string(branch, "id")?.to_owned();
        }
        let Some(next) = response
            .pointer("/pagination/next")
            .and_then(serde_json::Value::as_str)
            .filter(|next| !next.is_empty())
        else {
            return Ok(());
        };
        next.clone_into(&mut cursor);
    };
    if branch == crate::field(fields, "branch_id")? {
        return Err(ErrorKind::Invalid("cannot delete the source branch".to_owned()).into());
    }
    let response = client
        .delete(crate::endpoint(
            BASE,
            &["projects", project, "branches", &branch],
        )?)
        .bearer_auth(token)
        .send()
        .await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }
    if response.status().is_success() {
        return wait(client, project, token, &response.json().await?).await;
    }
    Err(ErrorKind::Provider {
        status: response.status().as_u16(),
        message: response.text().await?,
    }
    .into())
}

async fn wait(
    client: &reqwest::Client,
    project: &str,
    token: &str,
    response: &serde_json::Value,
) -> Result<()> {
    let operations = response
        .get("operations")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| ErrorKind::Invalid("Neon operations are missing".to_owned()))?;
    let started = jiff::Timestamp::now();
    for operation in operations {
        let id = crate::string(operation, "id")?;
        loop {
            let status = crate::json(
                client
                    .get(crate::endpoint(
                        BASE,
                        &["projects", project, "operations", id],
                    )?)
                    .bearer_auth(token),
            )
            .await?;
            match crate::string(crate::member(&status, "operation")?, "status")? {
                "finished" | "skipped" => break,
                "failed" | "cancelled" => {
                    return Err(ErrorKind::Invalid(format!("Neon operation {id} failed")).into());
                }
                _ => {}
            }
            if jiff::Timestamp::now().duration_since(started).as_secs() >= 1200 {
                return Err(ErrorKind::Invalid("Neon operation timed out".to_owned()).into());
            }
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    }
    Ok(())
}
