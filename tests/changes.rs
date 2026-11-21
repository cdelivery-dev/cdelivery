use serde_json::json;

use crate::support::{self, Context, Result, ensure, eventually, send};

pub(crate) async fn targets(context: &Context) -> Result<()> {
    for domain in [false, true] {
        let name = support::name("preview-target");
        let path = format!("/api/apps/{name}");
        let health_path = format!("{path}/xhealth");
        let hostname = format!("{name}.cdelivery.test");
        context.app(&name, "live-only").await?;
        context
            .api(
                "PATCH",
                &path,
                Some(if domain {
                    json!({"domains":[{"domain":hostname,"https":true,"target":8080}]})
                } else {
                    json!({"ports":[{"published":18091,"target":8080,"protocol":"Tcp"}]})
                }),
            )
            .await?;
        context.ready(&name, 1).await?;
        let method = if domain {
            json!({"Custom":{"command":["sh","-c","test \"$TARGET_PORT:$TARGET_SCHEME\" = 8080:http && test \"$(wget -q -O - \"$TARGET_SCHEME://$TARGET_HOST:$TARGET_PORT/\")\" = preview-only"]}})
        } else {
            json!({"Hurl":{"file":"GET {{TARGET_URL}}\nHTTP 200\n[Asserts]\nbody == \"preview-only\"\n","version":context.environment.images["hurl"].version}})
        };
        let target = if domain {
            json!({"Domain":hostname})
        } else {
            json!({"PublishedPort":{"published":18091}})
        };
        context
            .api(
                "PUT",
                &health_path,
                Some(json!({
                    "method":method,"target":target,
                    "interval":{"unit":"Hours","value":24},"release_check":true
                })),
            )
            .await?;
        eventually("live endpoint differs from preview", || async {
            ensure(
                context.api("GET", &health_path, None).await?["status"]
                    .get("Failed")
                    .is_some(),
                "the preview-only assertion must reject the live endpoint",
            )
        })
        .await?;
        let command = json!([
            "sh",
            "-c",
            "mkdir -p /www; printf preview-only > /www/index.html; exec httpd -f -p 8080 -h /www"
        ]);
        let accepted = send(
            context
                .request("PATCH", &path)?
                .json(&json!({"container_options":{"command":command}})),
            202,
        )
        .await?
        .json::<serde_json::Value>()
        .await?;
        let change_path = format!("/api/changes/{}", support::string(&accepted, "change")?);
        eventually("preview endpoint approves release", || async {
            let change = context.api("GET", &change_path, None).await?;
            ensure(
                change["state"] == "Completed"
                    && change["apps"] == json!([{"name":name,"check":true,"result":"Passed"}]),
                "release checks must use the preview's HTTP listener and response body",
            )
        })
        .await?;
        ensure(
            context.api("GET", &path, None).await?["container_options"]["command"] == command,
            "only the successfully checked preview may be promoted",
        )?;
        context.api("DELETE", &path, None).await?;
    }
    Ok(())
}

