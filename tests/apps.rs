use base64::Engine as _;
use serde_json::{Value, json};

use crate::support::{self, Context, Result, ensure, eventually, send, string};

#[expect(clippy::too_many_lines)]
pub(crate) async fn lifecycle(context: &Context) -> Result<()> {
    let name = support::name("app");
    let path = format!("/api/apps/{name}");
    context.app(&name, "initial").await?;
    context.ready(&name, 1).await?;
    let initial = context.api("GET", &path, None).await?;
    context
        .api("PATCH", &path, Some(json!({"replicas":{"Fixed":2}})))
        .await?;
    context.ready(&name, 2).await?;
    context
        .api("POST", &format!("{path}/rollback"), None)
        .await?;
    context.ready(&name, 1).await?;
    ensure(
        context.api("GET", &path, None).await?["replicas"] == initial["replicas"],
        "rollback must restore replicas",
    )?;
    let tasks = context.docker(&[
        "service",
        "ps",
        "--filter",
        "desired-state=running",
        "--format",
        "{{.ID}}",
        &name,
    ])?;
    context
        .api("POST", &format!("{path}/restart"), None)
        .await?;
    eventually("replacement task", || async {
        ensure(
            tasks
                != context.docker(&[
                    "service",
                    "ps",
                    "--filter",
                    "desired-state=running",
                    "--format",
                    "{{.ID}}",
                    &name,
                ])?,
            "restart must replace the task",
        )
    })
    .await?;
    context.ready(&name, 1).await?;
    let bytes = [0, 1, 2, 128, 255, b'\n'];
    context.api("PATCH",&path,Some(json!({"xconfigs":[{"Upload":{"target":"/fixture.bin","data_base64":base64::engine::general_purpose::STANDARD.encode(bytes)}}]}))).await?;
    context.ready(&name, 1).await?;
    let app = context.api("GET", &path, None).await?;
    let config = string(&app["xconfigs"][0], "config_name")?;
    let artifact = send(
        context.request("GET", &format!("{path}/xconfig/{config}/file"))?,
        200,
    )
    .await?;
    ensure(
        artifact.headers().contains_key("content-disposition"),
        "artifact requires a download filename",
    )?;
    ensure(
        artifact
            .headers()
            .get("content-type")
            .is_some_and(|value| value == "application/octet-stream"),
        "binary artifact must use its declared MIME type",
    )?;
    ensure(
        artifact.bytes().await?.as_ref() == bytes,
        "binary artifact must preserve every byte",
    )?;
    let mcp = crate::contracts::mcp(
        context,
        &context.credentials.api_key,
        "tools/call",
        json!({"name":"app_download_xconfig","arguments":{"app":name,"config":config}}),
    )
    .await?;
    let resource = &mcp["result"]["content"][0]["resource"];
    ensure(
        resource["blob"] == base64::engine::general_purpose::STANDARD.encode(bytes)
            && resource["mimeType"] == "application/octet-stream",
        "MCP must preserve the binary artifact as a resource",
    )?;
    send(
        context
            .request("PATCH", &path)?
            .json(&json!({"xconfigs":[{"Upload":{"target":"/fixture.bin","data_base64":"!"}}]})),
        400,
    )
    .await?;
    ensure(
        context.api("GET", &path, None).await?["xconfigs"] == app["xconfigs"],
        "invalid upload must leave the configuration unchanged",
    )?;
    let text = "configuration: 配置\né\n";
    context.api("PATCH", &path, Some(json!({"xconfigs":[{"Upload":{"target":"/fixture.txt","data_base64":base64::engine::general_purpose::STANDARD.encode(text)}}]}))).await?;
    let app = context.api("GET", &path, None).await?;
    let config = string(&app["xconfigs"][0], "config_name")?;
    let mcp = crate::contracts::mcp(
        context,
        &context.credentials.api_key,
        "tools/call",
        json!({"name":"app_download_xconfig","arguments":{"app":name,"config":config}}),
    )
    .await?;
    ensure(
        mcp["result"]["content"][0]["resource"]["text"] == text,
        "MCP must preserve UTF-8 artifacts as text resources",
    )?;
    context.api("DELETE", &path, None).await?;
    eventually("app service removal", || async {
        let services = context.docker(&[
            "service",
            "ls",
            "--filter",
            &format!("label=dev.cdelivery.app_name={name}"),
            "--format",
            "{{.Name}}",
        ])?;
        ensure(
            services.trim().is_empty(),
            "app deletion must remove managed services",
        )
    })
    .await?;
    Ok(())
}

