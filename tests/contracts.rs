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
        ("change_apply", "UserOperator"),
        ("change_close", "UserOperator"),
        ("change_complete", "UserOperator"),
        ("change_create", "Operator"),
        ("dependency_create", "Admin"),
        ("dependency_planetscale_sizes", "Shared"),
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
                "Operator" | "UserOperator" => role != "Reader",
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
    let sizes = context
        .api("GET", "/api/dependencies/.planetscale-sizes", None)
        .await?;
    ensure(
        sizes
            == json!({"sizes":[
                {"name":"PS-5","memory_bytes":536_870_912},
                {"name":"PS-10","memory_bytes":null},
                {"name":"PS-20","memory_bytes":2_147_483_648_u64}
            ]}),
        "PlanetScale sizes must exclude Metal, group variants, omit differing memory and sort by minimum memory",
    )?;
    let response = mcp(
        context,
        key,
        "tools/call",
        json!({"name":"dependency_planetscale_sizes","arguments":{}}),
    )
    .await?;
    let mcp_sizes: Value =
        serde_json::from_str(string(&response["result"]["content"][0], "text")?)?;
    ensure(
        mcp_sizes == sizes,
        "REST and MCP PlanetScale sizes must match",
    )?;
    let tool = |name: &str| -> Result<&Value> {
        tools
            .iter()
            .find(|tool| tool["name"] == name)
            .ok_or_else(|| support::ErrorKind::Check(format!("missing {name} tool")).into())
    };
    let resolve = |document: &Value, schema: &Value| -> Result<Value> {
        if let Some(reference) = schema["$ref"].as_str() {
            reference
                .strip_prefix('#')
                .and_then(|pointer| document.pointer(pointer))
                .cloned()
                .ok_or_else(|| {
                    support::ErrorKind::Check(format!("unresolved schema reference: {reference}"))
                        .into()
                })
        } else {
            ensure(schema.is_object(), "missing schema object")?;
            Ok(schema.clone())
        }
    };
    for (name, path, required) in [
        ("secret_create", "/api/secrets", &["name", "content"][..]),
        (
            "dependency_create",
            "/api/dependencies",
            &["name", "provider", "dsn"][..],
        ),
        ("change_create", "/api/changes", &["dependency", "sql"][..]),
    ] {
        let input = &tool(name)?["inputSchema"];
        for (document, schema) in [
            (
                &openapi,
                &openapi["paths"][path]["post"]["requestBody"]["content"]["application/json"]["schema"],
            ),
            (input, input),
        ] {
            let schema = resolve(document, schema)?;
            ensure(
                schema["required"].as_array().is_some_and(|fields| {
                    fields.len() == required.len()
                        && required.iter().all(|field| {
                            fields.contains(&json!(field))
                                && schema["properties"].get(field).is_some()
                        })
                }),
                &format!("{name} must retain its own request fields"),
            )?;
        }
    }
    for (name, path, method, status, list) in [
        ("change_create", "/api/changes", "post", "201", false),
        ("change_get", "/api/changes/{id}", "get", "200", false),
        ("change_list", "/api/changes", "get", "200", true),
    ] {
        let output = &tool(name)?["outputSchema"];
        for (document, schema) in [
            (
                &openapi,
                &openapi["paths"][path][method]["responses"][status]["content"]["application/json"]
                    ["schema"],
            ),
            (output, output),
        ] {
            let mut change = resolve(document, schema)?;
            if list {
                ensure(
                    change["properties"]["changes"]["type"] == "array",
                    "change_list must return a change list",
                )?;
                change = resolve(document, &change["properties"]["changes"]["items"])?;
            }
            ensure(
                change["properties"]["id"]["format"] == "uuid"
                    && change["required"].as_array().is_some_and(|required| {
                        ["id", "target", "apps", "stage", "state", "actions"]
                            .iter()
                            .all(|field| {
                                required.contains(&json!(field))
                                    && change["properties"].get(field).is_some()
                            })
                    }),
                &format!("{name} must return release changes"),
            )?;
            let target = resolve(document, &change["properties"]["target"])?;
            ensure(
                target["oneOf"].as_array().is_some_and(|variants| {
                    variants.len() == 2
                        && ["App", "PostgreSQL"].iter().all(|name| {
                            variants
                                .iter()
                                .any(|variant| variant["properties"].get(name).is_some())
                        })
                }),
                "release targets must be App or PostgreSQL",
            )?;
            let app = target["oneOf"]
                .as_array()
                .and_then(|variants| {
                    variants
                        .iter()
                        .find_map(|variant| variant.pointer("/properties/App/properties/changes"))
                })
                .ok_or_else(|| support::ErrorKind::Check("missing App target changes".into()))?;
            let patch = resolve(document, app)?;
            ensure(
                patch["properties"].get("image").is_some()
                    && patch["properties"].get("container_options").is_some()
                    && patch["properties"].get("changes").is_none(),
                "App release targets must contain the app patch schema",
            )?;
        }
    }
    let app_input = &tool("app_update")?["inputSchema"];
    for (document, schema) in [
        (
            &openapi,
            &openapi["paths"]["/api/apps/{name}"]["patch"]["requestBody"]["content"]["application/json"]
                ["schema"],
        ),
        (app_input, &app_input["properties"]["changes"]),
    ] {
        let patch = resolve(document, schema)?;
        ensure(
            patch["properties"].get("image").is_some()
                && patch["properties"].get("changes").is_none(),
            "app_update must retain its patch schema",
        )?;
    }
    let app_output = &tool("app_get")?["outputSchema"];
    for (document, schema) in [
        (
            &openapi,
            &openapi["paths"]["/api/apps/{name}"]["get"]["responses"]["200"]["content"]["application/json"]
                ["schema"],
        ),
        (app_output, app_output),
    ] {
        let app = resolve(document, schema)?;
        let rollback = resolve(document, &app["properties"]["rollback"]["oneOf"][0])?;
        let history = resolve(document, &rollback["oneOf"][0]["properties"]["Available"])?;
        let patch = resolve(document, &history["properties"]["changes"])?;
        ensure(
            history["properties"].get("operator").is_some()
                && history["properties"].get("id").is_none()
                && patch["properties"].get("image").is_some(),
            "app history must retain its operator and app patch",
        )?;
    }
    for (name, method, field) in [
        ("xhealth_get", "get", ""),
        ("xhealth_create", "put", "spec"),
        ("xhealth_update", "patch", "changes"),
    ] {
        let operation = &openapi["paths"]["/api/apps/{name}/xhealth"][method];
        let (schema, mcp_schema) = if field.is_empty() {
            (
                &operation["responses"]["200"]["content"]["application/json"]["schema"],
                &tool(name)?["outputSchema"],
            )
        } else {
            (
                &operation["requestBody"]["content"]["application/json"]["schema"],
                &tool(name)?["inputSchema"],
            )
        };
        let input = if field.is_empty() {
            mcp_schema
        } else {
            &mcp_schema["properties"][field]
        };
        for (document, schema) in [(&openapi, schema), (mcp_schema, input)] {
            let health = resolve(document, schema)?;
            let target = resolve(document, &health["properties"]["target"])?;
            ensure(
                target["oneOf"].as_array().is_some_and(|variants| {
                    variants.len() == 3
                        && ["Domain", "PublishedPort", "InternalPort"]
                            .iter()
                            .all(|name| {
                                variants
                                    .iter()
                                    .any(|variant| variant["properties"].get(name).is_some())
                            })
                }),
                &format!("{name} must retain health-check targets"),
            )?;
        }
    }
    for (name, path, collection) in [
        ("secret_list", "/api/secrets", "secrets"),
        ("event_list", "/api/events", "records"),
        ("event_conditions", "/api/events/conditions", "conditions"),
    ] {
        let operation = &openapi["paths"][path]["get"];
        let output = &tool(name)?["outputSchema"];
        let mut kinds = Vec::new();
        for (document, schema) in [
            (
                &openapi,
                &operation["responses"]["200"]["content"]["application/json"]["schema"],
            ),
            (output, output),
        ] {
            let schema = resolve(document, schema)?;
            let item = resolve(document, &schema["properties"][collection]["items"])?;
            kinds.push(resolve(document, &item["properties"]["kind"])?);
        }
        if name != "event_conditions" {
            let query = operation["parameters"]
                .as_array()
                .and_then(|parameters| {
                    parameters
                        .iter()
                        .find(|parameter| parameter["name"] == "kind")
                })
                .ok_or_else(|| {
                    support::ErrorKind::Check(format!("missing {name} kind parameter"))
                })?;
            kinds.push(resolve(&openapi, &query["schema"])?);
            let input = &tool(name)?["inputSchema"];
            let query = if name == "event_list" {
                resolve(input, &input["properties"]["query"])?
            } else {
                input.clone()
            };
            kinds.push(resolve(input, &query["properties"]["kind"]["oneOf"][0])?);
        }
        for kind in kinds {
            ensure(
                if name == "secret_list" {
                    kind["enum"] == json!(["Text", "File"])
                } else {
                    kind["enum"].as_array().is_some_and(|variants| {
                        ["AppCreated", "ChangeSubmitted", "XHealthFailed"]
                            .iter()
                            .all(|variant| variants.contains(&json!(variant)))
                            && !variants.contains(&json!("Text"))
                            && !variants.contains(&json!("File"))
                    })
                },
                &format!("{name} must retain its own kind enum"),
            )?;
        }
    }
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
