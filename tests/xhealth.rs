use base64::Engine as _;
use jiff::{SignedDuration, Timestamp};
use serde_json::{Value, json};

use crate::support::{self, Context, Result, ensure, eventually, send, string};

async fn outcome(context: &Context, path: &str, expected: &str) -> Result<()> {
    eventually("XHealth target outcome", || async {
        let health = context.api("GET", path, None).await?;
        ensure(
            health["status"].get(expected).is_some(),
            &format!("expected {expected}: {health}"),
        )
    })
    .await
}

fn runtime_env(context: &Context, service: &str, expected: &[String]) -> Result<()> {
    let service = context.inspect_service(service)?;
    let env = &service[0]["Spec"]["TaskTemplate"]["ContainerSpec"]["Env"];
    ensure(
        env.as_array().is_some_and(|env| {
            expected
                .iter()
                .all(|value| env.iter().any(|entry| entry.as_str() == Some(value)))
        }),
        &format!("expected runtime variables {expected:?}, got {env}"),
    )
}

pub(crate) async fn domain(context: &Context) -> Result<()> {
    let name = support::name("health-domain");
    let domain = format!("{name}.cdelivery.test");
    let path = format!("/api/apps/{name}");
    let health_path = format!("{path}/xhealth");
    std::fs::write(
        context.root.join("domain-app.json"),
        serde_json::to_vec(&name)?,
    )?;
    context
        .api("POST", "/api/groups", Some(json!({"name":name})))
        .await?;
    context.api("POST", "/api/apps", Some(json!({
        "name":name,"group":name,"image":context.environment.images["fixture"].reference,
        "replicas":{"Fixed":1},
        "domains":[{"domain":domain,"https":false,"target":8080},{"domain":domain,"https":false,"target":8090,"path_prefix":"/other"}],
        "container_options":{"command":["sh","-c","mkdir -p /www; printf domain-through-ingress > /www/index.html; exec httpd -f -p 8080 -h /www"]}
    }))).await?;
    let rejected: Value = send(
        context.request("PUT", &health_path)?.json(&json!({
            "method":{"Custom":{"command":["true"]}},"target":{"Domain":domain},
            "interval":{"unit":"Hours","value":24},"release_check":false
        })),
        422,
    )
    .await?
    .json()
    .await?;
    ensure(
        rejected["code"] == "XHealthTargetAmbiguous",
        "multiple internal ports must remain invalid without release checks",
    )?;
    context.api("PATCH", &path, Some(json!({
        "domains":[{"domain":domain,"https":false,"target":8080},{"domain":domain,"https":false,"target":8080,"path_prefix":"/other"}]
    }))).await?;
    context.ready(&name, 1).await?;
    context.docker(&[
        "service",
        "update",
        "--detach=false",
        "--network-add",
        &format!("name=cdelivery-group-{name},alias={domain}"),
        "cdelivery-pingress",
    ])?;
    context.api("PUT", &health_path, Some(json!({
        "method":{"Custom":{"command":["sh","-c","test \"$TARGET_SCHEME:$TARGET_PORT\" = http:80 && test \"$(wget -qO- \"$TARGET_SCHEME://$TARGET_HOST:$TARGET_PORT/\")\" = domain-through-ingress"]}},
        "target":{"Domain":domain},"interval":{"unit":"Hours","value":24},"release_check":false
    }))).await?;
    outcome(context, &health_path, "Passed").await?;
    let file = "GET {{TARGET_URL}}\nHTTP 200\n[Asserts]\nbody == \"domain-through-ingress\"\n";
    context
        .api(
            "PATCH",
            &health_path,
            Some(json!({
                "method":{"Hurl":{"file":file,"version":context.environment.images["hurl"].version}}
            })),
        )
        .await?;
    outcome(context, &health_path, "Passed").await?;
    let certificate = rcgen::generate_simple_self_signed(vec![domain.clone()])?;
    let unrelated = rcgen::generate_simple_self_signed(vec!["unrelated.cdelivery.test".into()])?;
    context.api("PUT", &format!("/api/pingress/certs/{domain}"), Some(json!({
        "cert_chain_pem":certificate.cert.pem(),"private_key_pem":certificate.signing_key.serialize_pem()
    }))).await?;
    context.api("PATCH", &path, Some(json!({
        "domains":[{"domain":domain,"https":true,"target":8080}],
        "xconfigs":[
            {"Upload":{"target":"/health-ca.pem","data_base64":base64::engine::general_purpose::STANDARD.encode(certificate.cert.pem())}},
            {"Upload":{"target":"/health-unrelated-ca.pem","data_base64":base64::engine::general_purpose::STANDARD.encode(unrelated.cert.pem())}}
        ]
    }))).await?;
    context.ready(&name, 1).await?;
    for (ca, expected) in [
        ("/health-ca.pem", "Passed"),
        ("/health-unrelated-ca.pem", "Failed"),
        ("/health-ca.pem", "Passed"),
    ] {
        let file = format!(
            "GET {{{{TARGET_URL}}}}\n[Options]\ncacert: {ca}\nHTTP 200\n[Asserts]\nbody == \"domain-through-ingress\"\n"
        );
        context
            .api(
                "PATCH",
                &health_path,
                Some(json!({
                    "method":{"Hurl":{"file":file}}
                })),
            )
            .await?;
        outcome(context, &health_path, expected).await?;
        runtime_env(
            context,
            &format!("cdelivery-xhealth-{name}"),
            &[format!("HURL_VARIABLE_TARGET_URL=https://{domain}:443")],
        )?;
    }
    Ok(())
}

