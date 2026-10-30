use std::time::Duration;

use base64::Engine as _;
use reqwest::{Certificate, Client};
use serde_json::json;

use crate::support::{self, Context, Result, ensure, eventually, send, string};

fn ca(context: &Context, name: &str, pem: &str) -> Result<String> {
    let path = context.root.join(name);
    std::fs::write(&path, pem)?;
    Ok(format!("/mnt/mac{}", path.canonicalize()?.display()))
}

fn leaf(context: &Context, machine: &str, domain: &str, root: &str) -> Result<Vec<u8>> {
    let (body, metadata) = context.curl(
        machine,
        &[
            "--cacert",
            root,
            "--resolve",
            &format!("{domain}:443:127.0.0.1"),
            &format!("https://{domain}/"),
        ],
    )?;
    ensure(
        metadata["exitcode"] == 0 && metadata["http_code"] == 200 && body == "tls-ok",
        &format!("HTTPS must route to the requested app: {metadata}"),
    )?;
    let encoded = string(&metadata, "certs")?
        .split_once("-----BEGIN CERTIFICATE-----")
        .and_then(|(_, pem)| pem.split_once("-----END CERTIFICATE-----"))
        .ok_or_else(|| support::ErrorKind::Api("TLS peer certificate missing".into()))?
        .0;
    let encoded: String = encoded
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect();
    Ok(base64::engine::general_purpose::STANDARD.decode(encoded)?)
}

fn gateway_tasks(context: &Context) -> Result<Vec<String>> {
    let output = context.docker(&[
        "service",
        "ps",
        "--no-trunc",
        "--filter",
        "desired-state=running",
        "--format",
        "{{.ID}}",
        "cdelivery-pingress",
    ])?;
    let mut tasks: Vec<_> = output.lines().map(str::to_owned).collect();
    tasks.sort();
    ensure(
        tasks.len() == context.environment.machines.len(),
        "one gateway task per node",
    )?;
    Ok(tasks)
}

async fn prepare(context: &Context, name: &str) -> Result<String> {
    let path = format!("/api/apps/{name}");
    send(context.request("DELETE", &path)?, 204).await?;
    context.app(name, "tls-ok").await?;
    context.ready(name, 1).await?;
    Ok(path)
}

