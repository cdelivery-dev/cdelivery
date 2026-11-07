use std::collections::BTreeSet;
use std::path::Path;

use base64::Engine as _;
use serde_json::{Value, json};

use crate::environment::Image;
use crate::support::{self, Context, Result, ensure, eventually, send, string};

pub(crate) async fn restore(context: &Context) -> Result<()> {
    context.upgrade("restore").map(|_| ())
}

fn release_path(context: &Context) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../target/tests/current/releases/v1/{}/cdelivery-tests/latest.json",
        context.environment.registry
    ))
}

async fn publish(context: &Context, releases: &Value) -> Result<()> {
    std::fs::write(release_path(context), serde_json::to_vec_pretty(releases)?)?;
    restart(context).await
}

async fn restart(context: &Context) -> Result<()> {
    let before = context.docker(&[
        "service",
        "ps",
        "--filter",
        "desired-state=running",
        "--format",
        "{{.ID}}",
        "cdelivery",
    ])?;
    context.docker(&["service", "update", "--detach=true", "--force", "cdelivery"])?;
    eventually("release fixture loaded after restart", || async {
        let current = context.docker(&[
            "service",
            "ps",
            "--filter",
            "desired-state=running",
            "--format",
            "{{.ID}} {{.CurrentState}}",
            "cdelivery",
        ])?;
        ensure(
            !current.contains(before.trim()) && current.contains("Running"),
            "controller has not restarted",
        )?;
        context.api("GET", "/api/me", None).await?;
        Ok(())
    })
    .await
}

pub(crate) async fn running(
    context: &Context,
    service: &str,
    image: &Image,
) -> Result<BTreeSet<String>> {
    support::eventually_for(
        "running containers use the expected image digest",
        std::time::Duration::from_secs(360),
        || async {
            let expected = image
                .digest
                .rsplit_once('@')
                .ok_or_else(|| support::ErrorKind::Check("missing expected digest".into()))?
                .1;
            let inspected = context.inspect_service(service)?;
            ensure(
                inspected[0]["Spec"]["TaskTemplate"]["ContainerSpec"]["Image"]
                    .as_str()
                    .is_some_and(|current| {
                        current.split('@').next() == Some(image.reference.as_str())
                    }),
                "service target image mismatch",
            )?;
            if let Some(state) = inspected[0]["UpdateStatus"]["State"].as_str() {
                ensure(
                    matches!(state, "completed" | "rollback_completed"),
                    &format!("service update has not converged: {state}"),
                )?;
            }
            let mut containers = BTreeSet::new();
            for machine in &context.environment.machines {
                let ids = context.machine(
                    machine,
                    &[
                        "ps",
                        "--filter",
                        &format!("label=com.docker.swarm.service.name={service}"),
                        "--format",
                        "{{.ID}}",
                    ],
                )?;
                for id in ids.lines() {
                    let actual =
                        context.machine(machine, &["inspect", "--format", "{{.Image}}", id])?;
                    let digests: Vec<String> = serde_json::from_str(&context.machine(
                        machine,
                        &[
                            "image",
                            "inspect",
                            "--format",
                            "{{json .RepoDigests}}",
                            actual.trim(),
                        ],
                    )?)?;
                    ensure(
                        digests.iter().any(|digest| {
                            digest
                                .rsplit_once('@')
                                .is_some_and(|(_, digest)| digest == expected)
                        }),
                        &format!("unexpected running image on {machine}"),
                    )?;
                    containers.insert(format!("{machine}/{id}"));
                }
            }
            let desired = if matches!(service, "cdelivery" | "cdelivery-insight") {
                1
            } else {
                context.environment.machines.len()
            };
            ensure(
                containers.len() == desired,
                "not all target nodes are running the expected image",
            )?;
            Ok(containers)
        },
    )
    .await
}

