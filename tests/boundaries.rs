use std::collections::BTreeSet;

use base64::Engine as _;
use serde_json::{Value, json};

use crate::support::{self, Context, Result, ensure, eventually, send, string};

#[expect(clippy::too_many_lines)]
pub(crate) async fn apps(context: &Context) -> Result<()> {
    let group = support::name("boundary");
    let group_path = format!("/api/groups/{group}");
    context
        .api("POST", "/api/groups", Some(json!({"name":group})))
        .await?;
    let name = support::name("boundary");
    let path = format!("/api/apps/{name}");
    let app = json!({"name":name,"group":group,"image":context.environment.images["fixture"].reference,"replicas":{"Fixed":1},"container_options":{"command":["sleep","86400"]}});
    context.api("POST", "/api/apps", Some(app.clone())).await?;
    context.ready(&name, 1).await?;
    send(context.request("POST", "/api/apps")?.json(&app), 409).await?;
    let busy: Value = send(context.request("DELETE", &group_path)?, 409)
        .await?
        .json()
        .await?;
    ensure(
        busy["code"] == "GroupBusy",
        "nonempty groups must not be removed",
    )?;
    let before = context.api("GET", &path, None).await?;
    for patch in [
        json!({"replicas":{"Fixed":0}}),
        json!({"replicas":{"Fixed":65537}}),
        json!({"replicas":{"Dynamic":{"min":2,"max":1}}}),
        json!({"ports":[{"target":0,"published":8080,"protocol":"Tcp"}]}),
        json!({"xconfigs":[{"Upload":{"target":"../escape","data_base64":"YQ=="}}]}),
        json!({"unexpected":true}),
    ] {
        send(context.request("PATCH", &path)?.json(&patch), 422).await?;
        ensure(
            context.api("GET", &path, None).await? == before,
            "invalid app updates must preserve the saved app",
        )?;
    }
    send(
        context
            .request("PATCH", &path)?
            .json(&json!({"replicas":null})),
        400,
    )
    .await?;
    ensure(
        context.api("GET", &path, None).await? == before,
        "null replica policy must preserve the app",
    )?;
    let bytes = vec![0x5a; 1_023_999];
    context.api("PATCH", &path, Some(json!({"xconfigs":[{"Upload":{"target":"/boundary.bin","data_base64":base64::engine::general_purpose::STANDARD.encode(&bytes)}}]}))).await?;
    context.ready(&name, 1).await?;
    let configured = context.api("GET", &path, None).await?;
    let config = string(&configured["xconfigs"][0], "config_name")?;
    let downloaded = send(
        context.request("GET", &format!("{path}/xconfig/{config}/file"))?,
        200,
    )
    .await?
    .bytes()
    .await?;
    ensure(
        downloaded.as_ref() == bytes,
        "large XConfig must preserve every byte",
    )?;
    let other = support::name("foreign");
    context.app(&other, "foreign").await?;
    send(
        context.request("GET", &format!("/api/apps/{other}/xconfig/{config}/file"))?,
        404,
    )
    .await?;
    context
        .api("DELETE", &format!("/api/apps/{other}"), None)
        .await?;
    let oversized = base64::engine::general_purpose::STANDARD.encode(vec![0x5a; 1_024_001]);
    send(
        context.request("PATCH", &path)?.json(
            &json!({"xconfigs":[{"Upload":{"target":"/large.bin","data_base64":oversized}}]}),
        ),
        422,
    )
    .await?;
    ensure(
        context.api("GET", &path, None).await?["xconfigs"] == configured["xconfigs"],
        "oversized upload must preserve existing files",
    )?;
    for index in 0..16 {
        context.api("PUT", &format!("{path}/xjobs/job-{index}"), Some(json!({"interval":{"unit":"Hours","value":24},"timeout":{"unit":"Minutes","value":1},"command":["true"]}))).await?;
    }
    let overflow: Value = send(context.request("PUT", &format!("{path}/xjobs/overflow"))?.json(&json!({"interval":{"unit":"Hours","value":24},"timeout":{"unit":"Minutes","value":1},"command":["true"]})), 422).await?.json().await?;
    ensure(
        overflow["code"] == "XJobLimit",
        "job limit must reject another managed runner",
    )?;
    ensure(
        context.api("GET", &format!("{path}/xjobs"), None).await?["jobs"]
            .as_array()
            .is_some_and(|jobs| jobs.len() == 16),
        "job limit must not leave an extra job",
    )?;
    context.api("DELETE", &path, None).await?;
    context.api("DELETE", &path, None).await?;
    eventually("dependent services removed", || async {
        ensure(
            context
                .docker(&[
                    "service",
                    "ls",
                    "--filter",
                    &format!("label=dev.cdelivery.app_name={name}"),
                    "--format",
                    "{{.Name}}",
                ])?
                .trim()
                .is_empty(),
            "deleting the app must remove every runner",
        )
    })
    .await?;
    eventually("group network released after app cleanup", || async {
        context.api("DELETE", &group_path, None).await
    })
    .await?;
    context.api("DELETE", &group_path, None).await?;
    Ok(())
}

