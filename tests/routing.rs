use serde_json::json;

use crate::support::{self, Context, Result, ensure, eventually};

#[expect(clippy::too_many_lines)]
pub(crate) async fn lifecycle(context: &Context) -> Result<()> {
    let domain = format!("{}.cdelivery.test", support::name("routing"));
    let fallback = support::name("fallback");
    let prefix = support::name("prefix");
    let conditional = support::name("condition");
    for (name, route) in [
        (
            &fallback,
            json!({"domain":domain,"https":false,"target":8080}),
        ),
        (
            &prefix,
            json!({"domain":domain,"https":false,"target":8080,"path_prefix":"/api"}),
        ),
        (
            &conditional,
            json!({"domain":domain,"https":false,"target":8080,"path_prefix":"/api","header":{"key":"x-route","value":"canary"},"cookie":{"key":"variant","value":"canary"}}),
        ),
    ] {
        context.api("POST", "/api/apps", Some(json!({"name":name,"group":"tests","image":context.environment.images["fixture"].reference,"replicas":{"Fixed":1},"domains":[route],"container_options":{"command":["sh","-c",format!("mkdir -p /www/api /www/api/nested /www/apix; for dir in /www /www/api /www/api/nested /www/apix; do printf {name} > $dir/index.html; done; exec httpd -f -p 8080 -h /www")]}}))).await?;
        context.ready(name, 1).await?;
    }
    for machine in &context.environment.machines {
        for (path, header, cookie, expected) in [
            ("/", "other", "other", &fallback),
            ("/api/", "other", "other", &prefix),
            ("/api/nested/", "other", "other", &prefix),
            ("/apix/", "canary", "canary", &fallback),
            ("/api/", "canary", "other", &prefix),
            ("/api/", "other", "canary", &prefix),
            ("/api/", "canary", "canary", &conditional),
        ] {
            eventually("route matching and precedence", || async {
                let (body, metadata) = context.curl(
                    machine,
                    &[
                        "--resolve",
                        &format!("{domain}:80:127.0.0.1"),
                        "--header",
                        &format!("X-Route: {header}"),
                        "--cookie",
                        &format!("variant={cookie}"),
                        &format!("http://{domain}{path}"),
                    ],
                )?;
                ensure(
                    metadata["http_code"] == 200 && body == *expected,
                    &format!("unexpected route for {path}: {metadata}; {body}"),
                )
            })
            .await?;
        }
    }
    let saved_domain = context.api("GET", "/api/instance", None).await?["domain"].clone();
    context
        .api(
            "PATCH",
            "/api/instance",
            Some(json!({"domain":{"domain":domain,"https":false}})),
        )
        .await?;
    let result = async {
        for machine in &context.environment.machines {
            eventually("platform domain takes precedence", || async {
                let (body, metadata) = context.curl(
                    machine,
                    &[
                        "--resolve",
                        &format!("{domain}:80:127.0.0.1"),
                        &format!("http://{domain}/login"),
                    ],
                )?;
                ensure(
                    metadata["http_code"] == 200 && body.contains("login_form"),
                    "platform domain must resolve to the login page",
                )
            })
            .await?;
        }
        Ok::<(), support::Error>(())
    }
    .await;
    context
        .api(
            "PATCH",
            "/api/instance",
            Some(json!({"domain":saved_domain})),
        )
        .await?;
    result?;
    for machine in &context.environment.machines {
        eventually("cleared platform domain restores app route", || async {
            let (body, metadata) = context.curl(
                machine,
                &[
                    "--resolve",
                    &format!("{domain}:80:127.0.0.1"),
                    &format!("http://{domain}/"),
                ],
            )?;
            ensure(
                metadata["http_code"] == 200 && body == fallback,
                "clearing the platform domain must invalidate its route",
            )
        })
        .await?;
    }
    context
        .api(
            "PATCH",
            &format!("/api/apps/{prefix}"),
            Some(json!({"domains":[]})),
        )
        .await?;
    context
        .api("DELETE", &format!("/api/apps/{conditional}"), None)
        .await?;
    for machine in &context.environment.machines {
        eventually("removed routes expire on every gateway", || async {
            let (body, metadata) = context.curl(
                machine,
                &[
                    "--resolve",
                    &format!("{domain}:80:127.0.0.1"),
                    "--header",
                    "X-Route: canary",
                    "--cookie",
                    "variant=canary",
                    &format!("http://{domain}/api/"),
                ],
            )?;
            ensure(
                metadata["http_code"] == 200 && body == fallback,
                "deleted routes must stop selecting their old upstreams",
            )
        })
        .await?;
    }
    context
        .api("DELETE", &format!("/api/apps/{fallback}"), None)
        .await?;
    for machine in &context.environment.machines {
        eventually("deleted domain expires", || async {
            let (_, metadata) = context.curl(
                machine,
                &[
                    "--resolve",
                    &format!("{domain}:80:127.0.0.1"),
                    &format!("http://{domain}/"),
                ],
            )?;
            ensure(
                metadata["http_code"] == 404,
                "unknown domain must not reuse a deleted route",
            )
        })
        .await?;
    }
    context
        .api("DELETE", &format!("/api/apps/{prefix}"), None)
        .await?;
    Ok(())
}