#[expect(clippy::too_many_lines)]
pub(crate) async fn decisions(context: &Context) -> Result<()> {
    for decision in ["close", "apply"] {
        let name = support::name("change");
        let path = format!("/api/apps/{name}");
        let health_path = format!("{path}/xhealth");
        context.app(&name, "healthy").await?;
        context.ready(&name, 1).await?;
        let file = "GET {{TARGET_URL}}\nHTTP 200\n[Asserts]\nbody == \"healthy\"\n";
        context.api("PUT", &health_path, Some(json!({
            "method":{"Hurl":{"file":file,"version":context.environment.images["hurl"].version}},
            "interval":{"unit":"Hours","value":24},
            "target":{"InternalPort":{"port":8080}},"release_check":true
        }))).await?;
        eventually("live health passes", || async {
            ensure(
                context.api("GET", &health_path, None).await?["status"]
                    .get("Passed")
                    .is_some(),
                "live app must be healthy before the preview",
            )
        })
        .await?;
        let live = context.api("GET", &path, None).await?;
        let command = json!([
            "sh",
            "-c",
            "mkdir -p /www; printf broken > /www/index.html; exec httpd -f -p 8080 -h /www"
        ]);
        let payload = json!({"container_options":{"command":command}});
        let accepted = send(context.request("PATCH", &path)?.json(&payload), 202)
            .await?
            .json::<serde_json::Value>()
            .await?;
        let change = format!("/api/changes/{}", support::string(&accepted, "change")?);
        eventually("preview rejection", || async {
            let state = context.api("GET", &change, None).await?;
            ensure(
                state["state"] == "Open"
                    && state["busy"] == false
                    && state["apps"][0]["result"]["Failed"]["error"].is_string(),
                "preview must fail before an operator decision",
            )
        })
        .await?;
        let pending = context.api("GET", &change, None).await?;
        ensure(
            pending["error"].is_null()
                && pending["apps"]
                    .as_array()
                    .is_some_and(|apps| apps.len() == 1)
                && pending["apps"][0]["name"] == name
                && pending["apps"][0]["check"] == true
                && pending["actions"] == json!(["Apply", "Close"]),
            "the target participates once and exposes only pre-production actions",
        )?;
        send(context.request("POST", &format!("{change}/complete"))?, 409).await?;
        ensure(
            context.api("GET", &path, None).await?["container_options"]
                == live["container_options"],
            "failed preview must preserve the live configuration",
        )?;
        send(
            context
                .request("PATCH", &path)?
                .json(&json!({"description":"occupied"})),
            409,
        )
        .await?;
        if decision == "close" {
            let downloaded = send(context.request("GET", &format!("{health_path}/file"))?, 200)
                .await?
                .text()
                .await?;
            ensure(downloaded == file, "Hurl download must preserve the file")?;
            let passed = context.api("GET", &health_path, None).await?;
            send(context.request("POST", &format!("{health_path}/run"))?, 202).await?;
            eventually("manual Hurl check during a failed release", || async {
                let health = context.api("GET", &health_path, None).await?;
                ensure(
                    health["status"].get("Passed").is_some()
                        && health["status"] != passed["status"],
                    "manual checks must use the live app while its preview remains failed",
                )
            })
            .await?;
            ensure(
                context.api("GET", &change, None).await? == pending,
                "manual XHealth success must not change the pending release",
            )?;
        }
        let endpoint = format!("{change}/{decision}");
        let (first, second) = tokio::try_join!(
            context.request("POST", &endpoint)?.send(),
            context.request("POST", &endpoint)?.send(),
        )?;
        let mut statuses = [first.status().as_u16(), second.status().as_u16()];
        statuses.sort_unstable();
        ensure(
            statuses == [200, 409],
            "simultaneous actions must start one task and reject the other",
        )?;
        eventually("change resolved", || async {
            let state = context.api("GET", &change, None).await?;
            ensure(
                state["state"]
                    == if decision == "apply" {
                        "Completed"
                    } else {
                        "Closed"
                    },
                "cleanup must finish before releasing the app",
            )
        })
        .await?;
        ensure(
            context.api("GET", &change, None).await?["actions"] == json!([]),
            "terminal changes must expose no actions",
        )?;
        ensure(
            context.api("GET", &change, None).await?["apps"] == pending["apps"],
            "closing or applying must retain the rehearsal results",
        )?;
        for action in ["apply", "complete", "close"] {
            send(context.request("POST", &format!("{change}/{action}"))?, 409).await?;
        }
        if decision == "apply" {
            ensure(
                context.api("GET", &path, None).await?["container_options"]["command"] == command,
                "direct apply must publish the requested configuration",
            )?;
        } else {
            ensure(
                context.api("GET", &path, None).await?["container_options"]
                    == live["container_options"],
                "closing must preserve the live configuration",
            )?;
        }
        if decision == "close" {
            send(
                context
                    .request("PATCH", &path)?
                    .json(&json!({"group":"missing-group"})),
                400,
            )
            .await?;
            ensure(
                context.api("GET", &change, None).await?["state"] == "Closed",
                "a rejected submission must preserve the latest record",
            )?;
            let next = context.api("PATCH", &path, Some(payload)).await?;
            ensure(
                next["change"] != accepted["change"],
                "a fresh submission must create a different change",
            )?;
            let next_path = format!("/api/changes/{}", support::string(&next, "change")?);
            eventually("new preview rejection", || async {
                let state = context.api("GET", &next_path, None).await?;
                ensure(
                    state["state"] == "Open"
                        && state["busy"] == false
                        && state["apps"][0]["result"]["Failed"]["error"].is_string(),
                    "new change must run its own checks",
                )
            })
            .await?;
            send(context.request("GET", &change)?, 404).await?;
            ensure(
                context
                    .docker(&[
                        "secret",
                        "ls",
                        "--filter",
                        &format!("label=dev.cdelivery.change.target={name}"),
                        "--format",
                        "{{.Name}}",
                    ])?
                    .lines()
                    .count()
                    == 1,
                "only the latest record may remain in storage",
            )?;
            context
                .api("POST", &format!("{next_path}/close"), None)
                .await?;
            eventually("new change closed", || async {
                ensure(
                    context.api("GET", &next_path, None).await?["state"] == "Closed",
                    "new resources must be cleaned up",
                )
            })
            .await?;
            let command = json!([
                "sh",
                "-c",
                "mkdir -p /www; printf healthy > /www/index.html; echo promoted; exec httpd -f -p 8080 -h /www"
            ]);
            let promoted = send(
                context.request("PATCH", &path)?.json(&json!({
                    "container_options":{"command":command}
                })),
                202,
            )
            .await?
            .json::<serde_json::Value>()
            .await?;
            let promoted_path = format!("/api/changes/{}", support::string(&promoted, "change")?);
            eventually("successful release after rejection", || async {
                let current = context.api("GET", &promoted_path, None).await?;
                ensure(
                    current["state"] == "Completed"
                        && current["apps"] == json!([{"name":name,"check":true,"result":"Passed"}]),
                    "a healthy replacement must complete its release check",
                )
            })
            .await?;
            ensure(
                context.api("GET", &path, None).await?["container_options"]["command"] == command,
                "successful checks must promote the requested configuration",
            )?;
        }
        context.api("DELETE", &path, None).await?;
        ensure(
            context
                .docker(&[
                    "secret",
                    "ls",
                    "--filter",
                    &format!("label=dev.cdelivery.change.target={name}"),
                    "--format",
                    "{{.Name}}",
                ])?
                .is_empty(),
            "deleting the target must remove its terminal change record",
        )?;
    }
    Ok(())
}

