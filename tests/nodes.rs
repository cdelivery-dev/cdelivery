use serde_json::{Value, json};

use crate::support::{self, Context, Result, ensure, eventually, send, string};

pub(crate) async fn membership(context: &Context) -> Result<()> {
    let nodes = context.api("GET", "/api/nodes", None).await?;
    let nodes = nodes["nodes"]
        .as_array()
        .ok_or_else(|| support::ErrorKind::Check("missing node list".into()))?;
    ensure(
        nodes.len() == context.environment.machines.len()
            && nodes.iter().all(|node| node["state"] == "Ready"),
        "all environment nodes must be ready",
    )?;
    let invitation = context
        .api(
            "POST",
            "/api/nodes/join-invitations",
            Some(json!({"role":"Worker"})),
        )
        .await?;
    let body =
        json!({"advertise_addr":context.environment.addresses[&context.environment.machines[0]]});
    let token = string(&invitation, "invitation")?;
    let redeem = || {
        context
            .client
            .post(format!("{}/join/redeem", context.environment.base_url))
            .bearer_auth(token)
            .json(&body)
    };
    let first: Value = send(redeem(), 200).await?.json().await?;
    let second: Value = send(redeem(), 200).await?.json().await?;
    ensure(
        first["JoinToken"] == second["JoinToken"],
        "valid join invitations are reusable",
    )?;
    ensure(
        !nodes.iter().any(|node| node.to_string().contains("SWMTKN")),
        "management output must not expose raw join tokens",
    )?;
    context.docker(&["service", "update", "--detach=true", "--force", "cdelivery"])?;
    eventually("restart invalidates join invitation", || async {
        send(redeem(), 401).await.map(|_| ())
    })
    .await?;
    context.api("GET", "/api/me", None).await?;
    Ok(())
}

pub(crate) async fn recovery(context: &Context) -> Result<()> {
    ensure(
        context.environment.profile == "cluster",
        "node recovery requires a cluster environment",
    )?;
    let name = "node-recovery";
    let path = format!("/api/apps/{name}");
    context.api("DELETE", &path, None).await?;
    context.api("POST","/api/apps",Some(json!({"name":name,"group":"tests","image":context.environment.images["fixture"],"replicas":{"Fixed":3},"placement":{"label":"tests","max_replicas_per_node":2},"ports":[{"target":8080,"published":18080,"protocol":"Tcp"}],"container_options":{"command":["sh","-c","mkdir -p /www; echo cluster-ok > /www/index.html; exec httpd -f -p 8080 -h /www"]}}))).await?;
    context.ready(name, 3).await?;
    let tasks = context.docker(&[
        "service",
        "ps",
        "--filter",
        "desired-state=running",
        "--format",
        "{{.Node}}",
        name,
    ])?;
    let worker = context
        .environment
        .machines
        .iter()
        .skip(1)
        .find(|machine| tasks.lines().any(|node| node == machine.as_str()))
        .ok_or_else(|| support::ErrorKind::Check("no app task scheduled on a worker".into()))?;
    published(context, None).await?;
    context.control_machine("stop", worker)?;
    eventually("worker failure detection", || async {
        let nodes = context.api("GET", "/api/nodes", None).await?;
        ensure(
            nodes["nodes"].as_array().is_some_and(|nodes| {
                nodes
                    .iter()
                    .any(|node| node["hostname"] == *worker && node["state"] == "Down")
            }),
            "stopped worker must be reported Down",
        )
    })
    .await?;
    context.ready(name, 3).await?;
    let replacement = context.docker(&[
        "service",
        "ps",
        "--filter",
        "desired-state=running",
        "--format",
        "{{.Node}}",
        name,
    ])?;
    ensure(
        !replacement.lines().any(|node| node == worker),
        "replacement replicas must run on surviving nodes",
    )?;
    published(context, Some(worker)).await?;
    context.control_machine("start", worker)?;
    eventually("worker rejoin", || async {
        let nodes = context.api("GET", "/api/nodes", None).await?;
        ensure(
            nodes["nodes"].as_array().is_some_and(|nodes| {
                nodes
                    .iter()
                    .any(|node| node["hostname"] == *worker && node["state"] == "Ready")
            }),
            "worker must recover membership after restart",
        )
    })
    .await?;
    published(context, None).await?;
    context.api("DELETE", &path, None).await?;
    Ok(())
}

async fn published(context: &Context, stopped: Option<&str>) -> Result<()> {
    for (machine, address) in &context.environment.addresses {
        if Some(machine.as_str()) == stopped {
            continue;
        }
        eventually("cross-node published port", || async {
            let (body, metadata) = context.curl(
                &context.environment.machines[0],
                &[&format!("http://{address}:18080/")],
            )?;
            ensure(
                metadata["exitcode"] == 0
                    && metadata["http_code"] == 200
                    && body.trim() == "cluster-ok",
                &format!("published port must work on {machine}: {metadata}"),
            )
        })
        .await?;
    }
    Ok(())
}
