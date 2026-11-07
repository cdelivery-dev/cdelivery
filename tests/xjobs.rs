use jiff::{SignedDuration, Timestamp};
use serde_json::{Value, json};

use crate::support::{self, Context, Result, ensure, eventually, send};

#[expect(clippy::too_many_lines)]
pub(crate) async fn lifecycle(context: &Context) -> Result<()> {
    let name = support::name("xjobs");
    let path = format!("/api/apps/{name}");
    let job_path = format!("{path}/xjobs/interval");
    let service = format!("cdelivery-xjob-{name}_interval");
    context.app(&name, "healthy").await?;
    let spec = json!({"interval":{"unit":"Minutes","value":60},"timeout":{"unit":"Hours","value":1},"command":["sh","-c","sleep 3"]});
    for (field, value) in [
        ("interval", json!({"unit":"Minutes","value":0})),
        ("interval", json!({"unit":"Minutes","value":1441})),
        ("timeout", json!({"unit":"Hours","value":25})),
        ("timeout", json!({"unit":"Seconds","value":60})),
        ("timeout", json!({"unit":"Hours","value":u64::MAX})),
        ("timeout", Value::Null),
        ("schedule", json!({"Interval":{"minutes":60}})),
        ("command", json!(["echo", "bad\u{0}argument"])),
    ] {
        let mut invalid = spec.clone();
        invalid[field] = value;
        for method in ["PUT", "PATCH"] {
            send(context.request(method, &job_path)?.json(&invalid), 422).await?;
        }
    }
    send(
        context
            .request("PUT", &job_path)?
            .json(&json!({"interval":{"unit":"Minutes","value":1},"command":["true"]})),
        422,
    )
    .await?;
    let job = context.api("PUT", &job_path, Some(spec)).await?;
    let before = context.docker(&["service", "inspect", &service])?;
    let stored: Value = serde_json::from_str(&before)?;
    let created: Timestamp = serde_json::from_value(stored[0]["CreatedAt"].clone())?;
    let next: Timestamp = serde_json::from_value(job["next_run"].clone())?;
    ensure(
        next.duration_since(created) == SignedDuration::from_hours(1),
        "first run must wait from service creation",
    )?;
    for patch in [
        json!({}),
        json!({"interval":{"unit":"Hours","value":1},"timeout":{"unit":"Minutes","value":60}}),
    ] {
        context.api("PATCH", &job_path, Some(patch)).await?;
        ensure(
            before == context.docker(&["service", "inspect", &service])?,
            "equivalent patches must not change the service",
        )?;
    }
    for field in ["interval", "timeout", "command"] {
        send(
            context
                .request("PATCH", &job_path)?
                .json(&json!({field:null})),
            422,
        )
        .await?;
    }
    let mcp = crate::contracts::mcp(context, &context.credentials.api_key, "tools/call", json!({"name":"xjob_update","arguments":{"app":name,"job":"interval","changes":{"timeout":{"unit":"Seconds","value":1}}}})).await?;
    let problem: Value =
        serde_json::from_str(support::string(&mcp["result"]["content"][0], "text")?)?;
    ensure(
        mcp["result"]["isError"] == true && problem["status"] == 422,
        "MCP must reject unsupported duration units with the same validation status as REST",
    )?;
    let raw = stored[0]["Spec"]["Labels"]["dev.cdelivery.xjob_config"]
        .as_str()
        .ok_or_else(|| support::ErrorKind::Check("missing XJob config".to_owned()))?;
    let stored_spec: Value = serde_json::from_str(raw)?;
    ensure(
        stored_spec.get("epoch").is_none() && stored_spec.get("schedule").is_none(),
        "stored config must contain only interval, timeout and command",
    )?;
    for (field, value) in [
        ("interval", json!("PT0S")),
        ("command", json!(["echo", "bad\u{0}argument"])),
    ] {
        let mut invalid = stored_spec.clone();
        invalid[field] = value;
        context.docker(&[
            "service",
            "update",
            "--detach=true",
            "--label-add",
            &format!("dev.cdelivery.xjob_config={invalid}"),
            &service,
        ])?;
        send(context.request("GET", &job_path)?, 500).await?;
        ensure(
            context.api("GET", &format!("{path}/xjobs"), None).await?["jobs"] == json!([]),
            "invalid stored config must be excluded from runnable jobs",
        )?;
    }
    context.docker(&[
        "service",
        "update",
        "--detach=true",
        "--label-add",
        &format!("dev.cdelivery.xjob_config={raw}"),
        &service,
    ])?;
    context
        .api(
            "PATCH",
            &job_path,
            Some(json!({"interval":{"unit":"Minutes","value":1}})),
        )
        .await?;
    let finished = eventually("automatic first run", || async {
        let state = context.api("GET", &job_path, None).await?;
        ensure(
            state["status"].get("Passed").is_some(),
            &format!("job: {state}"),
        )?;
        Ok(state)
    })
    .await?;
    let ended: Timestamp =
        serde_json::from_value(finished["status"]["Passed"]["finished_at"].clone())?;
    let next: Timestamp = serde_json::from_value(finished["next_run"].clone())?;
    ensure(
        next.duration_since(ended) == SignedDuration::from_mins(1),
        "next run must wait from completion",
    )?;
    eventually("automatic second run", || async {
        let state = context.api("GET", &job_path, None).await?;
        ensure(
            state["last_run"] != finished["last_run"],
            "second run has not started",
        )?;
        let started: Timestamp = serde_json::from_value(state["last_run"].clone())?;
        ensure(
            started >= next,
            "automatic run started before the completion-based interval",
        )
    })
    .await?;
    context.api("DELETE", &job_path, None).await?;

    let job_path = format!("{path}/xjobs/timeout");
    let service = format!("cdelivery-xjob-{name}_timeout");
    context.api("PUT", &job_path, Some(json!({"interval":{"unit":"Hours","value":24},"timeout":{"unit":"Minutes","value":2},"command":["sleep","300"]}))).await?;
    let run_path = format!("{job_path}/run");
    let (a, b) = tokio::join!(
        context.request("POST", &run_path)?.send(),
        context.request("POST", &run_path)?.send()
    );
    let mut codes = [a?.status().as_u16(), b?.status().as_u16()];
    codes.sort_unstable();
    ensure(
        codes == [202, 409],
        "concurrent manual requests must start only one round",
    )?;
    let active = eventually("running task", || async {
        let state = context.api("GET", &job_path, None).await?;
        ensure(
            state["status"].get("Running").is_some(),
            "task has not started",
        )?;
        ensure(
            state["next_run"].is_null(),
            "active task must not report a next run",
        )?;
        Ok(state)
    })
    .await?;
    let started: Timestamp =
        serde_json::from_value(active["status"]["Running"]["started_at"].clone())?;
    send(context.request("POST", &run_path)?, 409).await?;
    context
        .api(
            "PATCH",
            &job_path,
            Some(json!({"timeout":{"unit":"Minutes","value":3}})),
        )
        .await?;
    context.docker(&["service", "update", "--detach=true", "--force", "cdelivery"])?;
    eventually("controller recovery and elapsed runtime", || async {
        let state = context.api("GET", &job_path, None).await?;
        ensure(
            state["status"]["Running"]["started_at"] == active["status"]["Running"]["started_at"],
            "controller restart must preserve the current round",
        )?;
        ensure(
            Timestamp::now().duration_since(started) >= SignedDuration::from_secs(65),
            "waiting for runtime before shortening timeout",
        )
    })
    .await?;
    let state = context
        .api(
            "PATCH",
            &job_path,
            Some(json!({"timeout":{"unit":"Minutes","value":1}})),
        )
        .await?;
    ensure(
        state["status"]["Running"]["started_at"] == active["status"]["Running"]["started_at"],
        "timeout update must not reset the start",
    )?;
    eventually("timeout stops current round", || async {
        ensure(
            context
                .docker(&[
                    "service",
                    "inspect",
                    "--format",
                    "{{.Spec.Mode.Replicated.Replicas}}",
                    &service,
                ])?
                .trim()
                == "0",
            "timeout must scale the runner to zero",
        )
    })
    .await?;
    eventually("timeout failure event", || async {
        let events = context
            .api("GET", &format!("/api/events?subject={name}/timeout"), None)
            .await?;
        ensure(
            events["records"].as_array().is_some_and(|records| {
                records.iter().any(|event| {
                    event["kind"] == "XJobFailed"
                        && event["detail"]
                            .as_str()
                            .is_some_and(|detail| detail.contains("requested stop"))
                })
            }),
            "timeout must be observable even if the task is reaped",
        )
    })
    .await?;
    eventually("task record removal", || async {
        ensure(
            context
                .docker(&["service", "ps", "--format", "{{.ID}}", &service])?
                .trim()
                .is_empty(),
            "stopped task records have not been reaped",
        )
    })
    .await?;
    let recovered = context
        .api(
            "PATCH",
            &job_path,
            Some(json!({"interval":{"unit":"Minutes","value":1},"command":["true"]})),
        )
        .await?;
    let stored: Value = serde_json::from_str(&context.docker(&["service", "inspect", &service])?)?;
    let updated: Timestamp = serde_json::from_value(stored[0]["UpdatedAt"].clone())?;
    let next: Timestamp = serde_json::from_value(recovered["next_run"].clone())?;
    ensure(
        recovered["status"] == "Settled"
            && next.duration_since(updated) == SignedDuration::from_mins(1),
        "missing task records must use the documented service-update approximation",
    )?;
    eventually("automatic recovery after timeout", || async {
        let state = context.api("GET", &job_path, None).await?;
        ensure(
            state["status"].get("Passed").is_some(),
            "recovered job has not completed",
        )?;
        let started: Timestamp = serde_json::from_value(state["last_run"].clone())?;
        ensure(
            started >= next,
            "recovery must respect the approximate next-run time",
        )
    })
    .await?;
    context.api("DELETE", &path, None).await?;
    Ok(())
}
