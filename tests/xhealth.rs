use jiff::{SignedDuration, Timestamp};
use serde_json::{Value, json};

use crate::support::{self, Context, Result, ensure, eventually, send, string};

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
        "name":name,"group":"tests","image":context.environment.images["fixture"],
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
        "guardian":false
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
