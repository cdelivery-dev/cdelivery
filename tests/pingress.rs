use serde_json::{Value, json};

use crate::support::{Context, Result, ensure, eventually, send};

#[expect(clippy::too_many_lines)]
pub(crate) async fn enable(context: &Context) -> Result<()> {
    let baseline = context.api("GET", "/api/pingress", None).await?;
    ensure(
        baseline["acme_enabled"] == false,
        "ACME must start disabled",
    )?;
    ensure(
        context.api("GET", "/api/pingress/certs", None).await?["certs"] == json!([]),
        "certificate inventory must start empty",
    )?;
    send(
        context.request("PUT", "/api/pingress")?.json(
            &json!({"version":baseline["version"],"placement":baseline["placement"],"acme":null}),
        ),
        409,
    )
    .await?;
    std::fs::write(
        context.root.join("pingress-baseline.json"),
        serde_json::to_vec(&baseline)?,
    )?;
    context.api("DELETE", "/api/pingress", None).await?;
    for _ in 0..2 {
        context.api("DELETE", "/api/pingress/account", None).await?;
        ensure(
            context
                .docker(&["secret", "ls", "--filter", "name=cdelivery-acme", "--quiet"])?
                .trim()
                .is_empty(),
            "disabling ACME without a record must not create one",
        )?;
    }
    let ca = json!({"Pebble":{"url":"https://pebble:19400/dir","root_ca_pem":include_str!("fixtures/pebble.minica.pem")}});
    let mut input =
        json!({"version":baseline["version"],"placement":baseline["placement"],"acme":ca});
    for (invalid, status) in [
        (json!(true), 400),
        (json!({}), 400),
        (json!({"LetsEncrypt":{}}), 422),
    ] {
        let mut request = input.clone();
        request["acme"] = invalid;
        send(
            context.request("PUT", "/api/pingress")?.json(&request),
            status,
        )
        .await?;
    }
    let mut invalid = input.clone();
    invalid["acme"]["Pebble"]["unexpected"] = json!(true);
    send(context.request("PUT", "/api/pingress")?.json(&invalid), 422).await?;
    invalid = input.clone();
    invalid["acme"]["Pebble"]["url"] = json!("https://pebble:19401/dir");
    send(context.request("PUT", "/api/pingress")?.json(&invalid), 502).await?;
    send(context.request("GET", "/api/pingress")?, 404).await?;

    context.docker(&[
        "service",
        "create",
        "--detach=true",
        "--name",
        "cdelivery-acme-stall",
        "--network",
        "cdelivery",
        &context.environment.images["fixture"].reference,
        "sh",
        "-c",
        "sleep 60 | nc -l -p 19401",
    ])?;
    eventually("ACME timeout fixture starts", || async {
        ensure(
            context
                .docker(&[
                    "service",
                    "ps",
                    "--filter",
                    "desired-state=running",
                    "--format",
                    "{{.CurrentState}}",
                    "cdelivery-acme-stall",
                ])?
                .starts_with("Running"),
            "ACME timeout fixture must be running",
        )
    })
    .await?;
    invalid["acme"]["Pebble"]["url"] = json!("https://cdelivery-acme-stall:19401/dir");
    let timeout: Value = send(context.request("PUT", "/api/pingress")?.json(&invalid), 504)
        .await?
        .json()
        .await?;
    ensure(
        timeout["code"] == "OperationTimeout",
        "registration must respect the operation budget",
    )?;
    send(context.request("GET", "/api/pingress")?, 404).await?;
    context.docker(&["service", "rm", "cdelivery-acme-stall"])?;

    context.engine(
        "POST",
        "/secrets/create",
        &json!({"Name":"cdelivery-acme","Data":"eA=="}),
    )?;
    send(context.request("PUT", "/api/pingress")?.json(&input), 409).await?;
    send(context.request("GET", "/api/pingress")?, 404).await?;
    let rejected = context.engine("GET", "/secrets/cdelivery-acme", &Value::Null)?;
    ensure(
        rejected["Spec"]["Labels"]["dev.cdelivery.acme_account"].is_null(),
        "failed account persistence must leave no saved account",
    )?;
    context.engine("DELETE", "/secrets/cdelivery-acme", &Value::Null)?;

    eventually("gateway network released", || async {
        context.docker(&["network", "rm", "cdelivery-pingress"])
    })
    .await?;
    let bridge = context.docker(&[
        "network",
        "create",
        "--driver",
        "bridge",
        "cdelivery-pingress",
    ])?;
    send(context.request("PUT", "/api/pingress")?.json(&input), 500).await?;
    send(context.request("GET", "/api/pingress")?, 404).await?;
    let prepared = context.engine("GET", "/secrets/cdelivery-acme", &Value::Null)?;
    ensure(
        prepared["Spec"]["Labels"]["dev.cdelivery.acme_account"].is_string(),
        "deployment failure must retain the prepared account",
    )?;
    send(context.request("PUT", "/api/pingress")?.json(&input), 500).await?;
    ensure(
        context.engine("GET", "/secrets/cdelivery-acme", &Value::Null)? == prepared,
        "retry must reuse the account without saving it again",
    )?;
    invalid["acme"]["Pebble"]["url"] = json!("https://pebble:19401/dir");
    send(context.request("PUT", "/api/pingress")?.json(&invalid), 502).await?;
    ensure(
        context.engine("GET", "/secrets/cdelivery-acme", &Value::Null)? == prepared,
        "failed CA replacement must preserve the old account",
    )?;
    input["acme"]["Pebble"]["url"] = json!("https://pebble:19400/dir?retry=1");
    send(context.request("PUT", "/api/pingress")?.json(&input), 500).await?;
    let replaced = context.engine("GET", "/secrets/cdelivery-acme", &Value::Null)?;
    ensure(
        replaced["Spec"]["Labels"]["dev.cdelivery.acme_account"]
            != prepared["Spec"]["Labels"]["dev.cdelivery.acme_account"],
        "a different CA configuration must replace the unused account",
    )?;
    context.docker(&["network", "rm", bridge.trim()])?;
    let deployed = context
        .api("PUT", "/api/pingress", Some(input.clone()))
        .await?;
    ensure(
        deployed["acme_enabled"] == true,
        "combined enable must prepare ACME",
    )?;
    ensure(
        context.engine("GET", "/secrets/cdelivery-acme", &Value::Null)? == replaced,
        "successful retry must reuse the prepared account",
    )?;
    let account = context
        .api(
            "PUT",
            "/api/pingress/account",
            Some(json!({"ca":input["acme"]})),
        )
        .await?;
    ensure(
        account["created"] == false,
        "standalone enable must reuse the same account",
    )?;
    let mismatch: Value = send(
        context
            .request("PUT", "/api/pingress/account")?
            .json(&json!({"ca":ca})),
        409,
    )
    .await?
    .json()
    .await?;
    ensure(
        mismatch["code"] == "AcmeCaMismatch",
        "running Pingress must retain its CA restriction",
    )?;
    input["acme"] = Value::Null;
    send(context.request("PUT", "/api/pingress")?.json(&input), 409).await?;
    ensure(
        context.engine("GET", "/secrets/cdelivery-acme", &Value::Null)? == replaced,
        "duplicate deployment must not change ACME",
    )?;

    let pair = rcgen::generate_simple_self_signed(vec!["pending.cdelivery.test".into()])?;
    context.api("PUT", "/api/pingress/certs/pending.cdelivery.test", Some(json!({"cert_chain_pem":pair.cert.pem(),"private_key_pem":pair.signing_key.serialize_pem()}))).await?;
    let certificates = context.api("GET", "/api/pingress/certs", None).await?;
    context.engine("DELETE", "/services/cdelivery-pingress", &Value::Null)?;
    let deployed = context.api("PUT", "/api/pingress", Some(input)).await?;
    ensure(
        deployed["acme_enabled"] == false,
        "explicit opt-out must clear a retained account",
    )?;
    ensure(
        context.api("GET", "/api/pingress/certs", None).await? == certificates,
        "opt-out must preserve uploaded certificates",
    )?;
    let disabled = context.engine("GET", "/secrets/cdelivery-acme", &Value::Null)?;
    for _ in 0..2 {
        context.api("DELETE", "/api/pingress/account", None).await?;
        ensure(
            context.engine("GET", "/secrets/cdelivery-acme", &Value::Null)? == disabled,
            "repeated disable must preserve uploaded certificates and the stored version",
        )?;
    }
    Ok(())
}

