use serde_json::{Value, json};

use crate::support::{self, Context, Result, ensure, eventually, send, string};

pub(crate) async fn restore(context: &Context) -> Result<()> {
    let path = context.root.join("platform-baseline.json");
    if !path.exists() {
        return Ok(());
    }
    eventually("management available for component restoration", || async {
        context.api("GET", "/api/me", None).await
    })
    .await?;
    let saved: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let mut storage = saved["insight"]["server"]["storage"].clone();
    if let Some(config) = storage.get_mut("S3Compatible") {
        config["secret_access_key"] =
            json!("2222222222222222222222222222222222222222222222222222222222222222");
    }
    context
        .api(
            "PUT",
            "/api/volume",
            Some(json!({"version":context.environment.images["cdelivery-agent"].version})),
        )
        .await?;
    let response = context.request("GET", "/api/pingress")?.send().await?;
    ensure(
        matches!(response.status().as_u16(), 200 | 404),
        "cannot determine gateway state for restoration",
    )?;
    let method = if response.status().as_u16() == 404 {
        "PUT"
    } else {
        "PATCH"
    };
    let mut pingress = json!({"version":context.environment.images["pingress"].version,"placement":saved["pingress"]["placement"]});
    if method == "PUT" {
        pingress["acme"] = Value::Null;
    }
    context.api(method, "/api/pingress", Some(pingress)).await?;
    context.api("PATCH", "/api/insight", Some(json!({"server_version":context.environment.images["insight"].version,"agent_version":context.environment.images["insight-agent"].version,"storage":storage,"placement":saved["insight"]["server"]["placement"],"retention_days":saved["insight"]["server"]["retention_days"],"sample_interval_secs":saved["insight"]["agent"]["sample_interval_secs"],"flush_interval":saved["insight"]["server"]["flush_interval"]}))).await?;
    for (component, service) in [
        ("cdelivery", "cdelivery"),
        ("cdelivery-agent", "cdelivery-agent"),
        ("pingress", "cdelivery-pingress"),
        ("insight", "cdelivery-insight"),
        ("insight-agent", "cdelivery-insight-agent"),
    ] {
        crate::upgrade::running(context, service, &context.environment.images[component]).await?;
    }
    Ok(())
}

#[expect(clippy::too_many_lines)]
pub(crate) async fn lifecycle(context: &Context) -> Result<()> {
    let saved = json!({"pingress":context.api("GET", "/api/pingress", None).await?,"insight":context.api("GET", "/api/insight", None).await?,"volume":context.api("GET", "/api/volume", None).await?});
    std::fs::write(
        context.root.join("platform-baseline.json"),
        serde_json::to_vec_pretty(&saved)?,
    )?;
    let name = support::name("platform");
    let path = format!("/api/apps/{name}");
    let created = context.api("POST", "/api/apps", Some(json!({"name":name,"group":"tests","image":context.environment.images["fixture"].reference,"replicas":{"Fixed":1},"domains":[{"domain":format!("{name}.cdelivery.test"),"https":false,"target":8080}],"container_options":{"command":["sleep","86400"]}}))).await?;
    ensure(
        created["log_driver"]["driver"] == "Insight",
        "new apps must default to Insight when collection is enabled",
    )?;
    for (endpoint, code) in [("pingress", "PingressBusy"), ("insight", "InsightBusy")] {
        let problem: Value = send(context.request("DELETE", &format!("/api/{endpoint}"))?, 409)
            .await?
            .json()
            .await?;
        ensure(
            problem["code"] == code,
            "active consumers must prevent component disable",
        )?;
    }
    insight_dependencies(context, &name).await?;
    context.api("DELETE", &path, None).await?;
    for component in ["volume", "pingress", "insight"] {
        eventually("unused component disables", || async {
            let response = context
                .request("DELETE", &format!("/api/{component}"))?
                .send()
                .await?;
            ensure(
                response.status().is_success(),
                &format!("{component} still has active consumers"),
            )
        })
        .await?;
        eventually("repeated disable after controller replacement", || async {
            context
                .api("DELETE", &format!("/api/{component}"), None)
                .await
        })
        .await?;
    }
    eventually("management after component disable", || async {
        context.api("GET", "/api/me", None).await
    })
    .await?;
    ensure(
        context.api("GET", "/api/volume", None).await?["image"].is_null(),
        "volume agent must be disabled",
    )?;
    ensure(
        context.api("GET", "/api/insight", None).await?["status"] == "Disabled",
        "both Insight components must be disabled",
    )?;
    context.app(&name, "without-insight").await?;
    ensure(
        context.api("GET", &path, None).await?["log_driver"]["driver"] == "FollowHost",
        "new apps must follow the host when Insight is disabled",
    )?;
    restore(context).await?;
    context
        .api(
            "PATCH",
            &path,
            Some(json!({"description":"preserved logging"})),
        )
        .await?;
    ensure(
        context.api("GET", &path, None).await?["log_driver"]["driver"] == "FollowHost",
        "enabling Insight and unrelated updates must preserve existing logging",
    )?;
    context.api("DELETE", &path, None).await?;
    for driver in [
        json!({"driver":"FollowHost"}),
        json!({"driver":"JsonFile","max_size":{"unit":"MiB","value":10},"max_file":5}),
    ] {
        let created = context.api("POST", "/api/apps", Some(json!({"name":name,"group":"tests","image":context.environment.images["fixture"].reference,"replicas":{"Fixed":1},"log_driver":driver,"container_options":{"command":["sleep","86400"]}}))).await?;
        ensure(
            created["log_driver"] == driver,
            "explicit logging must override the Insight default",
        )?;
        context.api("DELETE", &path, None).await?;
    }
    eventually("platform components restored", || async {
        ensure(
            context.api("GET", "/api/insight", None).await? == saved["insight"],
            "Insight configuration must round trip disable and enable",
        )?;
        let nodes = context.api("GET", "/api/nodes", None).await?;
        for node in nodes["nodes"]
            .as_array()
            .ok_or_else(|| support::ErrorKind::Check("missing nodes".into()))?
        {
            let response = context
                .request(
                    "GET",
                    &format!("/api/nodes/{}/volumes", string(node, "id")?),
                )?
                .send()
                .await?;
            ensure(
                response.status().is_success(),
                "volume agent has not recovered",
            )?;
        }
        Ok(())
    })
    .await?;
    crate::telemetry::telemetry(context).await?;
    crate::telemetry::access(context).await?;
    Ok(())
}