#[expect(clippy::too_many_lines)]
pub(crate) async fn settings(context: &Context) -> Result<()> {
    for channel in ["email", "feishu", "lark", "slack", "telegram", "discord"] {
        let path = format!("/api/settings/channels/{channel}");
        let before = context.api("GET", &path, None).await?;
        ensure(
            before[channel].is_null(),
            "local acceptance must not have external channels configured",
        )?;
        for body in [json!({"unexpected":true}), json!({})] {
            send(context.request("PATCH", &path)?.json(&body), 422).await?;
            ensure(
                context.api("GET", &path, None).await? == before,
                "invalid channel settings must not persist",
            )?;
        }
        for _ in 0..2 {
            context.api("DELETE", &path, None).await?;
        }
    }
    let path = "/api/settings/agent";
    let before = context.api("GET", path, None).await?;
    ensure(
        before["agent"].is_null(),
        "local acceptance must not have an external model configured",
    )?;
    for (body, status) in [
        (json!({}), 422),
        (json!({"provider":null}), 400),
        (json!({"provider":"unknown"}), 422),
        (json!({"provider":"OpenAI","model":"acceptance"}), 422),
        (json!({"provider":"Ollama","model":"acceptance"}), 422),
        (
            json!({"provider":"Ollama","model":"acceptance","endpoint":"http://127.0.0.1:1","api_key":"unsupported"}),
            422,
        ),
        (
            json!({"provider":"OpenAI","model":"acceptance","endpoint":"http://127.0.0.1:1","api_key":"unused"}),
            422,
        ),
        (
            json!({"patrol_interval":{"unit":"Minutes","value":14}}),
            422,
        ),
        (json!({"patrol_interval":{"unit":"Hours","value":25}}), 422),
    ] {
        send(context.request("PATCH", path)?.json(&body), status).await?;
        ensure(
            context.api("GET", path, None).await? == before,
            "rejected agent configuration must preserve settings",
        )?;
    }
    for _ in 0..2 {
        context.api("DELETE", path, None).await?;
    }
    let saved = context.api("GET", "/api/me/im", None).await?;
    ensure(
        saved["bindings"] == json!({}),
        "local acceptance requires no external IM bindings",
    )?;
    let user = context.api("GET", "/api/me", None).await?;
    let preferences = user["preferences"].clone();
    let secret = format!("cdelivery-user-{}", string(&user, "id")?);
    let mut alerts: BTreeSet<String> = serde_json::from_value(preferences["alerts"].clone())?;
    let platforms = [
        ("Feishu", "ou_acceptance1", "ou_acceptance2"),
        ("Lark", "ou_acceptance1", "ou_acceptance2"),
        ("Slack", "U0123456789", "U9876543210"),
        ("Telegram", "123456789", "987654321"),
        ("Discord", "123456789012345678", "987654321098765432"),
    ];
    let result = async {
        for (platform, id, replacement) in platforms {
            context
                .api(
                    "PUT",
                    "/api/me/im",
                    Some(json!({"platform":platform,"id":id})),
                )
                .await?;
            ensure(
                context.api("GET", "/api/me/im", None).await?["bindings"][platform] == id,
                "IM binding must round trip the immutable ID",
            )?;
            alerts.insert(platform.to_owned());
            let subscribed = context.api("GET", "/api/me", None).await?["preferences"].clone();
            ensure(
                serde_json::from_value::<BTreeSet<String>>(subscribed["alerts"].clone())? == alerts
                    && subscribed["alert_email"] == preferences["alert_email"],
                "new bindings must enable only their own notifications",
            )?;
            send(
                context
                    .request("PUT", "/api/me/im")?
                    .json(&json!({"platform":platform,"id":"invalid!"})),
                422,
            )
            .await?;
            ensure(
                context.api("GET", "/api/me/im", None).await?["bindings"][platform] == id
                    && context.api("GET", "/api/me", None).await?["preferences"] == subscribed,
                "invalid binding must preserve the old ID and preferences",
            )?;
            alerts.remove(platform);
            context
                .api(
                    "PATCH",
                    "/api/me/preferences",
                    Some(json!({"alerts":alerts})),
                )
                .await?;
            let disabled = context.api("GET", "/api/me", None).await?["preferences"].clone();
            ensure(
                serde_json::from_value::<BTreeSet<String>>(disabled["alerts"].clone())? == alerts,
                "notifications must be independently disabled",
            )?;
            for target in [id, replacement] {
                let before = context.docker(&["secret", "inspect", &secret])?;
                context
                    .api(
                        "PUT",
                        "/api/me/im",
                        Some(json!({"platform":platform,"id":target})),
                    )
                    .await?;
                ensure(
                    context.api("GET", "/api/me/im", None).await?["bindings"][platform] == target
                        && context.api("GET", "/api/me", None).await?["preferences"] == disabled,
                    "repeating or replacing a binding must preserve notification preferences",
                )?;
                if target == id {
                    ensure(
                        before == context.docker(&["secret", "inspect", &secret])?,
                        "repeated binding must not persist another version",
                    )?;
                }
            }
            for _ in 0..2 {
                context
                    .api(
                        "PUT",
                        "/api/me/im",
                        Some(json!({"platform":platform,"id":null})),
                    )
                    .await?;
                ensure(
                    context.api("GET", "/api/me/im", None).await?["bindings"]
                        .get(platform)
                        .is_none()
                        && context.api("GET", "/api/me", None).await?["preferences"] == disabled,
                    "unbinding must remove only the identity and its notification subscription",
                )?;
                context
                    .api(
                        "PUT",
                        "/api/me/im",
                        Some(json!({"platform":platform,"id":id})),
                    )
                    .await?;
                ensure(
                    context.api("GET", "/api/me", None).await?["preferences"] == subscribed,
                    "rebinding must enable notifications without changing other preferences",
                )?;
            }
            alerts.insert(platform.to_owned());
        }
        Ok(())
    }
    .await;
    for (platform, _, _) in platforms {
        context
            .api(
                "PUT",
                "/api/me/im",
                Some(json!({"platform":platform,"id":null})),
            )
            .await?;
    }
    context
        .api(
            "PATCH",
            "/api/me/preferences",
            Some(json!({"alerts":preferences["alerts"]})),
        )
        .await?;
    result
}

