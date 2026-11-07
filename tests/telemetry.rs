use serde_json::json;

use crate::support::{self, Context, Result, ensure, eventually, string};

#[expect(clippy::too_many_lines)]
pub(crate) async fn idle_connection(context: &Context) -> Result<()> {
    ensure(
        context.environment.profile == "cluster" && context.environment.machines.len() == 3,
        "idle connection recovery requires two workers",
    )?;
    let actors = [
        (&context.environment.machines[1], support::name("idle")),
        (&context.environment.machines[2], support::name("busy")),
    ];
    let tasks = || {
        context.docker(&[
            "service",
            "ps",
            "--filter",
            "desired-state=running",
            "--format",
            "{{.ID}}",
            "cdelivery",
            "cdelivery-insight",
            "cdelivery-insight-agent",
        ])
    };
    let before = tasks()?;
    std::fs::write(context.root.join("tasks-before.txt"), &before)?;
    let started = jiff::Timestamp::now().to_string();
    let mut labels = Vec::new();
    let mut apps = Vec::new();
    let mut collectors = Vec::new();
    let mut result: Result<()> = async {
        for (index, (machine, name)) in actors.iter().enumerate() {
            let node = context.machine(machine, &["info", "--format", "{{.Swarm.NodeID}}"])?;
            labels.push((node.trim().to_owned(), name));
            context.docker(&["node", "update", "--label-add", &format!("dev.cdelivery.labels.{name}=true"), node.trim()])?;
            let collector = context.machine(machine, &["ps", "--filter", "label=com.docker.swarm.service.name=cdelivery-insight-agent", "--format", "{{.ID}}"])?;
            ensure(collector.lines().count() == 1, "expected one collector per worker")?;
            collectors.push((*machine, collector.trim().to_owned()));
            let tail = if index == 0 { "exec sleep 86400" } else { "while true; do echo keepalive; sleep 10; done" };
            apps.push(name.clone());
            context.api("POST", "/api/apps", Some(json!({
                "name": name, "group": "tests", "image": context.environment.images["fixture"].reference,
                "replicas": {"Fixed": 1}, "placement": {"label": name},
                "log_driver": {"driver": "Insight"},
                "container_options": {"command": ["sh", "-c", format!("echo {name}-initial; {tail}")]}
            }))).await?;
            context.ready(name, 1).await?;
            collected(context, name, &["initial"]).await?;
        }
        let original = context.machine(actors[0].0, &["ps", "--filter", &format!("label=com.docker.swarm.service.name={}", actors[0].1), "--format", "{{.ID}}"])?;
        ensure(original.lines().count() == 1, "expected one original application container")?;
        let since = jiff::Timestamp::now().to_string();
        std::fs::write(context.root.join("idle-started.txt"), &since)?;
        tokio::time::sleep(std::time::Duration::from_secs(315)).await;
        for (index, (machine, collector)) in collectors.iter().enumerate() {
            let logs = context.machine(machine, &["logs", "--timestamps", "--since", &since, collector])?;
            std::fs::write(context.root.join(format!("after-idle-{index}-collector.log")), &logs)?;
            ensure(
                logs.contains("closed while waiting for data") == (index == 0),
                "only the idle collector must observe the remote connection closure",
            )?;
        }
        for (machine, name) in &actors {
            let probe = format!("{name}-new");
            apps.push(probe.clone());
            context.api("POST", "/api/apps", Some(json!({
                "name": probe, "group": "tests", "image": context.environment.images["fixture"].reference,
                "replicas": {"Fixed": 1}, "placement": {"label": name},
                "log_driver": {"driver": "Insight"},
                "container_options": {"command": ["sh", "-c", format!("echo {probe}-first; sleep 20; echo {probe}-second; sleep 20; echo {probe}-third; exec sleep 86400")]}
            }))).await?;
            context.ready(&probe, 1).await?;
            let placement = context.machine(machine, &["ps", "--filter", &format!("label=com.docker.swarm.service.name={probe}"), "--format", "{{.ID}}"])?;
            ensure(placement.lines().count() == 1, "probe must run on its assigned worker")?;
        }
        for (_, name) in &actors {
            collected(context, &format!("{name}-new"), &["first", "second", "third"]).await?;
        }
        let name = &actors[0].1;
        context.machine(actors[0].0, &["exec", original.trim(), "sh", "-c", &format!(
            "echo {name}-first > /proc/1/fd/1; sleep 20; echo {name}-second > /proc/1/fd/1; sleep 20; echo {name}-third > /proc/1/fd/1"
        )])?;
        collected(context, name, &["initial", "first", "second", "third"]).await?;
        let current = context.machine(actors[0].0, &["ps", "--filter", &format!("label=com.docker.swarm.service.name={name}"), "--format", "{{.ID}}"])?;
        ensure(current == original, "original application must survive the idle period")?;
        Ok(())
    }.await;
    let mut finish = |name: &str, outcome: Result<()>| context.finish(name, outcome, &mut result);
    finish(
        "tasks",
        (|| {
            let after = tasks()?;
            std::fs::write(context.root.join("tasks-after.txt"), &after)?;
            ensure(
                before == after,
                "components must not restart during idle recovery",
            )
        })(),
    );
    for (machine, collector) in &collectors {
        let diagnostic: Result<()> = (|| {
            std::fs::write(
                context.root.join(format!("{machine}-collector.log")),
                context.machine(
                    machine,
                    &["logs", "--timestamps", "--since", &started, collector],
                )?,
            )?;
            Ok(())
        })();
        finish(&format!("{machine}-diagnostic"), diagnostic);
    }
    for name in apps.iter().rev() {
        let diagnostic: Result<()> = (|| {
            std::fs::write(
                context.root.join(format!("{name}-application.log")),
                context.docker(&["service", "logs", "--timestamps", name])?,
            )?;
            std::fs::write(
                context.root.join(format!("{name}-tasks.json")),
                context.docker(&[
                    "service",
                    "ps",
                    "--no-trunc",
                    "--format",
                    "{{json .}}",
                    name,
                ])?,
            )?;
            Ok(())
        })();
        finish(&format!("{name}-diagnostic"), diagnostic);
        finish(
            &format!("{name}-cleanup"),
            context
                .api("DELETE", &format!("/api/apps/{name}"), None)
                .await
                .map(|_| ()),
        );
    }
    for (node, name) in labels {
        finish(
            &format!("{name}-label-cleanup"),
            context
                .docker(&[
                    "node",
                    "update",
                    "--label-rm",
                    &format!("dev.cdelivery.labels.{name}"),
                    &node,
                ])
                .map(|_| ()),
        );
    }
    result
}

