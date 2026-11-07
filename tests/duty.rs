mod fixture;

use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::support::{self, Context, Result, ensure, eventually, string};

async fn prepare(context: &Context, fixture: &fixture::Fixture) -> Result<()> {
    for path in ["/api/settings/agent", "/api/settings/channels/email"] {
        let settings = context.api("GET", path, None).await?;
        ensure(
            settings
                .as_object()
                .is_some_and(|settings| settings.values().all(Value::is_null)),
            "duty acceptance requires unconfigured model and email",
        )?;
    }
    let root = format!("/mnt/mac{}", context.root.canonicalize()?.display());
    let hosts = support::process::run(
        Command::new("orb").args([
            "-m",
            &context.environment.machines[0],
            "-u",
            "root",
            "getent",
            "ahostsv4",
            "host.orb.internal",
        ]),
        b"",
        Duration::from_secs(30),
        None,
    )?;
    let address = hosts
        .split_whitespace()
        .find_map(|value| value.parse::<std::net::Ipv4Addr>().ok())
        .ok_or_else(|| support::ErrorKind::Check("host.orb.internal did not resolve".into()))?;
    context.docker(&[
        "service",
        "update",
        "--detach=true",
        "--mount-add",
        &format!("type=bind,source={root}/duty-ca.pem,target=/duty-ca.pem,readonly"),
        "--env-add",
        "SSL_CERT_FILE=/duty-ca.pem",
        "--host-add",
        &format!("duty.cdelivery.test:{address}"),
        "cdelivery",
    ])?;
    eventually("controller restarted with fixture trust", || async {
        let service = context.inspect_service("cdelivery")?;
        ensure(
            service[0]["UpdateStatus"]["State"] == "completed",
            "controller update pending",
        )
    })
    .await?;
    eventually("controller with fixture trust", || async {
        context
            .api(
                "PATCH",
                "/api/settings/channels/email",
                Some(json!({
                    "address":"duty@cdelivery.test",
                    "smtp":{"host":"duty.cdelivery.test","port":fixture.smtp_port,"password":"fixture"}
                })),
            )
            .await
    })
    .await?;
    context
        .api(
            "PATCH",
            "/api/settings/agent",
            Some(json!({
                "provider":"OpenAICompatible","model":"duty-fixture","api_key":"fixture",
                "endpoint":format!("http://duty.cdelivery.test:{}/v1/", fixture.http_port),
                "role":"Operator","duty_enabled":false
            })),
        )
        .await?;
    Ok(())
}

