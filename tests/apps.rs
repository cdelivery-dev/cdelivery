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
    ensure(
        initial["log_driver"]["driver"] == "Insight",
        "omitted logging must default to Insight when collection is enabled",
    )?;
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
            &context.environment.images["fixture"].reference,
            "sh",
            "-c",
            "printf %s \"$1\" > /data/sentinel",
            "--",
            &sentinel,
        ],
    )?;
    let app = support::name("stateful");
    let app_path = format!("/api/apps/{app}");
    context.api("POST","/api/apps",Some(json!({"name":app,"group":"tests","image":context.environment.images["fixture"].reference,"replicas":{"Fixed":1},"mounts":{"node_id":node,"mounts":[{"volume":name,"target":"/data"}]},"container_options":{"command":["sh","-c","test -f /data/sentinel && exec sleep 86400"]}}))).await?;
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
            &context.environment.images["fixture"].reference,
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
            &context.environment.images["fixture"].reference,
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