pub(crate) async fn restore_domain(context: &Context) -> Result<()> {
    let saved = context.root.join("domain-app.json");
    if !saved.try_exists()? {
        return Ok(());
    }
    let name: String = serde_json::from_slice(&std::fs::read(saved)?)?;
    let domain = format!("{name}.cdelivery.test");
    let gateway = context.inspect_service("cdelivery-pingress")?;
    if gateway[0]["Spec"]["TaskTemplate"]["Networks"]
        .as_array()
        .is_some_and(|networks| {
            networks.iter().any(|network| {
                network["Aliases"].as_array().is_some_and(|aliases| {
                    aliases
                        .iter()
                        .any(|alias| alias.as_str() == Some(domain.as_str()))
                })
            })
        })
    {
        context.docker(&[
            "service",
            "update",
            "--detach=false",
            "--network-rm",
            &format!("cdelivery-group-{name}"),
            "cdelivery-pingress",
        ])?;
    }
    let deleted = context
        .request("DELETE", &format!("/api/apps/{name}"))?
        .send()
        .await?;
    if deleted.status().as_u16() != 404 {
        deleted.error_for_status()?;
    }
    context
        .api("DELETE", &format!("/api/pingress/certs/{domain}"), None)
        .await?;
    eventually("domain test group cleanup", || async {
        let deleted = context
            .request("DELETE", &format!("/api/groups/{name}"))?
            .send()
            .await?;
        if deleted.status().as_u16() == 409 {
            let error: Value = deleted.json().await?;
            ensure(
                error["code"] != "GroupBusy",
                "domain test network is still in use",
            )?;
            return Err(support::ErrorKind::Api(error.to_string()).into());
        }
        deleted.error_for_status()?;
        Ok(())
    })
    .await
}