#[expect(clippy::too_many_lines, clippy::print_stdout)]
pub(crate) async fn certificates(context: &Context) -> Result<()> {
    let machines = &context.environment.machines;
    let path = prepare(context, "tls-manual").await?;
    let pair = rcgen::generate_simple_self_signed(vec!["static.cdelivery.test".into()])?;
    let pem = pair.cert.pem();
    let root = ca(context, "manual-ca.pem", &pem)?;
    let key = pair.signing_key.serialize_pem();
    let upload = json!({"cert_chain_pem":pem,"private_key_pem":key});
    context
        .api(
            "PUT",
            "/api/pingress/certs/static.cdelivery.test",
            Some(upload.clone()),
        )
        .await?;
    send(
        context
            .request("PUT", "/api/pingress/certs/wrong.cdelivery.test")?
            .json(&upload),
        422,
    )
    .await?;
    let other = rcgen::KeyPair::generate()?;
    send(
        context
            .request("PUT", "/api/pingress/certs/static.cdelivery.test")?
            .json(&json!({"cert_chain_pem":pem,"private_key_pem":other.serialize_pem()})),
        422,
    )
    .await?;
    let mut expired = rcgen::CertificateParams::new(vec!["expired.cdelivery.test".into()])?;
    expired.not_before = rcgen::date_time_ymd(2000, 1, 1);
    expired.not_after = rcgen::date_time_ymd(2001, 1, 1);
    let expired = expired.self_signed(&other)?;
    send(
        context
            .request("PUT", "/api/pingress/certs/expired.cdelivery.test")?
            .json(&json!({"cert_chain_pem":expired.pem(),"private_key_pem":other.serialize_pem()})),
        422,
    )
    .await?;
    context
        .api(
            "PATCH",
            &path,
            Some(
                json!({"domains":[{"domain":"static.cdelivery.test","https":true,"target":8080}]}),
            ),
        )
        .await?;
    for machine in machines {
        eventually("static TLS on each gateway", || async {
            ensure(
                leaf(context, machine, "static.cdelivery.test", &root)? == pair.cert.der().as_ref(),
                "gateway must serve the uploaded certificate",
            )
        })
        .await?;
    }
    let initial_tasks = gateway_tasks(context)?;
    for machine in machines {
        for tls in ["1.2", "1.3"] {
            let minimum = format!("--tlsv{tls}");
            for (option, version) in [("--http1.1", "1.1"), ("--http2", "2")] {
                let (body, metadata) = context.curl(
                    machine,
                    &[
                        &minimum,
                        "--tls-max",
                        tls,
                        option,
                        "--cacert",
                        &root,
                        "--resolve",
                        "static.cdelivery.test:443:127.0.0.1",
                        "https://static.cdelivery.test/",
                    ],
                )?;
                ensure(
                    metadata["exitcode"] == 0
                        && metadata["http_code"] == 200
                        && metadata["http_version"] == version
                        && body == "tls-ok",
                    &format!("TLS {tls} HTTP/{version} routing: {metadata}"),
                )?;
                println!("PASS: {machine} TLS {tls} HTTP/{version}");
            }
            for (domain, sni) in [
                ("wrong.cdelivery.test", true),
                ("static.cdelivery.test", false),
            ] {
                let url = if sni {
                    format!("https://{domain}/")
                } else {
                    "https://127.0.0.1/".into()
                };
                let (_, metadata) = context.curl(
                    machine,
                    &[
                        &minimum,
                        "--tls-max",
                        tls,
                        "--insecure",
                        "--resolve",
                        &format!("{domain}:443:127.0.0.1"),
                        "--header",
                        &format!("Host: {domain}"),
                        &url,
                    ],
                )?;
                ensure(
                    metadata["exitcode"] == 35
                        && metadata["http_code"] == 0
                        && string(&metadata, "errormsg")?.contains("alert"),
                    &format!("unknown or missing SNI must receive a TLS alert: {metadata}"),
                )?;
                leaf(context, machine, "static.cdelivery.test", &root)?;
                println!(
                    "PASS: {machine} TLS {tls} rejected domain={domain} sni={sni}; valid request still succeeds"
                );
            }
        }
    }
    let replacement = rcgen::generate_simple_self_signed(vec!["static.cdelivery.test".into()])?;
    let pem = replacement.cert.pem();
    let root = ca(context, "replacement-ca.pem", &pem)?;
    context
        .api(
            "PUT",
            "/api/pingress/certs/static.cdelivery.test",
            Some(json!({"cert_chain_pem":pem,"private_key_pem":replacement.signing_key.serialize_pem()})),
        )
        .await?;
    for machine in machines {
        eventually("manual certificate replacement on each gateway", || async {
            ensure(
                leaf(context, machine, "static.cdelivery.test", &root)?
                    == replacement.cert.der().as_ref(),
                "gateway must serve the replacement certificate",
            )
        })
        .await?;
    }
    ensure(
        initial_tasks == gateway_tasks(context)?,
        "TLS checks and certificate replacement must not restart gateways",
    )?;
    println!("PASS: manual certificate validation, hot replacement and gateway survival");
    context.api("DELETE", &path, None).await?;
    context
        .api("DELETE", "/api/pingress/certs/static.cdelivery.test", None)
        .await?;
    Ok(())
}