#[expect(clippy::too_many_lines)]
pub(crate) async fn direct_consumers(context: &Context) -> Result<()> {
    let source = support::name("source");
    let direct = support::name("direct");
    let indirect = support::name("indirect");
    let disabled = support::name("disabled");
    for (name, dependency, enabled) in [
        (&source, None, false),
        (&direct, Some(&source), true),
        (&indirect, Some(&direct), true),
        (&disabled, Some(&source), false),
    ] {
        let path = format!("/api/apps/{name}");
        context.app(name, "healthy").await?;
        if let Some(dependency) = dependency {
            let app = context.api("GET", &path, None).await?;
            context.api("PATCH", &path, Some(json!({"container_options":{
                "command":app["container_options"]["command"],
                "env":[{"name":"DEPENDENCY_HOST","value":{"dependency":{"App":dependency},"field":"host"}}]
            }}))).await?;
        }
        context
            .api(
                "PUT",
                &format!("{path}/xhealth"),
                Some(json!({
                    "method":{"Custom":{"command":["sh","-c","test \"$(wget -q -O - http://$DEPENDENCY_HOST:8080)\" = healthy"]}},
                    "interval":{"unit":"Hours","value":24},
                    "target":{"InternalPort":{"port":8080}},"release_check":enabled
                })),
            )
            .await?;
    }
    let source_path = format!("/api/apps/{source}");
    let live = context.api("GET", &source_path, None).await?;
    let accepted = send(
        context.request("PATCH", &source_path)?.json(&json!({
            "container_options":{"command":["sh","-c","mkdir -p /www; printf broken > /www/index.html; exec httpd -f -p 8080 -h /www"]}
        })),
        202,
    )
    .await?
    .json::<serde_json::Value>()
    .await?;
    let id = support::string(&accepted, "change")?;
    let change_path = format!("/api/changes/{id}");
    eventually("direct consumer failure", || async {
        let change = context.api("GET", &change_path, None).await?;
        ensure(
            change["state"] == "Open"
                && change["busy"] == false
                && change["apps"].as_array().is_some_and(|apps| {
                    apps.iter().any(|app| {
                        app["name"] == direct && app["result"]["Failed"]["error"].is_string()
                    })
                }),
            "direct consumer must complete its check",
        )
    })
    .await?;
    let change = context.api("GET", &change_path, None).await?;
    ensure(
        change["apps"].as_array().is_some_and(|apps| {
            apps.len() == 2
                && apps.contains(&json!({"name":source,"check":false,"result":null}))
                && apps.iter().any(|app| {
                    app["name"] == direct
                        && app["check"] == true
                        && app["result"]["Failed"]["error"].is_string()
                })
        }),
        "the fixed participants reserve the source and check only its enabled direct consumer",
    )?;
    ensure(
        change["stage"] == "Check",
        "consumer rejection must be a check failure",
    )?;
    let services = context.docker(&[
        "service",
        "ls",
        "--filter",
        &format!("label=dev.cdelivery.change_id={id}"),
        "--format",
        "{{.Name}}",
    ])?;
    let mut previews = 0;
    let mut checks = 0;
    for name in services.lines() {
        let service = context.inspect_service(name)?;
        let owner = &service[0]["Spec"]["Labels"]["dev.cdelivery.app_name"];
        if name.starts_with("cdelivery-preview-") {
            previews += 1;
            if owner == &direct {
                let env = service[0]["Spec"]["TaskTemplate"]["ContainerSpec"]["Env"]
                    .as_array()
                    .ok_or_else(|| {
                        crate::support::ErrorKind::Check("preview environment missing".to_owned())
                    })?;
                ensure(
                    env.iter().any(|value| {
                        value.as_str().is_some_and(|value| {
                            value.starts_with("DEPENDENCY_HOST=cdelivery-preview-")
                        })
                    }),
                    "direct consumer must use the target preview",
                )?;
            }
        } else if name.starts_with("cdelivery-check-") {
            checks += 1;
            ensure(
                owner == &direct,
                "the source and transitive consumers must not run checks",
            )?;
        }
    }
    ensure(
        previews == 2 && checks == 1,
        "the target preview exists even when its own check is disabled",
    )?;
    for name in [&source, &direct] {
        let path = format!("/api/apps/{name}");
        send(
            context
                .request("PATCH", &path)?
                .json(&json!({"description":"occupied"})),
            409,
        )
        .await?;
        send(context.request("DELETE", &path)?, 409).await?;
        send(context.request("POST", &format!("{path}/restart"))?, 409).await?;
        send(context.request("POST", &format!("{path}/rollback"))?, 409).await?;
        send(
            context
                .request("PATCH", &format!("{path}/xhealth"))?
                .json(&json!({"release_check":false})),
            409,
        )
        .await?;
        send(context.request("DELETE", &format!("{path}/xhealth"))?, 409).await?;
    }
    send(
        context
            .request("PATCH", &format!("/api/apps/{indirect}"))?
            .json(&json!({"description":"independent"})),
        200,
    )
    .await?;
    send(
        context
            .request("PATCH", &format!("/api/apps/{disabled}"))?
            .json(&json!({"description":"disabled"})),
        200,
    )
    .await?;
    send(
        context.request("POST", &format!("{change_path}/retry"))?,
        404,
    )
    .await?;
    context
        .api("POST", &format!("{change_path}/close"), None)
        .await?;
    eventually("all reservations released", || async {
        ensure(
            context.api("GET", &change_path, None).await?["state"] == "Closed",
            "manual cleanup must finish",
        )
    })
    .await?;
    ensure(
        context.api("GET", &source_path, None).await?["container_options"]
            == live["container_options"],
        "closing must preserve production",
    )?;
    let next = context.api("PATCH", &source_path, Some(json!({
        "container_options":{"command":["sh","-c","mkdir -p /www; printf healthy > /www/index.html; echo consumer-approved; exec httpd -f -p 8080 -h /www"]}
    }))).await?;
    let next_id = support::string(&next, "change")?;
    eventually("direct consumer approves the replacement", || async {
        let change = context
            .api("GET", &format!("/api/changes/{next_id}"), None)
            .await?;
        ensure(
            change["state"] == "Completed"
                && change["apps"].as_array().is_some_and(|apps| {
                    apps.len() == 2
                        && apps.contains(&json!({"name":source,"check":false,"result":null}))
                        && apps.contains(&json!({"name":direct,"check":true,"result":"Passed"}))
                }),
            "direct consumer must approve the new preview",
        )
    })
    .await?;
    ensure(
        context
            .docker(&[
                "service",
                "ls",
                "--filter",
                &format!("label=dev.cdelivery.change_id={next_id}"),
                "--format",
                "{{.Name}}",
            ])?
            .is_empty(),
        "completed release must leave no rehearsal services",
    )?;
    for name in [&indirect, &disabled, &direct, &source] {
        context
            .api("DELETE", &format!("/api/apps/{name}"), None)
            .await?;
    }
    Ok(())
}