#[expect(clippy::too_many_lines)]
async fn insight_dependencies(context: &Context, name: &str) -> Result<()> {
    let path = format!("/api/apps/{name}");
    let health = format!("{path}/xhealth");
    context.api("PUT", &health, Some(json!({"method":{"Custom":{"command":["true"]}},"interval":{"unit":"Hours","value":24},"target":{"InternalPort":{"port":8080}}}))).await?;
    for (job, command) in [
        ("completed", json!(["true"])),
        ("running", json!(["sleep", "86400"])),
    ] {
        let job_path = format!("{path}/xjobs/{job}");
        context.api("PUT", &job_path, Some(json!({"interval":{"unit":"Hours","value":24},"timeout":{"unit":"Hours","value":24},"command":command}))).await?;
        context
            .api("POST", &format!("{job_path}/run"), None)
            .await?;
    }
    for endpoint in [&health, &format!("{path}/xjobs/completed")] {
        eventually("completed Insight runner", || async {
            ensure(
                context.api("GET", endpoint, None).await?["status"]
                    .get("Passed")
                    .is_some(),
                "runner must complete before disabling Insight",
            )
        })
        .await?;
    }
    let running = format!("cdelivery-xjob-{name}_running");
    let task = eventually("running Insight job", || async {
        ensure(
            context
                .docker(&[
                    "service",
                    "ps",
                    "--filter",
                    "desired-state=running",
                    "--format",
                    "{{.CurrentState}}",
                    &running,
                ])?
                .trim()
                .starts_with("Running "),
            "job container must be running before disabling Insight",
        )?;
        context.docker(&[
            "service",
            "ps",
            "--filter",
            "desired-state=running",
            "--quiet",
            &running,
        ])
    })
    .await?;
    let mut runners = Vec::new();
    for service in [
        format!("cdelivery-xhealth-{name}"),
        format!("cdelivery-xjob-{name}_completed"),
        running.clone(),
    ] {
        let snapshot = context.inspect_service(&service)?[0].clone();
        ensure(
            snapshot["Spec"]["TaskTemplate"]["LogDriver"]["Name"] == "fluentd",
            "runner must retain its original Insight log driver",
        )?;
        runners.push((service, snapshot));
    }
    context
        .api(
            "PATCH",
            &path,
            Some(json!({"log_driver":{"driver":"FollowHost"}})),
        )
        .await?;
    eventually("disable Insight with auxiliary runners", || async {
        context.api("DELETE", "/api/insight", None).await
    })
    .await?;
    eventually("Insight disabled with auxiliary runners", || async {
        ensure(
            context.api("GET", "/api/insight", None).await?["status"] == "Disabled",
            "auxiliary runners must not prevent disabling Insight",
        )
    })
    .await?;
    context.api("DELETE", "/api/insight", None).await?;
    for (service, snapshot) in &runners {
        let current = context.inspect_service(service)?;
        ensure(
            current[0]["ID"] == snapshot["ID"] && current[0]["Spec"] == snapshot["Spec"],
            "disabling Insight must not rebuild or rerun auxiliary services",
        )?;
    }
    ensure(
        context.docker(&[
            "service",
            "ps",
            "--filter",
            "desired-state=running",
            "--quiet",
            &running,
        ])? == task,
        "disabling Insight must preserve the running job task",
    )?;
    for (endpoint, service) in [
        (health.clone(), format!("cdelivery-xhealth-{name}")),
        (
            format!("{path}/xjobs/completed"),
            format!("cdelivery-xjob-{name}_completed"),
        ),
    ] {
        context
            .api("POST", &format!("{endpoint}/run"), None)
            .await?;
        ensure(
            context.inspect_service(&service)?[0]["Spec"]["TaskTemplate"]["LogDriver"].is_null(),
            "the next round must inherit FollowHost without Insight",
        )?;
        eventually("runner completes without Insight", || async {
            ensure(
                context.api("GET", &endpoint, None).await?["status"]
                    .get("Passed")
                    .is_some(),
                "the next round must complete without Insight",
            )
        })
        .await?;
    }
    restore(context).await?;
    context
        .api(
            "PATCH",
            &health,
            Some(json!({"method":{"Custom":{"command":["false"]}},"guardian":true})),
        )
        .await?;
    for dependency in [
        json!({"log_driver":{"driver":"Insight"}}),
        json!({"replicas":{"Dynamic":{"min":1,"max":2}},"resources":{"limits":{"cpus":0.1}}}),
    ] {
        let mut changes = dependency.clone();
        changes["container_options"] = json!({"command":["sleep","86399"]});
        send(context.request("PATCH", &path)?.json(&changes), 202).await?;
        let live = context.api("GET", &path, None).await?;
        ensure(
            live["log_driver"]["driver"] == "FollowHost" && live["replicas"] == json!({"Fixed":1}),
            "Insight dependency must exist only in the Guardian candidate",
        )?;
        let problem: Value = send(context.request("DELETE", "/api/insight")?, 409)
            .await?
            .json()
            .await?;
        ensure(
            problem["code"] == "InsightBusy"
                && problem["detail"]
                    .as_str()
                    .is_some_and(|message| message.contains(&format!("cdelivery-shadow-{name}"))),
            "Guardian candidate logging and scaling must prevent disabling Insight",
        )?;
        context
            .api("DELETE", &format!("{path}/guardian"), None)
            .await?;
        context.api("PATCH", &path, Some(dependency)).await?;
        let problem: Value = send(context.request("DELETE", "/api/insight")?, 409)
            .await?
            .json()
            .await?;
        ensure(
            problem["code"] == "InsightBusy"
                && problem["detail"]
                    .as_str()
                    .is_some_and(|message| message.contains(name)),
            "live app logging and scaling must prevent disabling Insight",
        )?;
        context
            .api(
                "PATCH",
                &path,
                Some(json!({"log_driver":{"driver":"FollowHost"},"replicas":{"Fixed":1}})),
            )
            .await?;
    }
    Ok(())
}

