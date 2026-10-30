use serde_json::{Value, json};

use crate::support::{self, Context, Result, ensure, send, string};

#[expect(clippy::too_many_lines)]
pub(crate) async fn permissions(context: &Context) -> Result<()> {
    let base = &context.environment.base_url;
    send(support::client()?.get(format!("{base}/api/me")), 401).await?;
    let mut users = Vec::new();
    for role in ["Reader", "Operator", "Admin"] {
        let email = format!("{}@cdelivery.test", support::name("auth"));
        let password = "Test-Password-Aa123";
        let user = context.api("POST", "/api/users", Some(json!({"first_name":"Role","last_name":role,"email":email,"password_sha256":support::hash(password),"role":role}))).await?;
        let id = string(&user, "id")?.to_owned();
        let session = context.login(&email, password).await?;
        let key: Value = send(
            session
                .post(format!("{base}/api/apikeys"))
                .header("Origin", base)
                .json(&json!({"name":"auth","role":role})),
            201,
        )
        .await?
        .json()
        .await?;
        let token = string(&key, "key")?;
        for (client, bearer) in [(&session, false), (&context.client, true)] {
            let request = client.get(format!("{base}/api/groups"));
            let request = if bearer {
                request.bearer_auth(token)
            } else {
                request
            };
            send(request, 200).await?;
            let request = client
                .post(format!("{base}/api/groups"))
                .header("Origin", base)
                .header("Content-Type", "application/json")
                .body("{");
            let request = if bearer {
                request.bearer_auth(token)
            } else {
                request
            };
            send(request, if role == "Admin" { 400 } else { 403 }).await?;
            let request = client
                .post(format!("{base}/api/apps"))
                .header("Origin", base)
                .header("Content-Type", "application/json")
                .body("{");
            let request = if bearer {
                request.bearer_auth(token)
            } else {
                request
            };
            send(request, if role == "Reader" { 403 } else { 400 }).await?;
            let request = client
                .post(format!("{base}/api/apps/missing/xhealth/run"))
                .header("Origin", base);
            let request = if bearer {
                request.bearer_auth(token)
            } else {
                request
            };
            send(request, if role == "Reader" { 403 } else { 404 }).await?;
        }
        send(
            session
                .post(format!("{base}/apps/missing/xhealth/run"))
                .header("Origin", base)
                .form(&[("payload", "{")]),
            if role == "Reader" { 403 } else { 400 },
        )
        .await?;
        let listed: Value = send(session.get(format!("{base}/api/apikeys")), 200)
            .await?
            .json()
            .await?;
        ensure(
            listed["keys"]
                .as_array()
                .is_some_and(|keys| keys.iter().any(|entry| entry["id"] == key["id"])),
            "own key must be listed",
        )?;
        ensure(
            !listed.to_string().contains(token)
                && !user.to_string().contains(&support::hash(password)),
            "shared responses must not echo credentials",
        )?;
        if role != "Admin" {
            send(
                session
                    .post(format!("{base}/api/apikeys"))
                    .header("Origin", base)
                    .json(&json!({"name":"elevated","role":"Admin"})),
                403,
            )
            .await?;
        }
        users.push((id, email, session, key));
    }
    let (reader_id, reader_email, reader_session, reader_key) = &users[0];
    let (_, _, operator_session, operator_key) = &users[1];
    let foreign = string(operator_key, "id")?;
    send(
        reader_session
            .delete(format!("{base}/api/apikeys/{foreign}"))
            .header("Origin", base),
        204,
    )
    .await?;
    send(
        context
            .client
            .get(format!("{base}/api/me"))
            .bearer_auth(string(operator_key, "key")?),
        200,
    )
    .await?;
    context
        .api(
            "PATCH",
            &format!("/api/users/{reader_id}"),
            Some(json!({"role":"Admin"})),
        )
        .await?;
    send(
        context
            .client
            .post(format!("{base}/api/groups"))
            .bearer_auth(string(reader_key, "key")?)
            .json(&json!({})),
        403,
    )
    .await?;
    context
        .api(
            "PATCH",
            &format!("/api/users/{}", users[2].0),
            Some(json!({"role":"Reader"})),
        )
        .await?;
    for token in [None, Some(string(&users[2].3, "key")?)] {
        let request = users[2]
            .2
            .post(format!("{base}/api/groups"))
            .header("Origin", base)
            .json(&json!({}));
        let request = if let Some(token) = token {
            context
                .client
                .post(format!("{base}/api/groups"))
                .bearer_auth(token)
                .json(&json!({}))
        } else {
            request
        };
        send(request, 403).await?;
    }
    let new_password = "Changed-Password-Aa123";
    send(reader_session.put(format!("{base}/api/me/password")).header("Origin",base).json(&json!({"current_password_sha256":support::hash("Test-Password-Aa123"),"password_sha256":support::hash(new_password)})),204).await?;
    send(reader_session.get(format!("{base}/api/me")), 401).await?;
    send(
        context
            .client
            .get(format!("{base}/api/me"))
            .bearer_auth(string(reader_key, "key")?),
        200,
    )
    .await?;
    context.login(reader_email, new_password).await?;
    send(
        operator_session
            .delete(format!("{base}/api/apikeys/{foreign}"))
            .header("Origin", base),
        204,
    )
    .await?;
    send(
        context
            .client
            .get(format!("{base}/api/me"))
            .bearer_auth(string(operator_key, "key")?),
        401,
    )
    .await?;
    for (id, _, session, key) in &users {
        context
            .api("DELETE", &format!("/api/users/{id}"), None)
            .await?;
        send(session.get(format!("{base}/api/me")), 401).await?;
        send(
            context
                .client
                .get(format!("{base}/api/me"))
                .bearer_auth(string(key, "key")?),
            401,
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn setup(context: &Context) -> Result<()> {
    let before = context.api("GET", "/api/me", None).await?;
    let request = context.client.post(format!("{}/setup", context.environment.base_url))
        .header("Origin",&context.environment.base_url).header("Accept","application/json")
        .form(&[("payload",json!({"first_name":"Replacement","last_name":"Admin","email":"replacement@cdelivery.test","password_sha256":support::hash("Replacement-Aa123")}).to_string())]);
    let response: Value = send(request, 409).await?.json().await?;
    ensure(
        response.to_string().contains("AlreadyInitialized"),
        "repeated setup must report AlreadyInitialized",
    )?;
    ensure(
        before == context.api("GET", "/api/me", None).await?,
        "repeated setup must preserve the first administrator",
    )?;
    let container = context.docker(&[
        "ps",
        "--filter",
        "label=com.docker.swarm.service.name=cdelivery",
        "--format",
        "{{.ID}}",
    ])?;
    let recovery_email = format!("{}@cdelivery.test", support::name("recovery"));
    let output = context.docker(&[
        "exec",
        container.trim(),
        "/entrypoint",
        "add-admin",
        &recovery_email,
        "Recovery",
        "Admin",
    ])?;
    let password = output
        .trim()
        .split(": ")
        .last()
        .ok_or_else(|| support::ErrorKind::Check("recovery password missing".into()))?;
    let client = context.login(&recovery_email, password).await?;
    let recovered: Value = send(
        client.get(format!("{}/api/me", context.environment.base_url)),
        200,
    )
    .await?
    .json()
    .await?;
    ensure(
        recovered["role"] == "Admin",
        "host recovery must create an administrator",
    )?;
    context
        .api(
            "DELETE",
            &format!("/api/users/{}", string(&recovered, "id")?),
            None,
        )
        .await?;
    Ok(())
}
