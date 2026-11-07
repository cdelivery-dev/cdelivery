use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::support::{self, Context, Result, ensure, send, string};

#[expect(clippy::too_many_lines)]
pub(crate) async fn permissions(context: &Context) -> Result<()> {
    let exported: Value = serde_json::from_str(&context.docker(&[
        "run",
        "--rm",
        &context.environment.images["cdelivery"].reference,
        "contract",
        "mcp",
    ])?)?;
    let catalog = exported
        .as_array()
        .ok_or_else(|| support::ErrorKind::Check("missing exported catalog".into()))?;
    let openapi: Value = serde_json::from_str(&context.docker(&[
        "run",
        "--rm",
        &context.environment.images["cdelivery"].reference,
        "contract",
        "openapi",
    ])?)?;
    let paths = openapi["paths"]
        .as_object()
        .ok_or_else(|| support::ErrorKind::Check("missing REST paths".into()))?;
    for (operation, requirement) in [
        ("group_list", "Shared"),
        ("version_list", "Shared"),
        ("me_get", "Own"),
        ("app_create", "Operator"),
        ("secret_create", "Operator"),
        ("secret_delete", "Operator"),
        ("guardian_skip", "Operator"),
        ("instance_update", "Admin"),
        ("settings_discover_models", "Admin"),
        ("user_list", "Admin"),
    ] {
        ensure(
            catalog.iter().any(|entry| {
                entry["tool"]["name"] == operation && entry["requirement"] == requirement
            }),
            &format!("incorrect independent permission anchor: {operation}"),
        )?;
    }
    for role in ["Reader", "Operator", "Admin"] {
        let email = format!("{}@cdelivery.test", support::name("catalog"));
        let password = "Catalog-Aa123456";
        let user = context.api("POST", "/api/users", Some(json!({"first_name":"Catalog","last_name":role,"email":email,"password_sha256":support::hash(password),"role":role}))).await?;
        let session = context.login(&email, password).await?;
        let key: Value = send(
            session
                .post(format!("{}/api/apikeys", context.environment.base_url))
                .header("Origin", &context.environment.base_url)
                .json(&json!({"name":"catalog","role":role})),
            201,
        )
        .await?
        .json()
        .await?;
        let token = string(&key, "key")?;
        let listed = mcp(context, token, "tools/list", json!({})).await?;
        let tools = listed["result"]["tools"]
            .as_array()
            .ok_or_else(|| support::ErrorKind::Check("missing visible tools".into()))?;
        let mut expected = BTreeSet::new();
        for entry in catalog {
            let name = string(&entry["tool"], "name")?;
            let allowed = match string(entry, "requirement")? {
                "Shared" | "Own" => true,
                "Operator" => role != "Reader",
                "Admin" => role == "Admin",
                value => {
                    return Err(
                        support::ErrorKind::Check(format!("unknown requirement: {value}")).into(),
                    );
                }
            };
            if allowed {
                expected.insert(name);
                continue;
            }
            let denied = mcp(
                context,
                token,
                "tools/call",
                json!({"name":name,"arguments":{"unexpected":true}}),
            )
            .await?;
            let problem: Value =
                serde_json::from_str(string(&denied["result"]["content"][0], "text")?)?;
            ensure(
                denied["result"]["isError"] == true && problem["code"] == "Forbidden",
                &format!("MCP must authorize {name} before decoding for {role}"),
            )?;
            let mut matched = false;
            for (path, methods) in paths {
                for (method, operation) in methods.as_object().into_iter().flatten() {
                    if operation["operationId"] != name {
                        continue;
                    }
                    matched = true;
                    let path = path
                        .split('/')
                        .map(|segment| {
                            if segment.starts_with('{') {
                                "%FF"
                            } else {
                                segment
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("/");
                    for bearer in [false, true] {
                        let client = if bearer { &context.client } else { &session };
                        let method =
                            reqwest::Method::from_bytes(method.to_ascii_uppercase().as_bytes())
                                .map_err(|error| support::ErrorKind::Check(error.to_string()))?;
                        let mut request = client
                            .request(
                                method,
                                format!("{}{path}?limit=invalid", context.environment.base_url),
                            )
                            .header("Origin", &context.environment.base_url)
                            .header("Content-Type", "application/json")
                            .body("{");
                        if bearer {
                            request = request.bearer_auth(token);
                        }
                        let problem: Value = send(request, 403).await?.json().await?;
                        ensure(
                            problem["code"] == "Forbidden",
                            &format!("REST permission mismatch for {role}: {name}"),
                        )?;
                    }
                }
            }
            ensure(matched, &format!("missing REST route for {name}"))?;
        }
        let actual: BTreeSet<_> = tools
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        ensure(
            actual == expected && actual.len() == tools.len(),
            &format!("incorrect visible catalog for {role}"),
        )?;
        context
            .api(
                "DELETE",
                &format!("/api/users/{}", string(&user, "id")?),
                None,
            )
            .await?;
    }
    Ok(())
}

pub(crate) async fn mcp(
    context: &Context,
    token: &str,
    method: &str,
    mut params: Value,
) -> Result<Value> {
    params["_meta"] = json!({"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}});
    let mut request = context
        .client
        .post(format!("{}/mcp", context.environment.base_url))
        .bearer_auth(token)
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", method);
    if let Some(name) = params["name"].as_str() {
        request = request.header("Mcp-Name", name);
    }
    let request = request.json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}));
    Ok(send(request, 200).await?.json().await?)
}

#[expect(clippy::too_many_lines)]
pub(crate) async fn protocols(context: &Context) -> Result<()> {
    let key = &context.credentials.api_key;
    let catalog = mcp(context, key, "tools/list", json!({})).await?;
    let tools = catalog["result"]["tools"]
        .as_array()
        .ok_or_else(|| support::ErrorKind::Check("missing MCP tools".into()))?;
    let names: BTreeSet<_> = tools
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    ensure(
        names.len() == tools.len() && names.len() > 50,
        "MCP catalog must be populated without duplicates",
    )?;
    ensure(
        ["secret_create", "secret_list", "secret_delete"]
            .iter()
            .all(|name| names.contains(name))
            && !names.contains("secret_get")
            && !names.contains("app_upgrade_tini"),
        "Secret catalog exposes only create, list and delete",
    )?;
    let openapi: Value = serde_json::from_str(&context.docker(&[
        "run",
        "--rm",
        &context.environment.images["cdelivery"].reference,
        "contract",
        "openapi",
    ])?)?;
    let ids: BTreeSet<_> = openapi["paths"]
        .as_object()
        .into_iter()
        .flat_map(|paths| paths.values())
        .filter_map(Value::as_object)
        .flat_map(|path| path.values())
        .filter_map(|operation| operation["operationId"].as_str())
        .collect();
    ensure(
        ids == names,
        "REST operation IDs and administrator MCP tool names must match",
    )?;
    let run = tools
        .iter()
        .find(|tool| tool["name"] == "xhealth_run")
        .ok_or_else(|| support::ErrorKind::Check("missing xhealth_run tool".into()))?;
    ensure(
        run["annotations"]["readOnlyHint"] != true
            && run["annotations"]["destructiveHint"] == true
            && run["annotations"]["idempotentHint"] == false
            && openapi["paths"]["/api/apps/{name}/xhealth/run"]["post"]["responses"]["202"]
                .is_object(),
        "XHealth execution must expose acceptance and non-idempotent write semantics",
    )?;
    let xjob = &openapi["components"]["schemas"]["CreateXJobRequest"];
    ensure(
        xjob["required"].as_array().is_some_and(|required| {
            ["interval", "timeout", "command"]
                .into_iter()
                .all(|field| required.contains(&json!(field)))
        }) && xjob["properties"].get("schedule").is_none(),
        "XJob schema must require interval, timeout and command without calendar scheduling",
    )?;
    let pingress = &openapi["components"]["schemas"]["CreatePingressRequest"];
    let enable = tools
        .iter()
        .find(|tool| tool["name"] == "pingress_enable")
        .ok_or_else(|| support::ErrorKind::Check("missing pingress_enable tool".into()))?;
    ensure(
        pingress["properties"]["acme"]["default"] == "LetsEncrypt"
            && pingress["required"]
                .as_array()
                .is_some_and(|fields| !fields.contains(&json!("acme")))
            && enable["inputSchema"]["properties"]["acme"]["default"] == "LetsEncrypt"
            && enable["annotations"]["destructiveHint"] == true
            && enable["annotations"]["idempotentHint"] == false,
        "Pingress creation must expose its ACME default and configuration side effects",
    )?;
    let reader = context
        .api(
            "POST",
            "/api/apikeys",
            Some(json!({"name":support::name("protocol"),"role":"Reader"})),
        )
        .await?;
    for (path, name) in [
        ("/api/groups", "group_list"),
        ("/api/versions", "version_list"),
    ] {
        let url = format!("{}{path}", context.environment.base_url);
        send(context.client.get(&url), 401).await?;
        let rest: Value = send(
            context
                .client
                .get(&url)
                .bearer_auth(string(&reader, "key")?),
            200,
        )
        .await?
        .json()
        .await?;
        let result = mcp(
            context,
            string(&reader, "key")?,
            "tools/call",
            json!({"name":name,"arguments":{}}),
        )
        .await?;
        ensure(
            serde_json::from_str::<Value>(string(&result["result"]["content"][0], "text")?)?
                == rest,
            "Reader REST and MCP must encode the same data",
        )?;
    }
    let denied = mcp(
        context,
        string(&reader, "key")?,
        "tools/call",
        json!({"name":"group_create","arguments":{"invalid":true}}),
    )
    .await?;
    ensure(
        denied["result"]["isError"] == true && denied.to_string().contains("Forbidden"),
        "MCP must authorize before decoding arguments",
    )?;
    send(
        context
            .client
            .post(format!(
                "{}/api/apps/%FF/xhealth/run",
                context.environment.base_url
            ))
            .bearer_auth(string(&reader, "key")?),
        403,
    )
    .await?;
    let denied = mcp(
        context,
        string(&reader, "key")?,
        "tools/call",
        json!({"name":"xhealth_run","arguments":{"invalid":true}}),
    )
    .await?;
    ensure(
        denied["result"]["isError"] == true && denied.to_string().contains("Forbidden"),
        "XHealth must authorize before decoding MCP input",
    )?;
    for (arguments, status) in [
        (json!({"app":"invalid!"}), 422),
        (json!({"app":"missing","force":true}), 400),
    ] {
        let invalid = mcp(
            context,
            key,
            "tools/call",
            json!({"name":"xhealth_run","arguments":arguments}),
        )
        .await?;
        let problem: Value =
            serde_json::from_str(string(&invalid["result"]["content"][0], "text")?)?;
        ensure(
            invalid["result"]["isError"] == true && problem["status"] == status,
            "XHealth must reject invalid names and unsupported run options",
        )?;
    }
    context
        .api(
            "DELETE",
            &format!("/api/apikeys/{}", string(&reader, "id")?),
            None,
        )
        .await?;
    let group = support::name("patch");
    let path = format!("/api/groups/{group}");
    context
        .api(
            "POST",
            "/api/groups",
            Some(json!({"name":group,"description":"initial"})),
        )
        .await?;
    let network = context.docker(&[
        "network",
        "inspect",
        "--format",
        "{{.Id}}",
        &format!("cdelivery-group-{group}"),
    ])?;
    let before = context.docker(&["config", "inspect", network.trim()])?;
    context.api("PATCH", &path, Some(json!({}))).await?;
    ensure(
        before == context.docker(&["config", "inspect", network.trim()])?,
        "empty patch must not bump the stored version",
    )?;
    context
        .api("PATCH", &path, Some(json!({"description":null})))
        .await?;
    ensure(
        context.api("GET", &path, None).await?["description"].is_null(),
        "null clears nullable description",
    )?;
    context
        .api("PATCH", &path, Some(json!({"description":"changed"})))
        .await?;
    ensure(
        context.api("GET", &path, None).await?["description"] == "changed",
        "patch must persist the new value",
    )?;
    for body in [json!({"unknown":true}), json!({"description":123})] {
        send(context.request("PATCH", &path)?.json(&body), 422).await?;
    }
    send(
        context
            .request("PATCH", "/api/me")?
            .json(&json!({"first_name":null})),
        422,
    )
    .await?;
    let request = context
        .request("POST", "/api/groups")?
        .json(&json!({"name":"../invalid"}));
    let response = send(request, 422).await?;
    ensure(
        response
            .headers()
            .get("content-type")
            .is_some_and(|value| value == "application/problem+json"),
        "invalid input must use problem+json",
    )?;
    let problem: Value = response.json().await?;
    ensure(
        problem["code"].is_string(),
        "problem must include a stable code",
    )?;
    context.api("DELETE", &path, None).await?;
    Ok(())
}