async fn collected(context: &Context, name: &str, suffixes: &[&str]) -> Result<()> {
    let mut expected: Vec<_> = suffixes
        .iter()
        .map(|suffix| format!("{name}-{suffix}"))
        .collect();
    expected.sort_unstable();
    eventually("exact application log records", || async {
        let response = context
            .api("GET", &format!("/api/apps/{name}/logs?words={name}"), None)
            .await?;
        std::fs::write(
            context.root.join(format!("{name}-query.json")),
            response.to_string(),
        )?;
        let records = response["records"]
            .as_array()
            .ok_or_else(|| support::ErrorKind::Check("missing log records".into()))?;
        let mut actual: Vec<_> = records
            .iter()
            .map(|record| string(record, "log"))
            .collect::<Result<_>>()?;
        actual.sort_unstable();
        ensure(
            actual == expected,
            &format!("{name}: expected {expected:?}, got {actual:?}"),
        )
    })
    .await
}

#[expect(clippy::too_many_lines)]
pub(crate) async fn queries(context: &Context) -> Result<()> {
    let name = support::name("query");
    let path = format!("/api/apps/{name}");
    context.api("POST", "/api/apps", Some(json!({"name":name,"group":"tests","image":context.environment.images["fixture"].reference,"replicas":{"Fixed":1},"log_driver":{"driver":"Insight"},"container_options":{"command":["sh","-c",format!("echo '{name} Alpha first'; sleep 1; echo '{name} alpha second'; sleep 1; echo '{name} Beta error' >&2; exec sleep 86400")]}}))).await?;
    context.ready(&name, 1).await?;
    let logs = eventually("all finite log records", || async {
        let response = context
            .api("GET", &format!("{path}/logs?words={name}"), None)
            .await?;
        ensure(
            response["records"]
                .as_array()
                .is_some_and(|records| records.len() == 3),
            "exactly three fixture records must be collected",
        )?;
        Ok(response)
    })
    .await?;
    let end = jiff::Timestamp::now().to_string();
    let query = format!("{path}/logs?words={name}&end={end}");
    let full = context.api("GET", &query, None).await?;
    let records = full["records"]
        .as_array()
        .ok_or_else(|| support::ErrorKind::Check("missing logs".into()))?;
    ensure(
        records
            .windows(2)
            .all(|pair| pair[0]["timestamp"].as_str() >= pair[1]["timestamp"].as_str()),
        "logs must be newest first",
    )?;
    let mut pages = Vec::new();
    for skip in 0..3 {
        let page = context
            .api("GET", &format!("{query}&limit=1&skip={skip}"), None)
            .await?;
        pages.extend(
            page["records"]
                .as_array()
                .ok_or_else(|| support::ErrorKind::Check("missing page".into()))?
                .iter()
                .cloned(),
        );
    }
    ensure(
        &pages == records,
        "fixed-window pagination must preserve records and order",
    )?;
    ensure(
        context.api("GET", &format!("{query}&skip=3"), None).await?["records"] == json!([]),
        "pagination past the last record must be empty",
    )?;
    let filtered = context
        .api("GET", &format!("{query}&words=ALPHA&source=stdout"), None)
        .await?;
    ensure(
        filtered["records"].as_array().is_some_and(|records| {
            records.len() == 2 && records.iter().all(|record| record["source"] == "stdout")
        }),
        "word matching must be case insensitive and combine with source using AND",
    )?;
    ensure(
        context
            .api("GET", &format!("{query}&source=stderr"), None)
            .await?["records"]
            .as_array()
            .is_some_and(|records| {
                records.len() == 1
                    && records[0]["log"]
                        .as_str()
                        .is_some_and(|line| line.contains("Beta error"))
            }),
        "stderr filter must isolate the matching record",
    )?;
    ensure(
        full["service_id"] == logs["service_id"],
        "query filters must preserve the target service",
    )?;
    for query in [
        "limit=0",
        "limit=1001",
        "start=2030-01-01T00:00:00Z&end=2020-01-01T00:00:00Z",
        "start=2020-01-01T00:00:00Z&end=2021-01-01T00:00:00Z",
        "target=XJob",
        "job=unexpected",
    ] {
        support::send(
            context.request("GET", &format!("{path}/logs?{query}"))?,
            422,
        )
        .await?;
    }
    support::send(
        context.request("GET", &format!("{path}/logs?limit=bad"))?,
        400,
    )
    .await?;
    for query in ["max_buckets=0", "max_buckets=721"] {
        support::send(
            context.request("GET", &format!("{path}/metrics?{query}"))?,
            422,
        )
        .await?;
    }
    let foreign = support::name("foreign");
    context.app(&foreign, "unrelated").await?;
    ensure(
        context
            .api(
                "GET",
                &format!("/api/apps/{foreign}/logs?words={name}"),
                None,
            )
            .await?["records"]
            == json!([]),
        "logs must not leak between apps",
    )?;
    eventually("filtered creation audit", || async {
        let events = context
            .api(
                "GET",
                &format!("/api/events?subject={name}&kind=AppCreated"),
                None,
            )
            .await?;
        ensure(
            events["records"].as_array().is_some_and(|events| {
                events.len() == 1
                    && events[0]["subject"] == name
                    && events[0]["kind"] == "AppCreated"
            }),
            "subject and kind must jointly select the exact event",
        )
    })
    .await?;
    context
        .api("DELETE", &format!("/api/apps/{foreign}"), None)
        .await?;
    context.api("DELETE", &path, None).await?;
    Ok(())
}

