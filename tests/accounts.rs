use jiff::{SignedDuration, Timestamp};
use serde_json::{Value, json};

use crate::support::{self, Context, Result, ensure, eventually, send, string};

#[expect(clippy::too_many_lines)]
pub(crate) async fn lifecycle(context: &Context) -> Result<()> {
    let base = &context.environment.base_url;
    let email = format!("{}@cdelivery.test", support::name("account"));
    let password = "Account-Aa123456";
    let body = json!({"first_name":"Account","last_name":"Acceptance","email":email,"password_sha256":support::hash(password),"role":"Admin"});
    let user = context
        .api("POST", "/api/users", Some(body.clone()))
        .await?;
    let id = string(&user, "id")?;
    let path = format!("/api/users/{id}");
    let session = context.login(&email, password).await?;
    let second = context.login(&email, password).await?;
    let profile: Value = send(session.get(format!("{base}/api/me")), 200)
        .await?
        .json()
        .await?;
    ensure(
        profile["preferences"]["alert_email"] == true,
        "new accounts must receive email notifications by default",
    )?;
    let key: Value = send(
        session
            .post(format!("{base}/api/apikeys"))
            .header("Origin", base)
            .json(&json!({"name":"account"})),
        201,
    )
    .await?
    .json()
    .await?;
    ensure(key["role"] == "Reader", "API keys must default to Reader")?;
    send(context.request("POST", "/api/users")?.json(&body), 409).await?;
    let before = context.docker(&["secret", "inspect", &format!("cdelivery-user-{id}")])?;
    for patch in [
        json!({}),
        json!({"first_name":"Account","last_name":"Acceptance"}),
    ] {
        context.api("PATCH", &path, Some(patch)).await?;
        ensure(
            before == context.docker(&["secret", "inspect", &format!("cdelivery-user-{id}")])?,
            "equivalent user changes must not persist another version",
        )?;
    }
    for request in [
        session.delete(format!("{base}{path}")),
        session
            .patch(format!("{base}{path}"))
            .json(&json!({"role":"Reader"})),
    ] {
        let problem: Value = send(request.header("Origin", base), 403)
            .await?
            .json()
            .await?;
        ensure(
            problem["code"] == "SelfImmutable",
            "administrator self protection",
        )?;
    }
    for patch in [
        json!({"first_name":null}),
        json!({"role":"admin"}),
        json!({"email":"invalid"}),
        json!({"unexpected":true}),
    ] {
        send(context.request("PATCH", &path)?.json(&patch), 422).await?;
        ensure(
            context.api("GET", &path, None).await? == user,
            "rejected user update must preserve the record",
        )?;
    }
    let (first, last) = tokio::join!(
        context
            .request("PATCH", &path)?
            .json(&json!({"first_name":"Concurrent"}))
            .send(),
        context
            .request("PATCH", &path)?
            .json(&json!({"last_name":"Writer"}))
            .send()
    );
    let first = first?.status().as_u16();
    let last = last?.status().as_u16();
    ensure(
        [200, 409].contains(&first) && [200, 409].contains(&last) && (first == 200 || last == 200),
        "concurrent writes must succeed or report an explicit conflict",
    )?;
    let stored = context.api("GET", &path, None).await?;
    ensure(
        (first != 200 || stored["first_name"] == "Concurrent")
            && (last != 200 || stored["last_name"] == "Writer"),
        "successful independent patches must not be lost",
    )?;
    for patch in [
        json!({"alert_email":false}),
        json!({"language":"ZhCn","theme":"Dark"}),
        json!({"language":null}),
    ] {
        send(
            session
                .patch(format!("{base}/api/me/preferences"))
                .header("Origin", base)
                .json(&patch),
            204,
        )
        .await?;
    }
    let profile: Value = send(session.get(format!("{base}/api/me")), 200)
        .await?
        .json()
        .await?;
    ensure(
        profile["preferences"]["language"].is_null()
            && profile["preferences"]["theme"] == "Dark"
            && profile["preferences"]["alert_email"] == false,
        "clearing language must preserve unrelated preferences",
    )?;
    for address in [format!("updated-{email}"), email.clone()] {
        context
            .api("PATCH", &path, Some(json!({"email":address})))
            .await?;
        let updated: Value = send(session.get(format!("{base}/api/me")), 200)
            .await?
            .json()
            .await?;
        ensure(
            updated["email"] == address && updated["preferences"] == profile["preferences"],
            "changing email must preserve notification and UI preferences",
        )?;
    }
    let reset = "Reset-Aa123456";
    context
        .api(
            "PATCH",
            &path,
            Some(json!({"password_sha256":support::hash(reset)})),
        )
        .await?;
    for client in [&session, &second] {
        send(client.get(format!("{base}/api/me")), 401).await?;
    }
    send(
        context
            .client
            .get(format!("{base}/api/me"))
            .bearer_auth(string(&key, "key")?),
        200,
    )
    .await?;
    let reset_session = context.login(&email, reset).await?;
    let container = context.docker(&[
        "ps",
        "--filter",
        "label=com.docker.swarm.service.name=cdelivery",
        "--format",
        "{{.ID}}",
    ])?;
    let rescued = context.docker(&["exec", container.trim(), "/entrypoint", "rescue", &email])?;
    let password = rescued
        .trim()
        .rsplit_once(": ")
        .ok_or_else(|| support::ErrorKind::Check("missing rescue password".into()))?
        .1;
    context.login(&email, password).await?;
    send(reset_session.get(format!("{base}/api/me")), 401).await?;
    send(
        context
            .client
            .get(format!("{base}/api/me"))
            .bearer_auth(string(&key, "key")?),
        200,
    )
    .await?;
    let secret_name = format!("cdelivery-user-{id}");
    let secret: Value =
        serde_json::from_str(&context.docker(&["secret", "inspect", &secret_name])?)?;
    let mut spec = secret[0]["Spec"].clone();
    spec["Labels"]["dev.cdelivery.user.data"] = json!("corrupted-ciphertext");
    context.engine(
        "POST",
        &format!(
            "/secrets/{secret_name}/update?version={}",
            secret[0]["Version"]["Index"]
        ),
        &spec,
    )?;
    let broken = context
        .client
        .get(format!("{base}/api/me"))
        .bearer_auth(string(&key, "key")?)
        .send()
        .await?;
    ensure(
        !broken.status().is_success(),
        "corrupt credentials must fail closed",
    )?;
    ensure(
        context.api("GET", "/api/users", None).await?["users"]
            .as_array()
            .is_some_and(|users| users.iter().all(|entry| entry["id"] != id)),
        "corrupt user must be excluded without breaking the list",
    )?;
    context.api("DELETE", &path, None).await?;
    context.api("DELETE", &path, None).await?;
    Ok(())
}