#[expect(clippy::too_many_lines, clippy::print_stdout)]
pub(crate) async fn acme(context: &Context) -> Result<()> {
    let machines = &context.environment.machines;
    let path = prepare(context, "tls-acme").await?;
    context.api("DELETE", "/api/pingress/account", None).await?;
    context
        .api(
            "PATCH",
            &path,
            Some(json!({"domains":[{"domain":"http.cdelivery.test","target":8080}]})),
        )
        .await?;
    let initial_tasks = gateway_tasks(context)?;
    let transport_ca = include_str!("fixtures/pebble.minica.pem");
    let management = Client::builder()
        .no_proxy()
        .user_agent("cdelivery-tests")
        .timeout(Duration::from_secs(20))
        .tls_certs_only([Certificate::from_pem(transport_ca.as_bytes())?])
        .build()?;
    let root = send(management.get("https://localhost:19501/roots/0"), 200)
        .await?
        .text()
        .await?;
    let root = ca(context, "acme-ca.pem", &root)?;
    let account = context.api("PUT","/api/pingress/account",Some(json!({"ca":{"Pebble":{"url":"https://pebble:19400/dir","root_ca_pem":transport_ca}}}))).await?;
    let manager = &context.environment.addresses[&context.environment.machines[0]];
    for (endpoint, body) in [
        (
            "add-a",
            json!({"host":"http.cdelivery.test.","addresses":[manager]}),
        ),
        (
            "set-txt",
            json!({"host":"_validation-persist.dns.cdelivery.test.","value":format!("pebble.letsencrypt.org; accounturi={}",string(&account,"uri")?)}),
        ),
    ] {
        send(
            context
                .client
                .post(format!("http://localhost:19805/{endpoint}"))
                .json(&body),
            200,
        )
        .await?;
    }
    eventually("HTTP challenge route", || async {
        let (body, metadata) = context.curl(
            &machines[0],
            &[
                "--resolve",
                "http.cdelivery.test:80:127.0.0.1",
                "http://http.cdelivery.test/",
            ],
        )?;
        ensure(
            metadata["exitcode"] == 0 && metadata["http_code"] == 200 && body == "tls-ok",
            "HTTP domain routing",
        )
    })
    .await?;
    context.api("PATCH",&path,Some(json!({"domains":[{"domain":"http.cdelivery.test","https":true,"target":8080},{"domain":"dns.cdelivery.test","https":true,"target":8080}]}))).await?;
    context.docker(&["service", "update", "--detach=true", "--force", "cdelivery"])?;
    for domain in ["http.cdelivery.test", "dns.cdelivery.test"] {
        let certificate = eventually("ACME issuance", || async {
            leaf(context, &machines[0], domain, &root)
        })
        .await?;
        for machine in machines {
            eventually("ACME certificate distribution", || async {
                ensure(
                    leaf(context, machine, domain, &root)? == certificate,
                    "gateways must serve the same issued certificate",
                )
            })
            .await?;
        }
        println!("PASS: ACME issuance and distribution for {domain}");
    }
    let issued = leaf(context, &machines[0], "dns.cdelivery.test", &root)?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&issued);
    let lines = encoded
        .as_bytes()
        .chunks(64)
        .map(String::from_utf8_lossy)
        .collect::<Vec<_>>()
        .join("\n");
    let certificate = format!("-----BEGIN CERTIFICATE-----\n{lines}\n-----END CERTIFICATE-----\n");
    send(management.post("https://localhost:19501/set-renewal-info/").json(&json!({"Certificate":certificate,"ARIResponse":json!({"suggestedWindow":{"start":"2000-01-01T00:00:00Z","end":"2000-01-02T00:00:00Z"}}).to_string()})),200).await?;
    context.docker(&["service", "update", "--detach=true", "--force", "cdelivery"])?;
    let renewed = eventually("ARI renewal", || async {
        let certificate = leaf(context, &machines[0], "dns.cdelivery.test", &root)?;
        ensure(
            certificate != issued,
            "renewal must replace the leaf certificate",
        )?;
        Ok(certificate)
    })
    .await?;
    for machine in machines {
        eventually("renewed certificate distribution", || async {
            ensure(
                leaf(context, machine, "dns.cdelivery.test", &root)? == renewed,
                "gateways must serve the same renewed certificate",
            )
        })
        .await?;
    }
    let certs = context.api("GET", "/api/pingress/certs", None).await?;
    ensure(
        initial_tasks == gateway_tasks(context)?,
        "ACME issuance and renewal must not restart gateways",
    )?;
    println!("PASS: ARI renewal, certificate distribution and gateway survival");
    ensure(
        !certs.to_string().contains("PRIVATE KEY"),
        "certificate inventory must not expose private keys",
    )?;
    context.api("DELETE", &path, None).await?;
    context.api("DELETE", "/api/pingress/account", None).await?;
    Ok(())
}