pub(crate) async fn scaling(context: &Context) -> Result<()> {
    let name = support::name("scaling");
    let path = format!("/api/apps/{name}");
    context.api("POST", "/api/apps", Some(json!({"name":name,"group":"tests","image":context.environment.images["fixture"].reference,"replicas":{"Dynamic":{"min":1,"max":2}},"resources":{"limits":{"cpus":0.1}},"container_options":{"command":["sh","-c","while :; do :; done"]}}))).await?;
    context.ready(&name, 1).await?;
    support::eventually_for(
        "CPU pressure scales within the configured bound",
        std::time::Duration::from_secs(300),
        || async {
            let state: Value =
                serde_json::from_str(&context.docker(&["service", "inspect", &name])?)?;
            ensure(
                state[0]["Spec"]["Mode"]["Replicated"]["Replicas"] == 2,
                "CPU pressure has not produced two replicas",
            )
        },
    )
    .await?;
    context.ready(&name, 2).await?;
    ensure(
        context.api("GET", &path, None).await?["replicas"] == json!({"Dynamic":{"min":1,"max":2}}),
        "scaling must preserve the declared bounds",
    )?;
    context
        .api(
            "PATCH",
            &path,
            Some(json!({"replicas":{"Fixed":1},"container_options":{"command":["sleep","86400"]}})),
        )
        .await?;
    context.ready(&name, 1).await?;
    ensure(
        context.api("GET", &path, None).await?["replicas"] == json!({"Fixed":1}),
        "switching to a fixed count must clear automatic scaling",
    )?;
    context.api("DELETE", &path, None).await?;
    Ok(())
}