#[expect(clippy::too_many_lines)]
pub(crate) async fn targets(context: &Context) -> Result<()> {
    let name = support::name("health-targets");
    let path = format!("/api/apps/{name}");
    let health_path = format!("{path}/xhealth");
    let service = format!("cdelivery-xhealth-{name}");
    let domain = format!("{name}.cdelivery.test");
    let peer = support::name("health-peer");
    let peer_path = format!("/api/apps/{peer}");
    let command = [
        "sh",
        "-c",
        "mkdir -p /www; printf healthy > /www/index.html; httpd -p 8080 -h /www; while true; do printf pong | nc -l -p 8090; done",
    ];
    context.api("POST", "/api/apps", Some(json!({
        "name":name,"group":"tests","image":context.environment.images["fixture"].reference,
        "replicas":{"Fixed":1},"container_options":{"command":command},
        "ports":[{"published":18081,"target":8080,"protocol":"Tcp"},{"published":18082,"target":8090,"protocol":"Tcp"},{"published":18083,"target":8091,"protocol":"Udp"}],
        "domains":[{"domain":domain,"https":false,"target":8080}]
    }))).await?;
    context.ready(&name, 1).await?;
    let nodes = context.engine("GET", "/nodes", &Value::Null)?;
    let node = nodes
        .as_array()
        .into_iter()
        .flatten()
        .filter(|node| node["Status"]["State"] == "ready")
        .filter(|node| {
            node["Status"]["Addr"]
                .as_str()
                .and_then(|addr| addr.parse::<std::net::IpAddr>().ok())
                .is_some_and(|addr| !addr.is_loopback() && !addr.is_unspecified())
        })
        .min_by_key(|node| node["ID"].as_str())
        .ok_or_else(|| support::ErrorKind::Check("no eligible node address".into()))?;
    let host = string(&node["Status"], "Addr")?;
    let http = "test -z \"${TARGET_SCHEME+x}\" && test \"$(wget -qO- \"http://$TARGET_HOST:$TARGET_PORT/\")\" = healthy";
    context.api("PUT", &health_path, Some(json!({
        "method":{"Custom":{"command":["sh","-c",http]}},"interval":{"unit":"Hours","value":24},
        "target":{"PublishedPort":{"published":18081}},"release_check":false
    }))).await?;
    outcome(context, &health_path, "Passed").await?;
    runtime_env(
        context,
        &service,
        &[format!("TARGET_HOST={host}"), "TARGET_PORT=18081".into()],
    )?;
    let hurl = json!({"Hurl":{"file":"GET {{TARGET_URL}}\nHTTP 200\n[Asserts]\nbody == \"healthy\"\n","version":context.environment.images["hurl"].version}});
    context
        .api("PATCH", &health_path, Some(json!({"method":hurl})))
        .await?;
    outcome(context, &health_path, "Passed").await?;
    runtime_env(
        context,
        &service,
        &[format!("HURL_VARIABLE_TARGET_URL=http://{host}:18081")],
    )?;
    let rejected: Value = send(
        context.request("PATCH", &health_path)?.json(&json!({
            "target":{"PublishedPort":{"published":18083}}
        })),
        422,
    )
    .await?
    .json()
    .await?;
    ensure(
        rejected["code"] == "XHealthTargetNotFound",
        "Hurl must reject a UDP-only mapping",
    )?;
    context.api("PATCH", &health_path, Some(json!({
        "method":{"Custom":{"command":["sh","-c","test -z \"${TARGET_SCHEME+x}\" && test \"$(nc -w 5 \"$TARGET_HOST\" \"$TARGET_PORT\" </dev/null)\" = pong"]}},
        "target":{"PublishedPort":{"published":18082}}
    }))).await?;
    outcome(context, &health_path, "Passed").await?;
    context.app(&peer, "peer").await?;
    for (https, port, scheme) in [(false, "80", "http"), (true, "443", "https")] {
        context
            .api(
                "PATCH",
                &peer_path,
                Some(json!({
                    "domains":[{"domain":domain,"https":https,"target":8080,"path_prefix":"/peer"}]
                })),
            )
            .await?;
        context.api("PATCH", &health_path, Some(json!({
            "target":{"Domain":domain},
            "method":{"Custom":{"command":["sh","-c","test \"$TARGET_HOST\" = \"$1\" && test \"$TARGET_PORT\" = \"$2\" && test \"$TARGET_SCHEME\" = \"$3\"","check",domain,port,scheme]}}
        }))).await?;
        outcome(context, &health_path, "Passed").await?;
        runtime_env(
            context,
            &service,
            &[
                format!("TARGET_HOST={domain}"),
                format!("TARGET_PORT={port}"),
                format!("TARGET_SCHEME={scheme}"),
            ],
        )?;
        context
            .api("PATCH", &health_path, Some(json!({"method":hurl})))
            .await?;
        outcome(context, &health_path, "Failed").await?;
        runtime_env(
            context,
            &service,
            &[format!(
                "HURL_VARIABLE_TARGET_URL={scheme}://{domain}:{port}"
            )],
        )?;
    }
    context
        .api(
            "PATCH",
            &health_path,
            Some(json!({
                "method":{"Custom":{"command":["sh","-c","test \"$TARGET_SCHEME\" = https"]}}
            })),
        )
        .await?;
    outcome(context, &health_path, "Passed").await?;
    let secret_name = support::name("health-scheme");
    context
        .api(
            "POST",
            "/api/secrets",
            Some(json!({
                "name":secret_name,"content":{"Text":"custom-protocol"}
            })),
        )
        .await?;
    let secret = crate::secrets::metadata(context, &secret_name).await?;
    let reference = json!({"name":secret_name,"id":secret["id"]});
    let dependency = json!({"dependency":{"App":peer},"field":"host"});
    for value in [&reference, &dependency] {
        let rejected: Value = send(context.request("PATCH", &path)?.json(&json!({
            "container_options":{"command":command,"env":[{"name":"TARGET_SCHEME","value":value}]}
        })), 422).await?.json().await?;
        ensure(
            rejected["code"] == "SecretBindingInvalid",
            "Domain Custom must reject a reserved Secret or Dependency binding",
        )?;
    }
    context
        .api(
            "PATCH",
            &health_path,
            Some(json!({
                "target":{"InternalPort":{"port":8080}},
                "method":{"Custom":{"command":["sh","-c",http]}}
            })),
        )
        .await?;
    outcome(context, &health_path, "Passed").await?;
    for (value, expected) in [
        (json!("user-protocol"), "user-protocol"),
        (reference.clone(), "custom-protocol"),
        (dependency, peer.as_str()),
    ] {
        context.api("PATCH", &path, Some(json!({
            "container_options":{"command":command,"env":[{"name":"TARGET_SCHEME","value":value}]}
        }))).await?;
        context.ready(&name, 1).await?;
        if value.is_string() {
            context.api("PATCH", &health_path, Some(json!({
                "target":{"Domain":domain},
                "method":{"Custom":{"command":["sh","-c","test \"$TARGET_SCHEME\" = https"]}}
            }))).await?;
            outcome(context, &health_path, "Passed").await?;
        }
        context.api("PATCH", &health_path, Some(json!({
            "target":{"InternalPort":{"port":8080}},
            "method":{"Custom":{"command":["sh","-c","test \"$TARGET_SCHEME\" = \"$1\" && test \"$(wget -qO- \"http://$TARGET_HOST:$TARGET_PORT/\")\" = healthy","check",expected]}}
        }))).await?;
        outcome(context, &health_path, "Passed").await?;
    }
    context.api("DELETE", &path, None).await?;
    context.api("DELETE", &peer_path, None).await?;

    for (binding, expected_error) in [
        (false, "XHealthTargetNotFound"),
        (true, "SecretBindingInvalid"),
    ] {
        let name = support::name("health-rollback");
        let path = format!("/api/apps/{name}");
        let health_path = format!("{path}/xhealth");
        let service = format!("cdelivery-xhealth-{name}");
        let domain = format!("{name}.cdelivery.test");
        let domains = json!([{"domain":domain,"https":false,"target":8080}]);
        context.api("POST", "/api/apps", Some(json!({
            "name":name,"group":"tests","image":context.environment.images["fixture"].reference,
            "replicas":{"Fixed":1},"domains":if binding { domains.clone() } else { json!([]) },
            "container_options":{"command":["sleep","86400"],"env":if binding { json!([{"name":"TARGET_SCHEME","value":reference}]) } else { json!([]) }}
        }))).await?;
        context.ready(&name, 1).await?;
        context
            .api(
                "PATCH",
                &path,
                Some(json!({
                    "domains":domains,"container_options":{"command":["sleep","86400"],"env":[]}
                })),
            )
            .await?;
        context.ready(&name, 1).await?;
        context.api("PUT", &health_path, Some(json!({
            "method":{"Custom":{"command":["sh","-c","test \"$TARGET_SCHEME\" = http"]}},
            "interval":{"unit":"Hours","value":24},"target":{"Domain":domain},"release_check":false
        }))).await?;
        outcome(context, &health_path, "Passed").await?;
        context
            .api("POST", &format!("{path}/rollback"), None)
            .await?;
        context.ready(&name, 1).await?;
        let previous = context.api("GET", &health_path, None).await?;
        let before = context.inspect_service(&service)?;
        let rejected: Value = send(context.request("POST", &format!("{health_path}/run"))?, 422)
            .await?
            .json()
            .await?;
        ensure(
            rejected["code"] == expected_error,
            &format!("rollback must reject stale health configuration: {rejected}"),
        )?;
        let after = context.inspect_service(&service)?;
        ensure(
            before[0]["Version"] == after[0]["Version"]
                && before[0]["Spec"] == after[0]["Spec"]
                && context.api("GET", &health_path, None).await?["status"] == previous["status"],
            "rejected round must preserve the check service and previous outcome",
        )?;
        context.api("DELETE", &path, None).await?;
    }
    eventually("unused scheme Secret cleanup", || async {
        context
            .api("DELETE", &format!("/api/secrets/{secret_name}"), None)
            .await?;
        Ok(())
    })
    .await?;
    Ok(())
}

