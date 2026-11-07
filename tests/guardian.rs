use serde_json::json;

use crate::support::{self, Context, Result, ensure, eventually, send};

pub(crate) async fn decisions(context: &Context) -> Result<()> {
    for decision in ["retry", "cancel", "skip"] {
        let name = support::name("guardian");
        let path = format!("/api/apps/{name}");
        let guardian = format!("{path}/guardian");
        context.app(&name, "healthy").await?;
        context.ready(&name, 1).await?;
        context.api("PUT", &format!("{path}/xhealth"), Some(json!({"method":{"Hurl":{"file":"GET {{TARGET_URL}}\nHTTP 200\n[Asserts]\nbody == \"healthy\"\n","version":context.environment.images["hurl"].version}},"interval":{"unit":"Hours","value":24},"target":{"InternalPort":{"port":8080}},"guardian":true}))).await?;
        eventually("live health passes", || async {
            ensure(
                context.api("GET", &format!("{path}/xhealth"), None).await?["status"]
                    .get("Passed")
                    .is_some(),
                "live app must be healthy before the candidate",
            )
        })
        .await?;
        let live = context.api("GET", &path, None).await?;
        let command = json!([
            "sh",
            "-c",
            "mkdir -p /www; printf broken > /www/index.html; exec httpd -f -p 8080 -h /www"
        ]);
        send(
            context
                .request("PATCH", &path)?
                .json(&json!({"container_options":{"command":command}})),
            202,
        )
        .await?;
        eventually("candidate rejection", || async {
            ensure(
                context.api("GET", &guardian, None).await?["phase"] == "Failed",
                "candidate must fail before an operator decision",
            )
        })
        .await?;
        ensure(
            context.api("GET", &path, None).await?["container_options"]
                == live["container_options"],
            "failed candidate must not modify the live configuration",
        )?;
        if decision == "retry" {
            let failed_at = context.api("GET", &guardian, None).await?["at"].clone();
            context
                .api("POST", &format!("{guardian}/retry"), None)
                .await?;
            eventually("retry preserves failing verdict", || async {
                let retried = context.api("GET", &guardian, None).await?;
                ensure(
                    retried["phase"] == "Failed" && retried["at"] != failed_at,
                    "retry must execute the check again",
                )
            })
            .await?;
            context.api("DELETE", &guardian, None).await?;
        } else if decision == "skip" {
            context
                .api("POST", &format!("{guardian}/skip"), None)
                .await?;
            eventually("explicit skip promotes candidate", || async {
                ensure(
                    context.api("GET", &path, None).await?["container_options"]["command"]
                        == command,
                    "skip must promote the requested candidate",
                )
            })
            .await?;
        } else {
            context.api("DELETE", &guardian, None).await?;
            context.api("DELETE", &guardian, None).await?;
        }
        eventually("resolved guardian removed", || async {
            send(context.request("GET", &guardian)?, 404)
                .await
                .map(|_| ())
        })
        .await?;
        if decision != "skip" {
            ensure(
                context.api("GET", &path, None).await?["container_options"]
                    == live["container_options"],
                "cancel must preserve the live configuration",
            )?;
        }
        context.api("DELETE", &path, None).await?;
    }
    Ok(())
}