pub(crate) async fn invitations(context: &Context) -> Result<()> {
    for mutation in ["demote", "revoke", "delete"] {
        let email = format!("{}@cdelivery.test", support::name("issuer"));
        let password = "Invitation-Aa123456";
        let user = context.api("POST", "/api/users", Some(json!({"first_name":"Invitation","last_name":mutation,"email":email,"password_sha256":support::hash(password),"role":"Admin"}))).await?;
        let session = context.login(&email, password).await?;
        let base = &context.environment.base_url;
        let key: Value = send(
            session
                .post(format!("{base}/api/apikeys"))
                .header("Origin", base)
                .json(&json!({"name":"issuer","role":"Admin"})),
            201,
        )
        .await?
        .json()
        .await?;
        let invitation: Value = send(
            context
                .client
                .post(format!("{base}/api/nodes/join-invitations"))
                .bearer_auth(string(&key, "key")?)
                .json(&json!({"role":"Worker"})),
            200,
        )
        .await?
        .json()
        .await?;
        let token = string(&invitation, "invitation")?;
        let redeem = || {
            context.client.post(format!("{base}/join/redeem")).bearer_auth(token).json(&json!({"advertise_addr":context.environment.addresses[&context.environment.machines[0]]}))
        };
        send(redeem(), 200).await?;
        let path = format!("/api/users/{}", string(&user, "id")?);
        match mutation {
            "demote" => {
                context
                    .api("PATCH", &path, Some(json!({"role":"Reader"})))
                    .await?;
            }
            "revoke" => {
                send(
                    session
                        .delete(format!("{base}/api/apikeys/{}", string(&key, "id")?))
                        .header("Origin", base),
                    204,
                )
                .await?;
            }
            _ => {
                context.api("DELETE", &path, None).await?;
            }
        }
        send(redeem(), 401).await?;
        context.api("DELETE", &path, None).await?;
    }
    let nodes = context.api("GET", "/api/nodes", None).await?;
    for node in nodes["nodes"]
        .as_array()
        .ok_or_else(|| support::ErrorKind::Check("missing nodes".into()))?
    {
        let path = format!("/api/nodes/{}", string(node, "id")?);
        let original = node["labels"].clone();
        let mut labels = original
            .as_array()
            .ok_or_else(|| support::ErrorKind::Check("missing labels".into()))?
            .clone();
        labels.push(json!("acceptance-extra"));
        context
            .api("PATCH", &path, Some(json!({"labels":labels})))
            .await?;
        let changed = context.api("GET", &path, None).await?;
        context
            .api("PATCH", &path, Some(json!({"labels":original})))
            .await?;
        ensure(
            changed["labels"]
                .as_array()
                .is_some_and(|labels| labels.contains(&json!("acceptance-extra"))),
            "label addition must be visible",
        )?;
        ensure(
            context.api("GET", &path, None).await?["labels"] == node["labels"],
            "relabel must replace, not merge, the set",
        )?;
    }
    Ok(())
}