#[expect(clippy::too_many_lines)]
pub(crate) async fn jobs(context: &Context) -> Result<()> {
    let name = support::name("jobs");
    let path = format!("/api/apps/{name}");
    context.app(&name, "healthy").await?;
    context.ready(&name, 1).await?;
    for (job, command, outcome) in [("pass", "exit 0", "Passed"), ("fail", "exit 7", "Failed")] {
        let job_path = format!("{path}/xjobs/{job}");
        context
            .api(
                "PUT",
                &job_path,
                Some(
                    json!({"interval":{"unit":"Hours","value":24},"timeout":{"unit":"Minutes","value":1},"command":["sh","-c",command]}),
                ),
            )
            .await?;
        context
            .api("POST", &format!("{job_path}/run"), None)
            .await?;
        eventually("job terminal status", || async {
            let state = context.api("GET", &job_path, None).await?;
            ensure(
                state["status"].get(outcome).is_some(),
                &format!("job {job}: {state}"),
            )
        })
        .await?;
    }
    let file = "GET {{TARGET_URL}}\nHTTP 200\n[Asserts]\nbody == \"healthy\"\n";
    context.api("PUT",&format!("{path}/xhealth"),Some(json!({"method":{"Hurl":{"file":file,"version":context.environment.versions["hurl"]}},"interval":{"unit":"Minutes","value":1},"target":{"InternalPort":{"port":8080}},"guardian":true}))).await?;
    let downloaded = send(
        context.request("GET", &format!("{path}/xhealth/file"))?,
        200,
    )
    .await?
    .text()
    .await?;
    ensure(downloaded == file, "Hurl download must preserve the file")?;
    eventually("periodic health check", || async {
        let state = context.api("GET", &format!("{path}/xhealth"), None).await?;
        ensure(
            state["status"].get("Passed").is_some(),
            &format!("health check: {state}"),
        )
    })
    .await?;
    send(context.request("PATCH",&path)?.json(&json!({"container_options":{"command":["sh","-c","mkdir -p /www; echo broken > /www/index.html; exec httpd -f -p 8080 -h /www"]}})),202).await?;
    eventually("guarded update rejection", || async {
        let state = context
            .api("GET", &format!("{path}/guardian"), None)
            .await?;
        ensure(state["phase"] == "Failed", &format!("guardian: {state}"))
    })
    .await?;
    ensure(
        context.api("GET", &path, None).await?["container_options"]["command"][2]
            .as_str()
            .is_some_and(|command| command.contains("healthy")),
        "failed candidate must not replace the live app",
    )?;
    let health_path = format!("{path}/xhealth");
    let passed = eventually("live check ready for manual run", || async {
        let state = context.api("GET", &health_path, None).await?;
        ensure(
            state["status"].get("Passed").is_some(),
            "live check must pass",
        )?;
        Ok(state)
    })
    .await?;
    send(context.request("POST", &format!("{health_path}/run"))?, 202).await?;
    eventually("manual Hurl check with pending Guardian", || async {
        let state = context.api("GET", &health_path, None).await?;
        ensure(
            state["status"].get("Passed").is_some() && state["status"] != passed["status"],
            "manual check must use the live app while the candidate remains failed",
        )?;
        ensure(
            context
                .api("GET", &format!("{path}/guardian"), None)
                .await?["phase"]
                == "Failed",
            "manual XHealth success must not change the Guardian verdict",
        )
    })
    .await?;
    context
        .api("POST", &format!("{path}/guardian/retry"), None)
        .await?;
    eventually("Guardian check retry", || async {
        let state = context
            .api("GET", &format!("{path}/guardian"), None)
            .await?;
        let check: Value = serde_json::from_str(&context.docker(&[
            "service",
            "inspect",
            &format!("cdelivery-verify-{name}"),
        ])?)?;
        ensure(
            state["phase"] == "Failed" && check[0]["Spec"]["TaskTemplate"]["ForceUpdate"] == 1,
            "retry must advance the check round and retain the failed candidate",
        )
    })
    .await?;
    context
        .api("DELETE", &format!("{path}/guardian"), None)
        .await?;
    send(context.request("PATCH",&path)?.json(&json!({"container_options":{"command":["sh","-c","mkdir -p /www; printf healthy > /www/index.html; echo promoted; exec httpd -f -p 8080 -h /www"]}})),202).await?;
    eventually("guarded promotion", || async {
        let app = context.api("GET", &path, None).await?;
        ensure(
            app["container_options"]["command"][2]
                .as_str()
                .is_some_and(|command| command.contains("promoted")),
            "healthy candidate must be promoted",
        )
    })
    .await?;
    context.api("DELETE", &path, None).await?;
    Ok(())
}

