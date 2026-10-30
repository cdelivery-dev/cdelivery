use serde_json::json;

use crate::support::{self, Context, Result, ensure, eventually, string};

pub(crate) async fn telemetry(context: &Context) -> Result<()> {
    let name = support::name("telemetry");
    let path = format!("/api/apps/{name}");
    context.api("POST","/api/apps",Some(json!({"name":name,"group":"tests","image":context.environment.images["fixture"],"replicas":{"Fixed":1},"log_driver":{"driver":"Insight"},"container_options":{"command":["sh","-c",format!("mkdir -p /www; echo {name} > /www/index.html; while true; do echo {name}; sleep 1; done & exec httpd -f -p 8080 -h /www")]}}))).await?;
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
                "name":name,"group":"tests","image":context.environment.images["fixture"],
                "replicas":{"Fixed":1},"domains":[{"domain":domain,"target":8080}],
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