pub(crate) async fn telemetry(context: &Context) -> Result<()> {
    let name = support::name("telemetry");
    let path = format!("/api/apps/{name}");
    context.api("POST","/api/apps",Some(json!({"name":name,"group":"tests","image":context.environment.images["fixture"].reference,"replicas":{"Fixed":1},"log_driver":{"driver":"Insight"},"container_options":{"command":["sh","-c",format!("mkdir -p /www; echo {name} > /www/index.html; while true; do echo {name}; sleep 1; done & exec httpd -f -p 8080 -h /www")]}}))).await?;
    context.ready(&name, 1).await?;
    for suffix in ["logs", "metrics"] {
        eventually("observability collection", || async {
            let records = context
                .api("GET", &format!("{path}/{suffix}"), None)
                .await?;
            let populated = match suffix {
                "logs" => records.to_string().contains(&name),
                _ => records.as_object().is_some_and(|object| {
                    object
                        .values()
                        .any(|value| value.as_array().is_some_and(|array| !array.is_empty()))
                }),
            };
            ensure(populated, &format!("no {suffix} records yet"))
        })
        .await?;
    }
    let nodes = context.api("GET", "/api/nodes", None).await?;
    eventually("application audit event", || async {
        let events = context
            .api("GET", &format!("/api/events?subject={name}"), None)
            .await?;
        ensure(
            events["records"]
                .as_array()
                .is_some_and(|records| records.iter().any(|record| record["kind"] == "AppCreated")),
            "application creation must be recorded in Insight events",
        )
    })
    .await?;
    let node = string(&nodes["nodes"][0], "id")?;
    eventually("host metrics", || async {
        let records = context
            .api("GET", &format!("/api/nodes/{node}/metrics"), None)
            .await?;
        ensure(
            records["host"]
                .as_array()
                .is_some_and(|values| !values.is_empty()),
            "host metrics must be collected",
        )
    })
    .await?;
    let current = context.api("GET", "/api/insight", None).await?;
    let other = if context.environment.profile == "single" {
        json!({"S3Compatible":{"endpoint":"http://localhost:19390","region":"garage","bucket_name":"insight","access_key_id":"invalid","secret_access_key":"invalid"}})
    } else {
        json!("Local")
    };
    support::send(
        context
            .request("PATCH", "/api/insight")?
            .json(&json!({"storage":other})),
        422,
    )
    .await?;
    ensure(
        current["server"]["storage"]
            == context.api("GET", "/api/insight", None).await?["server"]["storage"],
        "storage backend must remain fixed",
    )?;
    context.api("DELETE", &path, None).await?;
    Ok(())
}