#[expect(clippy::too_many_lines, clippy::print_stderr)]
pub(crate) async fn notifications(context: &Context, scenarios: &[&str]) -> Result<()> {
    let fixture = fixture::Fixture::start(context).await?;
    prepare(context, &fixture).await?;
    let reader_email = format!("{}@cdelivery.test", support::name("duty-reader"));
    let reader = context
        .api(
            "POST",
            "/api/users",
            Some(json!({
                "first_name":"Duty","last_name":"Reader","email":reader_email,
                "password_sha256":support::hash("Duty-fixture-password-123!"),"role":"Reader"
            })),
        )
        .await?;
    let reader_path = format!("/api/users/{}", string(&reader, "id")?);
    for &scenario in scenarios {
        if scenario == "demote" {
            context
                .api("PATCH", &reader_path, Some(json!({"role":"Operator"})))
                .await?;
        }
        fixture.state.lock().await.mode = scenario.to_owned();
        context
            .api(
                "PATCH",
                "/api/settings/agent",
                Some(json!({"duty_enabled": scenario != "disabled"})),
            )
            .await?;
        let name = support::name("duty");
        let calls = fixture.snapshot().await["calls"]
            .as_array()
            .map_or(0, Vec::len);
        if scenario == "runtime" {
            context.api("POST", "/api/apps", Some(json!({
                "name":name,"group":"tests","image":context.environment.images["fixture"].reference,
                "replicas":{"Fixed":1},"container_options":{"command":["sh","-c","exit 1"]}
            }))).await?;
        } else {
            context.app(&name, "duty fixture").await?;
            context.ready(&name, 1).await?;
            context
                .api(
                    "PUT",
                    &format!("/api/apps/{name}/xhealth"),
                    Some(json!({
                        "method":{"Custom":{"command":["sh","-c","exit 1"]}},
                        "interval":{"unit":"Hours","value":1},
                        "target":{"InternalPort":{"port":8080}},"guardian":false
                    })),
                )
                .await?;
        }
        if matches!(scenario, "cancel" | "receipt-cancel" | "demote") {
            eventually("analysis started before disable", || async {
                let current = fixture.snapshot().await;
                ensure(
                    current["calls"].as_array().is_some_and(|entries| {
                        entries.iter().skip(calls).any(|entry| {
                            entry["alert"] == true
                                && (scenario != "receipt-cancel" || entry["tool_result"] == true)
                        })
                    }),
                    "no alert analysis yet",
                )
            })
            .await?;
            if scenario == "demote" {
                context
                    .api("PATCH", &reader_path, Some(json!({"role":"Reader"})))
                    .await?;
            } else {
                context
                    .api(
                        "PATCH",
                        "/api/settings/agent",
                        Some(json!({"duty_enabled":false})),
                    )
                    .await?;
            }
        }
        let mail = eventually(&format!("{scenario} delivery"), || async {
            let current = fixture.snapshot().await;
            let mail: Vec<_> = current["attempts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|mail| {
                    mail["text"]
                        .as_str()
                        .is_some_and(|text| text.contains(&name))
                })
                .cloned()
                .collect();
            ensure(
                mail.iter()
                    .any(|mail| mail["to"] == json!([context.credentials.email]))
                    && mail.iter().any(|mail| mail["to"] == json!([reader_email])),
                "both subscribers must receive the alert",
            )?;
            Ok(mail)
        })
        .await?;
        for email in [&context.credentials.email, &reader_email] {
            let notices: Vec<_> = mail
                .iter()
                .filter(|mail| mail["to"] == json!([email]))
                .collect();
            ensure(notices.len() == 1, "exactly one variant per subscriber")?;
            let text = string(notices[0], "text")?;
            let ai = matches!(
                scenario,
                "ok" | "read" | "runtime" | "channel-error" | "demote"
            ) && email == &context.credentials.email;
            ensure(
                text.starts_with("AI duty result:") == ai,
                &format!("{scenario}: unexpected notification variant: {text}"),
            )?;
            if !ai {
                ensure(
                    text.contains("Observed:") && !text.contains("by system"),
                    "raw alert must include observation time without actor noise",
                )?;
            }
            ensure(
                text.contains("app_restart:")
                    == (matches!(scenario, "receipt" | "receipt-cancel")
                        && email == &context.credentials.email),
                "write receipt must survive failure and respect the duty role",
            )?;
        }
        if scenario == "timeout" {
            let current = fixture.snapshot().await;
            let emitted: jiff::Timestamp =
                serde_json::from_value(current["calls"][calls]["events"][0]["timestamp"].clone())?;
            let delivered: jiff::Timestamp = serde_json::from_value(mail[0]["time"].clone())?;
            ensure(
                delivered.duration_since(emitted).as_secs() >= 55,
                "analysis must wait for its total budget",
            )?;
        }
        if matches!(scenario, "tool-error" | "receipt") {
            let current = fixture.snapshot().await;
            let rounds = current["calls"]
                .as_array()
                .into_iter()
                .flatten()
                .skip(calls)
                .filter(|call| call["alert"] == true)
                .count();
            ensure(
                rounds == 1,
                "a failed tool must immediately end alert analysis",
            )?;
        }
        if matches!(
            scenario,
            "timeout" | "cancel" | "receipt-cancel" | "channel-error"
        ) {
            tokio::time::sleep(Duration::from_secs(15)).await;
            let current = fixture.snapshot().await;
            let count = current["attempts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|mail| {
                    mail["text"]
                        .as_str()
                        .is_some_and(|text| text.contains(&name))
                })
                .count();
            ensure(
                count == mail.len(),
                "late model results must not send a second notification",
            )?;
        }
        if scenario == "runtime" {
            context
                .api(
                    "PATCH",
                    &format!("/api/apps/{name}"),
                    Some(json!({
                        "container_options":{"command":["sh","-c","sleep 3600"]}
                    })),
                )
                .await?;
            context.ready(&name, 1).await?;
            eventually("one recovery notification per subscriber", || async {
                let current = fixture.snapshot().await;
                for email in [&context.credentials.email, &reader_email] {
                    let notices: Vec<_> = current["attempts"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|mail| {
                            mail["to"] == json!([email])
                                && mail["text"]
                                    .as_str()
                                    .is_some_and(|text| text.contains(&name))
                        })
                        .collect();
                    ensure(
                        notices.len() == 2,
                        "expected one failure and one recovery notice",
                    )?;
                    let recovery = string(notices[1], "text")?;
                    ensure(
                        recovery.contains("RuntimeRecovered")
                            || recovery.contains("Active tasks: 1. Running tasks: 1. Target: 1."),
                        "recovery notification must contain the observed recovery",
                    )?;
                }
                Ok(())
            })
            .await?;
            eventually("runtime lifecycle history", || async {
                let events = context
                    .api("GET", &format!("/api/events?subject={name}"), None)
                    .await?;
                let kinds: Vec<_> = events["records"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|event| event["kind"].as_str())
                    .collect();
                for kind in [
                    "AppCreated",
                    "AppUpdated",
                    "RuntimeIssue",
                    "RuntimeRecovered",
                ] {
                    ensure(
                        kinds.iter().filter(|&&value| value == kind).count() == 1,
                        &format!("history must retain exactly one {kind}"),
                    )?;
                }
                Ok(())
            })
            .await?;
        }
        context
            .api("DELETE", &format!("/api/apps/{name}"), None)
            .await?;
        eprintln!("duty scenario {scenario}: passed");
    }
    let result = fixture.snapshot().await;
    ensure(
        result["calls"].as_array().is_some_and(|calls| {
            calls.iter().all(|call| {
                call["events"].as_array().is_some_and(|events| {
                    events.iter().all(|event| {
                        matches!(
                            event["kind"].as_str(),
                            Some("XHealthFailed" | "RuntimeIssue" | "RuntimeRecovered")
                        )
                    })
                })
            })
        }),
        "ordinary events must not reach the model",
    )?;
    ensure(
        result["attempts"]
            .as_array()
            .into_iter()
            .flatten()
            .all(|mail| {
                mail["text"]
                    .as_str()
                    .is_some_and(|text| !text.trim().is_empty() && text.trim() != "NOOP")
            }),
        "notifications must not contain empty or NOOP replies",
    )?;
    std::fs::write(
        context.root.join("duty-results.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    context
        .api(
            "DELETE",
            &format!("/api/users/{}", string(&reader, "id")?),
            None,
        )
        .await?;
    fixture.stop().await
}

pub(crate) async fn patrol(context: &Context, mode: &str) -> Result<()> {
    let fixture = fixture::Fixture::start(context).await?;
    prepare(context, &fixture).await?;
    fixture.state.lock().await.mode = mode.to_owned();
    context
        .api(
            "PATCH",
            "/api/settings/agent",
            Some(json!({
                "duty_enabled":true,"patrol_interval":{"unit":"Minutes","value":15}
            })),
        )
        .await?;
    support::eventually_for("scheduled patrol", Duration::from_mins(16), || async {
        let current = fixture.snapshot().await;
        ensure(
            current["calls"].as_array().is_some_and(|calls| {
                calls.iter().any(|call| {
                    call["events"]
                        .as_array()
                        .is_some_and(|events| events.iter().any(|event| event["kind"] == "Patrol"))
                })
            }),
            "patrol has not reached the model",
        )
    })
    .await?;
    if mode == "patrol" {
        eventually("patrol notification", || async {
            let current = fixture.snapshot().await;
            ensure(
                current["mail"].as_array().is_some_and(|mail| {
                    mail.iter().any(|mail| mail["text"] == "AI patrol finding")
                }),
                "patrol notification not accepted",
            )
        })
        .await?;
    } else {
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    let result = fixture.snapshot().await;
    ensure(
        result["attempts"]
            .as_array()
            .is_some_and(|attempts| attempts.len() == usize::from(mode == "patrol")),
        "patrol must send one finding or remain silent for NOOP",
    )?;
    std::fs::write(
        context.root.join("duty-results.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    fixture.stop().await
}

pub(crate) async fn restore(context: &Context) -> Result<()> {
    context.api("DELETE", "/api/settings/agent", None).await?;
    context
        .api("DELETE", "/api/settings/channels/email", None)
        .await?;
    context.docker(&[
        "service",
        "update",
        "--detach=true",
        "--mount-rm",
        "/duty-ca.pem",
        "--env-rm",
        "SSL_CERT_FILE",
        "--host-rm",
        "duty.cdelivery.test",
        "cdelivery",
    ])?;
    eventually("controller restored", || async {
        let service = context.inspect_service("cdelivery")?;
        ensure(
            service[0]["UpdateStatus"]["State"] == "completed",
            "controller restoration pending",
        )?;
        context.api("GET", "/api/overview", None).await
    })
    .await?;
    Ok(())
}

pub(crate) async fn runtime(context: &Context) -> Result<()> {
    let name = support::name("runtime");
    context.app(&name, "runtime fixture").await?;
    context.ready(&name, 1).await?;
    context.docker(&[
        "service",
        "update",
        "--detach=true",
        "--update-failure-action",
        "pause",
        "--update-monitor",
        "5s",
        "--entrypoint",
        "sh -c 'exit 1'",
        &name,
    ])?;
    eventually("current runtime issue", || async {
        let state = context.api("GET", "/api/events/conditions", None).await?;
        ensure(
            state["conditions"].as_array().is_some_and(|entries| {
                entries.iter().any(|entry| {
                    entry["subject"] == name
                        && entry["kind"] == "RuntimeIssue"
                        && entry["detail"]
                            .as_str()
                            .is_some_and(|detail| detail.contains("Service update paused"))
                })
            }),
            "current update failure must raise RuntimeIssue",
        )
    })
    .await?;
    context.docker(&["service", "rollback", "--detach=true", &name])?;
    context.ready(&name, 1).await?;
    let recovered = Instant::now();
    eventually("runtime recovery despite stopped failed tasks", || async {
        let state = context.api("GET", "/api/events/conditions", None).await?;
        ensure(
            state["conditions"].as_array().is_some_and(|entries| {
                entries
                    .iter()
                    .all(|entry| entry["subject"] != name || entry["kind"] != "RuntimeIssue")
            }),
            "stopped failed tasks must not retain RuntimeIssue",
        )
    })
    .await?;
    ensure(
        recovered.elapsed() < Duration::from_secs(90),
        "recovery must not wait for the old ten-minute history window",
    )?;
    let tasks = context.docker(&[
        "service",
        "ps",
        "--no-trunc",
        "--format",
        "{{.CurrentState}}",
        &name,
    ])?;
    ensure(
        tasks.contains("Failed"),
        "recovery test requires retained failed task history",
    )?;
    let events = eventually("recovery event with current counts", || async {
        let events = context
            .api(
                "GET",
                &format!("/api/events?subject={name}&kind=RuntimeRecovered"),
                None,
            )
            .await?;
        ensure(
            events["records"].as_array().is_some_and(|records| {
                records.len() == 1
                    && records[0]["detail"] == "Active tasks: 1. Running tasks: 1. Target: 1."
            }),
            "recovery must include current counts",
        )?;
        Ok(events)
    })
    .await?;
    std::fs::write(
        context.root.join("runtime-recovery.json"),
        serde_json::to_vec_pretty(&events)?,
    )?;
    context
        .api("DELETE", &format!("/api/apps/{name}"), None)
        .await?;
    Ok(())
}