async fn submit(
    context: &Context,
    method: &str,
    path: &str,
    field: &str,
    version: &str,
) -> Result<()> {
    let response = context
        .request(method, path)?
        .json(&json!({field:version}))
        .send()
        .await;
    match response {
        Ok(response) => ensure(
            response.status().is_success(),
            &format!("upgrade submission failed: {}", response.status()),
        ),
        Err(error)
            if path == "/api/instance"
                && (error.is_connect() || error.is_body() || error.is_request()) =>
        {
            std::fs::write(
                context.root.join("self-update-response.txt"),
                format!("{error:?}"),
            )?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

#[expect(clippy::too_many_lines)]
pub(crate) async fn component(initial: &Context, component: &str) -> Result<()> {
    let prepared = initial.upgrade("prepare")?;
    let context = &prepared;
    let (service, method, path, field) = match component {
        "cdelivery" => ("cdelivery", "PATCH", "/api/instance", "version"),
        "cdelivery-agent" => ("cdelivery-agent", "PUT", "/api/volume", "version"),
        "pingress" => ("cdelivery-pingress", "PATCH", "/api/pingress", "version"),
        "insight" => (
            "cdelivery-insight",
            "PATCH",
            "/api/insight",
            "server_version",
        ),
        "insight-agent" => (
            "cdelivery-insight-agent",
            "PATCH",
            "/api/insight",
            "agent_version",
        ),
        _ => return Err(support::ErrorKind::Check("unknown upgrade component".into()).into()),
    };
    let baseline = &context.environment.images[component];
    let target = &context.environment.upgrades[component];
    let configuration = context.api("GET", path, None).await?;
    ensure(
        baseline.digest != target.digest && baseline.version != target.version,
        "upgrade must use a distinct artifact and version",
    )?;
    let me = context.api("GET", "/api/me", None).await?;
    let session = context
        .login(&context.credentials.email, &context.credentials.password)
        .await?;
    let name = support::name("upgrade");
    let app_path = format!("/api/apps/{name}");
    let domain = format!("{name}.cdelivery.test");
    let certificate = rcgen::generate_simple_self_signed(vec![domain.clone()])?;
    let pem = certificate.cert.pem();
    let ca_path = context.root.join("upgrade-ca.pem");
    std::fs::write(&ca_path, &pem)?;
    let ca_path = format!("/mnt/mac{}", ca_path.canonicalize()?.display());
    let cert_path = format!("/api/pingress/certs/{domain}");
    context.api("PUT", &cert_path, Some(json!({"cert_chain_pem":pem,"private_key_pem":certificate.signing_key.serialize_pem()}))).await?;
    let manager = &context.environment.machines[0];
    let node = context.machine(manager, &["info", "--format", "{{.Swarm.NodeID}}"])?;
    let volume_path = format!("/api/nodes/{}/volumes", node.trim());
    context
        .api(
            "POST",
            &volume_path,
            Some(json!({"name":name,"description":"upgrade sentinel"})),
        )
        .await?;
    context.machine(
        manager,
        &[
            "run",
            "--rm",
            "--volume",
            &format!("{}{name}:/data", cdelivery_wire::VOLUME_PREFIX),
            &context.environment.images["fixture"].reference,
            "sh",
            "-c",
            &format!("printf {name} > /data/sentinel"),
        ],
    )?;
    let data_name = support::name("data");
    let data_path = format!("/api/apps/{data_name}");
    context.api("POST", "/api/apps", Some(json!({"name":data_name,"group":"tests","image":context.environment.images["fixture"].reference,"replicas":{"Fixed":1},"mounts":{"node_id":node.trim(),"mounts":[{"volume":name,"target":"/data"}]},"container_options":{"command":["sh","-c",format!("test \"$(cat /data/sentinel)\" = {name} && exec sleep 86400")]}}))).await?;
    context.ready(&data_name, 1).await?;
    context.api("POST", "/api/apps", Some(json!({"name":name,"group":"tests","image":context.environment.images["fixture"].reference,"replicas":{"Fixed":1},"xconfigs":[{"target":"/acceptance","data_base64":base64::engine::general_purpose::STANDARD.encode(&name)}],"domains":[{"domain":domain,"target":8080,"https":true}],"log_driver":{"driver":"Insight"},"container_options":{"command":["sh","-c",format!("echo '{name} persisted-before-upgrade'; mkdir -p /www; cp /acceptance /www/index.html; exec httpd -f -p 8080 -h /www")]}}))).await?;
    context.ready(&name, 1).await?;
    let app = context.api("GET", &app_path, None).await?;
    let route = async || {
        for machine in &context.environment.machines {
            eventually("HTTPS retains its route and certificate", || async {
                let (body, metadata) = context.curl(
                    machine,
                    &[
                        "--cacert",
                        &ca_path,
                        "--resolve",
                        &format!("{domain}:443:127.0.0.1"),
                        &format!("https://{domain}/"),
                    ],
                )?;
                ensure(
                    metadata["exitcode"] == 0
                        && metadata["http_code"] == 200
                        && body == name
                        && string(&metadata, "certs")?.contains(pem.trim()),
                    "upgrade route or certificate changed",
                )
            })
            .await?;
        }
        Ok::<(), support::Error>(())
    };
    route().await?;
    eventually("pre-upgrade log is queryable", || async {
        ensure(
            context
                .api("GET", &format!("{app_path}/logs?words={name}"), None)
                .await?["records"]
                .as_array()
                .is_some_and(|records| records.len() == 1),
            "waiting for pre-upgrade record",
        )
    })
    .await?;
    let service_id = context.docker(&["service", "inspect", "--format", "{{.ID}}", &name])?;
    eventually("pre-upgrade logs committed to Parquet", || async {
        let files = context.machine(
            manager,
            &[
                "run",
                "--rm",
                "--volume",
                "cdelivery-insight-data:/data:ro",
                &context.environment.images["fixture"].reference,
                "find",
                "/data",
                "-name",
                "*.parquet",
            ],
        )?;
        ensure(
            files.lines().any(|file| file.contains(service_id.trim())),
            "the app log has not reached durable storage",
        )?;
        std::fs::write(context.root.join("pre-upgrade-parquet.txt"), files)?;
        Ok(())
    })
    .await?;
    let job_path = format!("{app_path}/xjobs/upgrade-check");
    context.api("PUT", &job_path, Some(json!({"interval":{"unit":"Hours","value":24},"timeout":{"unit":"Minutes","value":1},"command":["true"]}))).await?;
    context
        .api("POST", &format!("{job_path}/run"), None)
        .await?;
    let job_finished = eventually("job finishes before upgrade", || async {
        let state = context.api("GET", &job_path, None).await?;
        ensure(
            state["status"].get("Passed").is_some(),
            "job has not passed",
        )?;
        Ok(state["status"]["Passed"]["finished_at"].clone())
    })
    .await?;
    let original: Value = serde_json::from_slice(&std::fs::read(release_path(context))?)?;
    let catalog_key = component.replace('-', "_");
    for (suffix, broken) in [("missing", false), ("broken", true)] {
        let failure = &context.environment.failures[component];
        let version = if broken {
            failure.version.clone()
        } else {
            format!("{}-missing", failure.version)
        };
        let image = if broken {
            failure.reference.clone()
        } else {
            format!("{}-missing", failure.reference)
        };
        let mut releases = original.clone();
        releases[&catalog_key]["image"] = json!(image);
        publish(context, &releases).await?;
        eventually("fault release available", || async {
            ensure(
                context.api("GET", "/api/versions", None).await?["components"][&catalog_key]["version"]
                    == version,
                "fault release not yet loaded",
            )
        })
        .await?;
        let previous: Value =
            serde_json::from_str(&context.docker(&["service", "inspect", service])?)?;
        submit(context, method, path, field, &version).await?;
        let automatic = matches!(component, "insight" | "insight-agent");
        let expected_state = if automatic {
            "rollback_completed"
        } else {
            "paused"
        };
        support::eventually_for(
            "failed rollout follows its recovery policy",
            std::time::Duration::from_secs(360),
            || async {
                let state = context.inspect_service(service)?;
                ensure(
                    state[0]["UpdateStatus"]["State"] == expected_state
                        && state[0]["UpdateStatus"]["StartedAt"]
                            != previous[0]["UpdateStatus"]["StartedAt"],
                    &format!(
                        "{suffix} update has not reached {expected_state}: {}",
                        state[0]["UpdateStatus"]
                    ),
                )
            },
        )
        .await?;
        let tasks = context.docker(&["service", "ps", "--no-trunc", service])?;
        std::fs::write(
            context.root.join(format!("{component}-{suffix}-tasks.txt")),
            &tasks,
        )?;
        ensure(
            tasks.contains(&image),
            "task history must show the requested failing image",
        )?;
        if !automatic {
            context.docker(&["service", "rollback", "--detach=true", service])?;
        }
        running(context, service, baseline).await?;
        eventually("management recovers after rollback", || async {
            context.api("GET", "/api/me", None).await
        })
        .await?;
    }
    publish(context, &original).await?;
    eventually("upgrade release restored", || async {
        ensure(
            context.api("GET", "/api/versions", None).await?["components"][&catalog_key]["version"]
                == target.version,
            "upgrade release not loaded",
        )
    })
    .await?;
    for version in [baseline.version.as_str(), "invalid"] {
        let problem: Value = send(
            context.request(method, path)?.json(&json!({field:version})),
            422,
        )
        .await?
        .json()
        .await?;
        ensure(
            problem["code"] == "VersionNotLatest",
            "only the catalog version may be selected",
        )?;
    }
    let before = running(context, service, baseline).await?;
    let agent_before = if component == "insight" {
        Some(
            running(
                context,
                "cdelivery-insight-agent",
                &context.environment.images["insight-agent"],
            )
            .await?,
        )
    } else {
        None
    };
    if component == "insight" {
        context.api("PATCH", "/api/insight", Some(json!({"server_version":target.version,"agent_version":context.environment.upgrades["insight-agent"].version}))).await?;
    } else if component == "pingress" {
        let result = crate::contracts::mcp(
            context,
            &context.credentials.api_key,
            "tools/call",
            json!({"name":"pingress_update","arguments":{"version":target.version}}),
        )
        .await?;
        ensure(
            result["result"]["isError"] != true,
            "MCP upgrade submission failed",
        )?;
    } else {
        submit(context, method, path, field, &target.version).await?;
    }
    let after = running(context, service, target).await?;
    ensure(
        before.is_disjoint(&after),
        "upgrade must replace actual containers",
    )?;
    if let Some(before) = agent_before {
        let after = running(
            context,
            "cdelivery-insight-agent",
            &context.environment.upgrades["insight-agent"],
        )
        .await?;
        ensure(
            before.is_disjoint(&after),
            "combined Insight upgrade must replace every agent",
        )?;
    }
    eventually("API restored after successful update", || async {
        context.api("GET", "/api/me", None).await
    })
    .await?;
    let saved: Value = serde_json::from_str(&context.docker(&["service", "inspect", service])?)?;
    submit(context, method, path, field, &target.version).await?;
    context.api(method, path, Some(json!({}))).await?;
    let unchanged: Value =
        serde_json::from_str(&context.docker(&["service", "inspect", service])?)?;
    ensure(
        saved[0]["Version"] == unchanged[0]["Version"] && saved[0]["Spec"] == unchanged[0]["Spec"],
        "same-version and empty updates must be no-ops",
    )?;
    ensure(
        context.api("GET", "/api/me", None).await? == me,
        "upgrade must preserve the current account and API key",
    )?;
    send(
        session.get(format!("{}/api/me", context.environment.base_url)),
        200,
    )
    .await?;
    ensure(
        context.api("GET", &app_path, None).await? == app,
        "platform upgrade must preserve application configuration",
    )?;
    let current = context.api("GET", path, None).await?;
    match component {
        "cdelivery" => ensure(
            current["domain"] == configuration["domain"],
            "controller upgrade must preserve its domain",
        )?,
        "pingress" => ensure(
            current["placement"] == configuration["placement"],
            "gateway upgrade must preserve placement",
        )?,
        "insight" | "insight-agent" => {
            for field in ["storage", "placement", "retention_days", "flush_interval"] {
                ensure(
                    current["server"][field] == configuration["server"][field],
                    &format!("Insight upgrade changed {field}"),
                )?;
            }
            ensure(
                current["agent"]["sample_interval_secs"]
                    == configuration["agent"]["sample_interval_secs"],
                "agent upgrade must preserve its sample interval",
            )?;
        }
        _ => {}
    }
    route().await?;
    let bytes = send(
        context.request(
            "GET",
            &format!(
                "{app_path}/xconfig/{}/file",
                string(&app["xconfigs"][0], "config_name")?
            ),
        )?,
        200,
    )
    .await?
    .bytes()
    .await?;
    ensure(
        bytes.as_ref() == name.as_bytes(),
        "upgrade must retain exact XConfig bytes",
    )?;
    let contents = context.machine(
        manager,
        &[
            "run",
            "--rm",
            "--volume",
            &format!("{}{name}:/data:ro", cdelivery_wire::VOLUME_PREFIX),
            &context.environment.images["fixture"].reference,
            "cat",
            "/data/sentinel",
        ],
    )?;
    ensure(
        contents == name,
        "upgrade must preserve exact mounted volume contents",
    )?;
    context
        .api("POST", &format!("{data_path}/restart"), None)
        .await?;
    context.ready(&data_name, 1).await?;
    context
        .api("POST", &format!("{job_path}/run"), None)
        .await?;
    eventually("job runs after upgrade", || async {
        let state = context.api("GET", &job_path, None).await?;
        ensure(
            state["status"].get("Passed").is_some()
                && state["status"]["Passed"]["finished_at"] != job_finished,
            "job scheduling must recover after upgrade",
        )
    })
    .await?;
    ensure(
        context
            .api("GET", &format!("{app_path}/logs?words={name}"), None)
            .await?["records"]
            .as_array()
            .is_some_and(|records| records.len() == 1),
        "upgrade must preserve historical logs without duplication",
    )?;
    context
        .api("POST", &format!("{app_path}/restart"), None)
        .await?;
    eventually("new logs after upgrade", || async {
        ensure(
            context
                .api("GET", &format!("{app_path}/logs?words={name}"), None)
                .await?["records"]
                .as_array()
                .is_some_and(|records| records.len() == 2),
            "collection must resume after upgrade",
        )
    })
    .await?;
    if component == "cdelivery-agent" {
        let nodes = context.api("GET", "/api/nodes", None).await?;
        for node in nodes["nodes"]
            .as_array()
            .ok_or_else(|| support::ErrorKind::Check("missing nodes".into()))?
        {
            context
                .api(
                    "GET",
                    &format!("/api/nodes/{}/volumes", string(node, "id")?),
                    None,
                )
                .await?;
        }
    }
    context.api("DELETE", &app_path, None).await?;
    context.api("DELETE", &data_path, None).await?;
    context.api("DELETE", &cert_path, None).await?;
    eventually("upgrade volume released", || async {
        let response = context
            .request("DELETE", &format!("{volume_path}/{name}"))?
            .send()
            .await?;
        ensure(
            response.status().is_success(),
            "app still holds the upgrade volume",
        )
    })
    .await?;
    Ok(())
}

#[expect(clippy::too_many_lines)]
pub(crate) async fn hurl(initial: &Context) -> Result<()> {
    let prepared = initial.upgrade("prepare")?;
    let context = &prepared;
    let name = support::name("hurl");
    let path = format!("/api/apps/{name}");
    context.app(&name, "healthy").await?;
    context.ready(&name, 1).await?;
    let releases: Value = serde_json::from_slice(&std::fs::read(release_path(context))?)?;
    let mut baseline = releases.clone();
    baseline["hurl"]["image"] = json!(context.environment.images["hurl"].reference);
    publish(context, &baseline).await?;
    eventually("baseline Hurl advertised", || async {
        ensure(
            context.api("GET", "/api/versions", None).await?["components"]["hurl"]["version"]
                == context.environment.images["hurl"].version,
            "baseline Hurl catalog not loaded",
        )
    })
    .await?;
    let file = "GET {{TARGET_URL}}\nHTTP 200\n[Asserts]\nbody == \"healthy\"\n";
    let health = format!("{path}/xhealth");
    context.api("PUT", &health, Some(json!({"method":{"Hurl":{"file":file,"version":context.environment.images["hurl"].version}},"interval":{"unit":"Hours","value":24},"target":{"InternalPort":{"port":8080}}}))).await?;
    eventually("baseline Hurl passes", || async {
        ensure(
            context.api("GET", &health, None).await?["status"]
                .get("Passed")
                .is_some(),
            "baseline check has not passed",
        )
    })
    .await?;
    let mut broken = releases.clone();
    broken["hurl"]["image"] = json!(context.environment.failures["hurl"].reference);
    publish(context, &broken).await?;
    eventually("broken Hurl advertised", || async {
        ensure(
            context.api("GET", "/api/versions", None).await?["components"]["hurl"]["version"]
                == context.environment.failures["hurl"].version,
            "broken Hurl catalog not loaded",
        )
    })
    .await?;
    context
        .api(
            "PATCH",
            &health,
            Some(
                json!({"method":{"Hurl":{"version":context.environment.failures["hurl"].version}}}),
            ),
        )
        .await?;
    eventually("broken Hurl reports failure", || async {
        ensure(
            context.api("GET", &health, None).await?["status"]
                .get("Failed")
                .is_some(),
            "broken runner must not report a passing check",
        )
    })
    .await?;
    publish(context, &releases).await?;
    eventually("Hurl upgrade advertised", || async {
        ensure(
            context.api("GET", "/api/versions", None).await?["components"]["hurl"]["version"]
                == context.environment.upgrades["hurl"].version,
            "new Hurl not loaded",
        )
    })
    .await?;
    context
        .api(
            "PATCH",
            &health,
            Some(
                json!({"method":{"Hurl":{"version":context.environment.upgrades["hurl"].version}}}),
            ),
        )
        .await?;
    ensure(
        send(context.request("GET", &format!("{health}/file"))?, 200)
            .await?
            .text()
            .await?
            == file,
        "Hurl upgrade must retain its file",
    )?;
    eventually("upgraded Hurl passes", || async {
        let state = context.api("GET", &health, None).await?;
        ensure(
            state["method"]["Hurl"]["image"] == context.environment.upgrades["hurl"].reference
                && state["status"].get("Passed").is_some(),
            "upgraded Hurl must run the saved file",
        )
    })
    .await?;
    let target = &context.environment.upgrades["hurl"];
    let expected = target
        .digest
        .rsplit_once('@')
        .ok_or_else(|| support::ErrorKind::Check("missing Hurl digest".into()))?
        .1;
    let mut found = false;
    for machine in &context.environment.machines {
        let ids = context.machine(
            machine,
            &[
                "ps",
                "--all",
                "--filter",
                &format!("label=com.docker.swarm.service.name=cdelivery-xhealth-{name}"),
                "--filter",
                &format!("ancestor={}", target.reference),
                "--format",
                "{{.ID}}",
            ],
        )?;
        for id in ids.lines() {
            let actual = context.machine(machine, &["inspect", "--format", "{{.Image}}", id])?;
            let digests: Vec<String> = serde_json::from_str(&context.machine(
                machine,
                &[
                    "image",
                    "inspect",
                    "--format",
                    "{{json .RepoDigests}}",
                    actual.trim(),
                ],
            )?)?;
            ensure(
                digests.iter().any(|digest| digest.ends_with(expected)),
                "completed Hurl runner used a different digest",
            )?;
            found = true;
        }
    }
    ensure(found, "no actual upgraded Hurl container found")?;
    context.api("DELETE", &path, None).await?;
    Ok(())
}

#[expect(clippy::too_many_lines)]
pub(crate) async fn catalog(initial: &Context) -> Result<()> {
    let prepared = initial.upgrade("prepare")?;
    let context = &prepared;
    let original: Value = serde_json::from_slice(&std::fs::read(release_path(context))?)?;
    let versions = context.api("GET", "/api/versions", None).await?;
    ensure(
        versions["templates"]
            == json!({
                "postgresql_18":"postgres:18",
                "mariadb_12":"mariadb:12.3",
            }),
        "template images must preserve the release fixture's spelling",
    )?;
    let session = context
        .login(&context.credentials.email, &context.credentials.password)
        .await?;
    for id in ["mysql", "postgresql", "mariadb", "postgresql_19"] {
        send(
            session.get(format!(
                "{}/groups/tests/apps/.template/{id}",
                context.environment.base_url,
            )),
            404,
        )
        .await?;
    }
    for (templates, expected, warning) in [
        (
            Some(json!({"postgresql_18":"postgres:18-alpine","postgresql_19":{"ignored":true}})),
            json!({"postgresql_18":"postgres:18-alpine"}),
            None,
        ),
        (
            Some(
                json!({"postgresql_18":"docker.io/library/postgres:18","mariadb_12":"registry.example.com/team/mariadb:12.3"}),
            ),
            json!({"postgresql_18":"docker.io/library/postgres:18","mariadb_12":"registry.example.com/team/mariadb:12.3"}),
            None,
        ),
        (Some(json!({})), json!({}), None),
        (None, json!({}), None),
        (
            Some(Value::Null),
            json!({}),
            Some("release templates entry invalid"),
        ),
        (
            Some(json!([])),
            json!({}),
            Some("release templates entry invalid"),
        ),
        (
            Some(json!("invalid")),
            json!({}),
            Some("release templates entry invalid"),
        ),
        (
            Some(json!({"postgresql_18":42,"mariadb_12":"mariadb:12.3"})),
            json!({"mariadb_12":"mariadb:12.3"}),
            Some("release template postgresql_18 entry invalid"),
        ),
        (
            Some(json!({"postgresql_18":"NOT/VALID:18","mariadb_12":"mariadb:12.3"})),
            json!({"mariadb_12":"mariadb:12.3"}),
            Some("release template postgresql_18 entry invalid"),
        ),
        (
            Some(
                json!({"postgresql_18":format!("postgres:18@sha256:{}", "a".repeat(64)),"mariadb_12":"mariadb:12.3"}),
            ),
            json!({"mariadb_12":"mariadb:12.3"}),
            Some("release template postgresql_18 entry invalid"),
        ),
        (
            Some(json!({"postgresql_18":"postgres:18","mariadb_12":false})),
            json!({"postgresql_18":"postgres:18"}),
            Some("release template mariadb_12 entry invalid"),
        ),
        (
            Some(original["templates"].clone()),
            original["templates"].clone(),
            None,
        ),
    ] {
        let mut catalog = original.clone();
        if let Some(templates) = templates {
            catalog["templates"] = templates;
        } else {
            catalog
                .as_object_mut()
                .ok_or_else(|| support::ErrorKind::Check("catalog is not an object".into()))?
                .remove("templates");
        }
        publish(context, &catalog).await?;
        eventually("catalog snapshot loaded", || async {
            ensure(
                context.api("GET", "/api/versions", None).await?
                    == json!({"components":versions["components"],"templates":expected}),
                "template metadata must not change component versions",
            )
        })
        .await?;
        if let Some(warning) = warning {
            let task = context.docker(&[
                "service",
                "ps",
                "--filter",
                "desired-state=running",
                "--format",
                "{{.ID}}",
                "cdelivery",
            ])?;
            eventually("template failure is observable", || async {
                let logs =
                    context.docker(&["service", "logs", "--raw", "--tail", "100", task.trim()])?;
                ensure(
                    logs.contains(warning),
                    "invalid template warning is missing",
                )?;
                std::fs::write(context.root.join("template-failure.log"), logs)?;
                Ok(())
            })
            .await?;
        }
        for id in ["postgresql_18", "mariadb_12"] {
            send(
                session.get(format!(
                    "{}/groups/tests/apps/.template/{id}",
                    context.environment.base_url,
                )),
                if expected.get(id).is_some() { 200 } else { 502 },
            )
            .await?;
        }
    }
    for invalid in [
        Some(json!({})),
        Some(Value::Null),
        Some(json!([])),
        Some(
            json!({"cdelivery":{"image":"invalid","changelog":""},"templates":original["templates"]}),
        ),
        Some(json!({"cdelivery":{"image":context.environment.upgrades["cdelivery"].reference}})),
        None,
    ] {
        if let Some(invalid) = &invalid {
            publish(context, invalid).await?;
        } else {
            std::fs::remove_file(release_path(context))?;
            restart(context).await?;
        }
        if invalid == Some(json!({})) {
            eventually("empty catalog is loaded", || async {
                let response = context.request("GET", "/api/versions")?.send().await?;
                ensure(response.status().is_success(), "catalog is still loading")?;
                ensure(
                    response.json::<Value>().await? == json!({"components":{},"templates":{}}),
                    "empty catalog not loaded",
                )
            })
            .await?;
        } else {
            let task = context.docker(&[
                "service",
                "ps",
                "--filter",
                "desired-state=running",
                "--format",
                "{{.ID}}",
                "cdelivery",
            ])?;
            eventually("catalog failure is observable", || async {
                let logs =
                    context.docker(&["service", "logs", "--raw", "--tail", "100", task.trim()])?;
                ensure(
                    logs.contains("release catalog refresh failed"),
                    "catalog fetch or decode has not reported its failure",
                )?;
                std::fs::write(context.root.join("catalog-failure.log"), logs)?;
                Ok(())
            })
            .await?;
            let problem: Value = send(context.request("GET", "/api/versions")?, 502)
                .await?
                .json()
                .await?;
            ensure(
                problem["code"] == "ReleaseUnavailable",
                "fetch failure, invalid catalog structure or a bad component must reject the entire catalog",
            )?;
        }
        send(
            session.get(format!(
                "{}/groups/tests/apps/.create",
                context.environment.base_url,
            )),
            200,
        )
        .await?;
        let setup = format!("{}/setup/platform", context.environment.base_url);
        let html = send(session.get(&setup), 200).await?.text().await?;
        ensure(
            html.contains("id=\"platform-setup-form\"")
                && html.contains(":disabled=\"submitting\""),
            "missing release metadata must not block the setup form",
        )?;
        send(
            session
                .post(&setup)
                .header("Origin", &context.environment.base_url)
                .form(&[("payload", r#"{"insight":{}}"#)]),
            303,
        )
        .await?;
        let response = send(
            context
                .request("PATCH", "/api/instance")?
                .json(&json!({"version":context.environment.upgrades["cdelivery"].version})),
            502,
        )
        .await?;
        let problem: Value = response.json().await?;
        ensure(
            problem["code"] == "ReleaseUnavailable",
            "invalid or missing release must not authorize an update",
        )?;
        ensure(
            context.api("GET", "/api/instance", None).await?["version"]
                == context.environment.images["cdelivery"].version,
            "bad catalog must preserve the current version",
        )?;
    }
    publish(context, &original).await?;
    Ok(())
}