#[expect(clippy::too_many_lines)]
pub(crate) async fn volumes(context: &Context) -> Result<()> {
    let nodes = context.api("GET", "/api/nodes", None).await?;
    let node = string(&nodes["nodes"][0], "id")?;
    let name = support::name("volume");
    let path = format!("/api/nodes/{node}/volumes");
    let node_data = context
        .api("GET", &format!("/api/nodes/{node}"), None)
        .await?;
    let machine = string(&node_data, "hostname")?;
    let sentinel = uuid::Uuid::new_v4().to_string();
    context
        .api(
            "POST",
            &path,
            Some(json!({"name":name,"description":"before"})),
        )
        .await?;
    let exists: Value = send(
        context.request("POST", &path)?.json(&json!({"name":name})),
        409,
    )
    .await?
    .json()
    .await?;
    ensure(
        exists["code"] == "VolumeExists",
        "duplicate creation must report VolumeExists",
    )?;
    context
        .api(
            "PATCH",
            &format!("{path}/{name}"),
            Some(json!({"description":null})),
        )
        .await?;
    context.machine(
        machine,
        &[
            "run",
            "--rm",
            "--volume",
            &format!("{}{name}:/data", cdelivery_wire::VOLUME_PREFIX),
            &context.environment.images["fixture"],
            "sh",
            "-c",
            "printf %s \"$1\" > /data/sentinel",
            "--",
            &sentinel,
        ],
    )?;
    let app = support::name("stateful");
    let app_path = format!("/api/apps/{app}");
    context.api("POST","/api/apps",Some(json!({"name":app,"group":"tests","image":context.environment.images["fixture"],"replicas":{"Fixed":1},"mounts":{"node_id":node,"mounts":[{"volume":name,"target":"/data"}]},"container_options":{"command":["sh","-c","test -f /data/sentinel && exec sleep 86400"]}}))).await?;
    context.ready(&app, 1).await?;
    send(
        context
            .request("PATCH", &app_path)?
            .json(&json!({"replicas":{"Fixed":2}})),
        422,
    )
    .await?;
    let in_use: Value = send(context.request("DELETE", &format!("{path}/{name}"))?, 409)
        .await?
        .json()
        .await?;
    ensure(
        in_use["code"] == "VolumeInUse",
        "a mounted volume must report VolumeInUse",
    )?;
    let mcp = crate::contracts::mcp(
        context,
        &context.credentials.api_key,
        "tools/call",
        json!({"name":"node_volume_delete","arguments":{"id":node,"name":name}}),
    )
    .await?;
    let problem: Value = serde_json::from_str(string(&mcp["result"]["content"][0], "text")?)?;
    ensure(
        mcp["result"]["isError"] == true
            && problem["status"] == 409
            && problem["code"] == "VolumeInUse",
        "MCP must report the same volume conflict as REST",
    )?;
    let tasks = context.docker(&[
        "service",
        "ps",
        "--filter",
        "desired-state=running",
        "--format",
        "{{.ID}}",
        &app,
    ])?;
    context
        .api("POST", &format!("{app_path}/restart"), None)
        .await?;
    eventually("stateful replacement task", || async {
        ensure(
            tasks
                != context.docker(&[
                    "service",
                    "ps",
                    "--filter",
                    "desired-state=running",
                    "--format",
                    "{{.ID}}",
                    &app,
                ])?,
            "restart must replace the stateful task",
        )
    })
    .await?;
    context.ready(&app, 1).await?;
    let data = context.machine(
        machine,
        &[
            "run",
            "--rm",
            "--volume",
            &format!("{}{name}:/data:ro", cdelivery_wire::VOLUME_PREFIX),
            &context.environment.images["fixture"],
            "cat",
            "/data/sentinel",
        ],
    )?;
    ensure(data == sentinel, "volume contents must survive app restart")?;
    context.api("DELETE", &app_path, None).await?;
    eventually("stateful containers removed", || async {
        ensure(
            context
                .machine(
                    machine,
                    &[
                        "ps",
                        "--all",
                        "--filter",
                        &format!("volume={}{name}", cdelivery_wire::VOLUME_PREFIX),
                        "--format",
                        "{{.ID}}",
                    ],
                )?
                .trim()
                .is_empty(),
            "deleted app containers still hold the volume",
        )
    })
    .await?;
    {
        let report = context.api("GET", &path, None).await?;
        ensure(
            report["volumes"]
                .as_array()
                .is_some_and(|volumes| volumes.iter().any(|volume| volume["name"] == name)),
            "volume must remain after deleting app",
        )?;
    }
    let stopped = format!("{name}-stopped");
    let data = context.machine(
        machine,
        &[
            "run",
            "--name",
            &stopped,
            "--volume",
            &format!("{}{name}:/data:ro", cdelivery_wire::VOLUME_PREFIX),
            &context.environment.images["fixture"],
            "cat",
            "/data/sentinel",
        ],
    )?;
    ensure(
        data == sentinel,
        "volume contents must survive app deletion",
    )?;
    let in_use: Value = send(context.request("DELETE", &format!("{path}/{name}"))?, 409)
        .await?
        .json()
        .await?;
    ensure(
        in_use["code"] == "VolumeInUse",
        "a stopped container must still prevent volume deletion",
    )?;
    context.machine(machine, &["rm", &stopped])?;
    for _ in 0..2 {
        let deleted: Value = send(context.request("DELETE", &format!("{path}/{name}"))?, 200)
            .await?
            .json()
            .await?;
        ensure(
            deleted == json!({"ok":true}),
            "volume deletion must succeed after references are removed and on repeat",
        )?;
    }
    Ok(())
}