pub(crate) async fn access(context: &Context) -> Result<()> {
    let name = support::name("access");
    let domain = format!("{name}.cdelivery.test");
    let path = format!("/api/apps/{name}");
    context
        .api(
            "POST",
            "/api/apps",
            Some(json!({
                "name":name,"group":"tests","image":context.environment.images["fixture"].reference,
                "replicas":{"Fixed":1},"domains":[{"domain":domain,"https":false,"target":8080}],
                "container_options":{"command":["sh","-c",format!("mkdir -p /www; echo {name} > /www/index.html; exec httpd -f -p 8080 -h /www")]}
            })),
        )
        .await?;
    context.ready(&name, 1).await?;
    for machine in &context.environment.machines {
        eventually("HTTP access on each gateway", || async {
            let (body, metadata) = context.curl(
                machine,
                &[
                    "--resolve",
                    &format!("{domain}:80:127.0.0.1"),
                    &format!("http://{domain}/"),
                ],
            )?;
            ensure(
                metadata["exitcode"] == 0 && metadata["http_code"] == 200 && body.trim() == name,
                "proxy must serve the app",
            )
        })
        .await?;
    }
    eventually("access records", || async {
        let records = context.api("GET", &format!("{path}/access"), None).await?;
        ensure(
            records.to_string().contains(&domain),
            "gateway requests must appear in Insight",
        )
    })
    .await?;
    context.api("DELETE", &path, None).await?;
    Ok(())
}