pub(crate) async fn restore(context: &Context) -> Result<()> {
    let path = context.root.join("pingress-baseline.json");
    if !path.exists() {
        return Ok(());
    }
    if !context
        .docker(&[
            "service",
            "ls",
            "--filter",
            "name=cdelivery-acme-stall",
            "--format",
            "{{.Name}}",
        ])?
        .trim()
        .is_empty()
    {
        context.docker(&["service", "rm", "cdelivery-acme-stall"])?;
    }
    for network in context
        .docker(&[
            "network",
            "ls",
            "--filter",
            "name=^cdelivery-pingress$",
            "--filter",
            "driver=bridge",
            "--format",
            "{{.ID}}",
        ])?
        .lines()
    {
        context.docker(&["network", "rm", network])?;
    }
    context
        .api("DELETE", "/api/pingress/certs/pending.cdelivery.test", None)
        .await?;
    if !context
        .docker(&[
            "secret",
            "ls",
            "--filter",
            "name=cdelivery-acme",
            "--format",
            "{{.Name}}",
        ])?
        .trim()
        .is_empty()
    {
        context.docker(&["secret", "rm", "cdelivery-acme"])?;
    }
    context.api("DELETE", "/api/pingress/account", None).await?;
    let baseline: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let status = context
        .request("GET", "/api/pingress")?
        .send()
        .await?
        .status()
        .as_u16();
    ensure(
        matches!(status, 200 | 404),
        "cannot determine Pingress state for restoration",
    )?;
    let mut input = json!({"version":baseline["version"],"placement":baseline["placement"]});
    let method = if status == 404 {
        input["acme"] = Value::Null;
        "PUT"
    } else {
        "PATCH"
    };
    context.api(method, "/api/pingress", Some(input)).await?;
    crate::upgrade::running(
        context,
        "cdelivery-pingress",
        &context.environment.images["pingress"],
    )
    .await?;
    Ok(())
}