pub(crate) async fn credentials(context: &Context) -> Result<()> {
    let base = &context.environment.base_url;
    let session = context
        .login(&context.credentials.email, &context.credentials.password)
        .await?;
    for origin in [
        None,
        Some("https://other.cdelivery.test"),
        Some("null"),
        Some("http://127.0.0.1:19000/"),
    ] {
        let mut request = session.patch(format!("{base}/api/me")).json(&json!({}));
        if let Some(origin) = origin {
            request = request.header("Origin", origin);
        }
        send(request, 403).await?;
    }
    send(context.request("PATCH", "/api/me")?.json(&json!({})), 204).await?;
    for token in ["", "invalid", "Bearer invalid"] {
        send(
            context
                .client
                .get(format!("{base}/api/me"))
                .bearer_auth(token),
            401,
        )
        .await?;
    }
    for expires_at in [json!("2000-01-01T00:00:00Z"), json!("invalid")] {
        send(
            context
                .request("POST", "/api/apikeys")?
                .json(&json!({"name":"expired","expires_at":expires_at})),
            422,
        )
        .await?;
    }
    let expires = Timestamp::now()
        .checked_add(SignedDuration::from_secs(10))
        .map_err(|error| support::ErrorKind::Check(error.to_string()))?;
    let key = context
        .api(
            "POST",
            "/api/apikeys",
            Some(json!({"name":support::name("expiry"),"expires_at":expires})),
        )
        .await?;
    let token = string(&key, "key")?;
    send(
        context
            .client
            .get(format!("{base}/api/me"))
            .bearer_auth(token),
        200,
    )
    .await?;
    while Timestamp::now() <= expires {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    send(
        context
            .client
            .get(format!("{base}/api/me"))
            .bearer_auth(token),
        401,
    )
    .await?;
    context
        .api(
            "DELETE",
            &format!("/api/apikeys/{}", string(&key, "id")?),
            None,
        )
        .await?;
    let before = context.api("GET", "/api/me", None).await?;
    send(
        context
            .request("PATCH", "/api/me")?
            .header("Content-Type", "application/json")
            .body(" ".repeat(6 * 1024 * 1024 + 1)),
        413,
    )
    .await?;
    ensure(
        before == context.api("GET", "/api/me", None).await?,
        "oversized body must not modify the account",
    )?;
    Ok(())
}

#[expect(clippy::too_many_lines)]
pub(crate) async fn totp(context: &Context) -> Result<()> {
    let base = &context.environment.base_url;
    let email = format!("{}@cdelivery.test", support::name("totp"));
    let password = "Totp-Aa123456";
    let user = context.api("POST", "/api/users", Some(json!({"first_name":"TOTP","last_name":"Acceptance","email":email,"password_sha256":support::hash(password),"role":"Reader"}))).await?;
    let session = context.login(&email, password).await?;
    let enrollment: Value = send(session.get(format!("{base}/api/me/totp/enrollment")), 200)
        .await?
        .json()
        .await?;
    let secret = string(&enrollment, "secret_base32")?;
    ensure(
        string(&enrollment, "uri")?.starts_with("otpauth://totp/"),
        "enrollment must provide a TOTP URI",
    )?;
    let secret = totp_rs::Secret::try_from_base32(secret)
        .map_err(|error| support::ErrorKind::Check(error.to_string()))?;
    let generator = totp_rs::Builder::new()
        .with_secret(secret)
        .with_step_duration(30)
        .build()
        .map_err(|error| support::ErrorKind::Check(error.to_string()))?;
    let current_code = || -> Result<String> {
        let seconds = u64::try_from(Timestamp::now().as_second())
            .map_err(|error| support::ErrorKind::Check(error.to_string()))?;
        Ok(generator.generate(seconds).to_string())
    };
    let mut step = Timestamp::now().as_second() / 30;
    let code = current_code()?;
    let payload = json!({"current_password_sha256":support::hash(password),"secret_base32":enrollment["secret_base32"],"code":code});
    send(
        session
            .post(format!("{base}/api/me/totp"))
            .header("Origin", base)
            .json(&payload),
        204,
    )
    .await?;
    send(
        session
            .post(format!("{base}/api/me/totp"))
            .header("Origin", base)
            .json(&payload),
        409,
    )
    .await?;
    let login_client = support::client()?;
    let login = |code: Option<String>| {
        login_client
            .post(format!("{base}/login"))
            .header("Origin", base)
            .header("Accept", "application/json")
            .form(&[(
                "payload",
                json!({"email":email,"password_sha256":support::hash(password),"totp_code":code})
                    .to_string(),
            )])
    };
    send(login(None), 401).await?;
    send(login(Some(code)), 401).await?;
    eventually("next TOTP step", || async {
        ensure(
            Timestamp::now().as_second() / 30 > step,
            "waiting for unused code",
        )
    })
    .await?;
    step = Timestamp::now().as_second() / 30;
    let code = current_code()?;
    send(login(Some(code.clone())), 303).await?;
    send(login(Some(code)), 401).await?;
    for _ in 1..5 {
        let now = u64::try_from(Timestamp::now().as_second())
            .map_err(|error| support::ErrorKind::Check(error.to_string()))?;
        let invalid = generator.generate(now.saturating_sub(3600)).to_string();
        send(login(Some(invalid)), 401).await?;
    }
    let locked_at = tokio::time::Instant::now();
    eventually("new code while cooled down", || async {
        ensure(
            Timestamp::now().as_second() / 30 > step,
            "waiting for a fresh code",
        )
    })
    .await?;
    send(login(Some(current_code()?)), 401).await?;
    while locked_at.elapsed() < std::time::Duration::from_secs(301) {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    step = Timestamp::now().as_second() / 30;
    send(login(Some(current_code()?)), 303).await?;
    eventually("next code for disabling TOTP", || async {
        ensure(
            Timestamp::now().as_second() / 30 > step,
            "waiting for unused code",
        )
    })
    .await?;
    send(
        session
            .post(format!("{base}/api/me/totp/disable"))
            .header("Origin", base)
            .json(
                &json!({"current_password_sha256":support::hash(password),"code":current_code()?}),
            ),
        204,
    )
    .await?;
    context.login(&email, password).await?;
    context
        .api(
            "DELETE",
            &format!("/api/users/{}", string(&user, "id")?),
            None,
        )
        .await?;
    Ok(())
}