#[expect(clippy::too_many_lines)]
pub(crate) async fn run_now(context: &Context) -> Result<()> {
    let name = support::name("xhealth");
    let path = format!("/api/apps/{name}");
    let health_path = format!("{path}/xhealth");
    let run_path = format!("{health_path}/run");
    let service = format!("cdelivery-xhealth-{name}");
    let missing: Value = send(context.request("POST", &run_path)?, 404)
        .await?
        .json()
        .await?;
    ensure(missing["code"] == "AppNotFound", "missing app error")?;
    context.api("POST", "/api/apps", Some(json!({
        "name":name,"group":"tests","image":context.environment.images["fixture"].reference,
        "replicas":{"Fixed":1},"log_driver":{"driver":"Insight"},
        "container_options":{"command":["sh","-c","mkdir -p /www; printf healthy > /www/index.html; exec httpd -f -p 8080 -h /www"]}
    }))).await?;
    context.ready(&name, 1).await?;
    let disabled: Value = send(context.request("POST", &run_path)?, 404)
        .await?
        .json()
        .await?;
    ensure(
        disabled["code"] == "XHealthNotEnabled",
        "running must not implicitly enable XHealth",
    )?;
    let operator = context
        .api(
            "POST",
            "/api/apikeys",
            Some(json!({
                "name":support::name("xhealth-operator"),"role":"Operator"
            })),
        )
        .await?;
    let spec = json!({
        "method":{"Custom":{"command":["sh","-c","echo checking; sleep 45; wget -q -O - \"http://${TARGET_HOST}:${TARGET_PORT}/\""]}},
        "interval":{"unit":"Minutes","value":1},
        "target":{"InternalPort":{"port":8080}},
        "release_check":false
    });
    context.api("PUT", &health_path, Some(spec)).await?;
    let busy: Value = send(context.request("POST", &run_path)?, 409)
        .await?
        .json()
        .await?;
    ensure(
        busy["code"] == "XHealthRunning",
        "pending or running checks must reject another run",
    )?;
    eventually("initial check", || async {
        let state = context.api("GET", &health_path, None).await?;
        ensure(
            state["status"].get("Passed").is_some(),
            "initial check has not passed",
        )
    })
    .await?;
    let before: Value = serde_json::from_str(&context.docker(&["service", "inspect", &service])?)?;
    ensure(
        before[0]["Spec"]["TaskTemplate"]["ForceUpdate"]
            .as_u64()
            .unwrap_or(0)
            == 0,
        "creation must start the initial round",
    )?;
    context.api("PATCH", &health_path, Some(json!({}))).await?;
    let invalid: Value = send(
        context.request("PATCH", &health_path)?.json(&json!({
            "target":{"PublishedPort":{"published":8080}}
        })),
        422,
    )
    .await?
    .json()
    .await?;
    ensure(
        invalid["code"] == "XHealthTargetNotFound",
        "missing port mapping",
    )?;
    let unchanged: Value =
        serde_json::from_str(&context.docker(&["service", "inspect", &service])?)?;
    ensure(
        unchanged[0]["Version"] == before[0]["Version"]
            && unchanged[0]["Spec"] == before[0]["Spec"],
        "empty updates and invalid targets must preserve configuration and round",
    )?;
    let (a, b) = tokio::join!(
        context.request("POST", &run_path)?.send(),
        context.request("POST", &run_path)?.send()
    );
    let mut codes = Vec::new();
    for response in [a?, b?] {
        let code = response.status().as_u16();
        let body: Value = response.json().await?;
        if code == 202 {
            ensure(body == json!({"ok":true}), "acceptance must return Done")?;
        } else {
            ensure(
                body["code"] == "Conflict" || body["code"] == "XHealthRunning",
                "competing run must report a version conflict or active check",
            )?;
        }
        codes.push(code);
    }
    codes.sort_unstable();
    ensure(
        codes == [202, 409],
        "only one competing request may start a round",
    )?;
    let active = eventually("manual check running", || async {
        let state = context.api("GET", &health_path, None).await?;
        ensure(
            state["status"].get("Running").is_some(),
            "check has not started",
        )?;
        Ok(state)
    })
    .await?;
    let started: Timestamp =
        serde_json::from_value(active["status"]["Running"]["started_at"].clone())?;
    let submitted: Value =
        serde_json::from_str(&context.docker(&["service", "inspect", &service])?)?;
    ensure(
        submitted[0]["Spec"]["Labels"] == before[0]["Spec"]["Labels"]
            && submitted[0]["Spec"]["TaskTemplate"]["ForceUpdate"] == 1,
        "manual run must preserve configuration and submit exactly one round",
    )?;
    send(context.request("POST", &run_path)?, 409).await?;
    context.docker(&["service", "update", "--detach=true", "--force", "cdelivery"])?;
    eventually(
        "controller recovery across the previous schedule",
        || async {
            let state = context.api("GET", &health_path, None).await?;
            let current: Value =
                serde_json::from_str(&context.docker(&["service", "inspect", &service])?)?;
            ensure(
                current[0]["Spec"]["TaskTemplate"]["ForceUpdate"] == 1
                    && state["status"]["Running"]["started_at"]
                        == active["status"]["Running"]["started_at"],
                "restart and the old scheduled time must preserve the manual round",
            )?;
            ensure(
                Timestamp::now().duration_since(started) >= SignedDuration::from_secs(30),
                "waiting past the previous scheduled time",
            )
        },
    )
    .await?;
    eventually("next periodic check", || async {
        let state = context.api("GET", &health_path, None).await?;
        let next = state["status"]["Running"]["started_at"].as_str();
        ensure(next.is_some(), "next check is not running")?;
        let next: Timestamp =
            serde_json::from_value(state["status"]["Running"]["started_at"].clone())?;
        ensure(
            next.duration_since(started) >= SignedDuration::from_mins(1),
            "next periodic round must wait from the manual round's start",
        )?;
        let current: Value =
            serde_json::from_str(&context.docker(&["service", "inspect", &service])?)?;
        ensure(
            current[0]["Spec"]["TaskTemplate"]["ForceUpdate"] == 2,
            "periodic scheduling must submit only one next round",
        )
    })
    .await?;
    context
        .api(
            "PATCH",
            &health_path,
            Some(json!({
                "method":{"Custom":{"command":["false"]}},
                "interval":{"unit":"Hours","value":24}
            })),
        )
        .await?;
    eventually("failed check and alert", || async {
        let state = context.api("GET", &health_path, None).await?;
        let events = context
            .api("GET", &format!("/api/events?subject={name}"), None)
            .await?;
        ensure(
            state["status"].get("Failed").is_some()
                && events["records"].as_array().is_some_and(|records| {
                    records.iter().any(|event| event["kind"] == "XHealthFailed")
                }),
            "failed check must remain observable",
        )
    })
    .await?;
    let result = crate::contracts::mcp(
        context,
        string(&operator, "key")?,
        "tools/call",
        json!({"name":"xhealth_run","arguments":{"app":name}}),
    )
    .await?;
    ensure(
        result["result"]["isError"] != true
            && serde_json::from_str::<Value>(string(&result["result"]["content"][0], "text")?)?
                == json!({"ok":true}),
        "MCP must accept a new round after a failed check",
    )?;
    eventually("manual trigger audit", || async {
        let events = context
            .api("GET", &format!("/api/events?subject={name}"), None)
            .await?;
        let triggers: Vec<_> = events["records"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|event| event["kind"] == "XHealthTriggered")
            .collect();
        ensure(
            triggers.len() == 2
                && triggers.iter().all(|event| {
                    event["actor"]
                        .as_str()
                        .is_some_and(|actor| !actor.is_empty() && actor != "system")
                })
                && triggers.iter().any(|event| {
                    event["actor"]
                        .as_str()
                        .is_some_and(|actor| actor.ends_with(" via mcp"))
                }),
            "only successful manual submissions must emit actor-attributed events",
        )
    })
    .await?;
    context
        .api(
            "PATCH",
            &health_path,
            Some(json!({"method":{"Custom":{"command":["true"]}}})),
        )
        .await?;
    eventually("check recovery", || async {
        let state = context.api("GET", &health_path, None).await?;
        let events = context
            .api("GET", &format!("/api/events?subject={name}"), None)
            .await?;
        ensure(
            state["status"].get("Passed").is_some()
                && events["records"].as_array().is_some_and(|records| {
                    records
                        .iter()
                        .any(|event| event["kind"] == "XHealthRecovered")
                }),
            "successful check must clear the existing failure",
        )
    })
    .await?;
    eventually("XHealth check output", || async {
        let logs = context
            .api("GET", &format!("{path}/logs?target=XHealth"), None)
            .await?;
        ensure(
            logs.to_string().contains("checking"),
            "check output must remain available through app logs",
        )
    })
    .await?;
    context.api("DELETE", &path, None).await?;
    context
        .api(
            "DELETE",
            &format!("/api/apikeys/{}", string(&operator, "id")?),
            None,
        )
        .await?;
    Ok(())
}
