use std::sync::Arc;
use std::time::Duration;

use axum::{Json, Router, extract, http::StatusCode, routing};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufStream};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tokio_rustls::{TlsAcceptor, rustls};

use crate::support::{Context, Result, ensure};

#[derive(Debug, Default, serde::Serialize)]
pub(super) struct State {
    pub(super) mode: String,
    mail: Vec<Value>,
    attempts: Vec<Value>,
    calls: Vec<Value>,
}

pub(super) struct Fixture {
    pub(super) http_port: u16,
    pub(super) smtp_port: u16,
    pub(super) state: Arc<Mutex<State>>,
    tasks: JoinSet<()>,
}

impl Fixture {
    #[expect(clippy::print_stderr)]
    pub(super) async fn start(context: &Context) -> Result<Self> {
        let key = rcgen::KeyPair::generate()?;
        let mut ca = rcgen::CertificateParams::new(Vec::new())?;
        ca.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        ca.key_usages.push(rcgen::KeyUsagePurpose::KeyCertSign);
        std::fs::write(
            context.root.join("duty-ca.pem"),
            ca.self_signed(&key)?.pem(),
        )?;
        let issuer = rcgen::Issuer::new(ca, key);
        let key = rcgen::KeyPair::generate()?;
        let certificate = rcgen::CertificateParams::new(vec!["duty.cdelivery.test".into()])?
            .signed_by(&key, &issuer)?;
        let tls = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![certificate.der().clone()],
                rustls::pki_types::PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
            )
            .map_err(std::io::Error::other)?;
        let http = TcpListener::bind(("0.0.0.0", 0)).await?;
        let smtp = TcpListener::bind(("0.0.0.0", 0)).await?;
        let mut fixture = Self {
            http_port: http.local_addr()?.port(),
            smtp_port: smtp.local_addr()?.port(),
            state: Arc::new(Mutex::new(State {
                mode: "ok".into(),
                ..State::default()
            })),
            tasks: JoinSet::new(),
        };
        let router = Router::new()
            .route("/v1/chat/completions", routing::post(model))
            .with_state(fixture.state.clone());
        fixture.tasks.spawn(async move {
            if let Err(error) = axum::serve(http, router).await {
                eprintln!("duty HTTP fixture failed: {error:?}");
            }
        });
        let state = fixture.state.clone();
        fixture.tasks.spawn(async move {
            if let Err(error) = serve_smtp(smtp, TlsAcceptor::from(Arc::new(tls)), state).await {
                eprintln!("duty SMTP fixture failed: {error:?}");
            }
        });
        Ok(fixture)
    }

    pub(super) async fn snapshot(&self) -> Value {
        json!(*self.state.lock().await)
    }

    pub(super) async fn stop(mut self) -> Result<()> {
        self.tasks.shutdown().await;
        for port in [self.http_port, self.smtp_port] {
            TcpListener::bind(("0.0.0.0", port)).await?;
        }
        Ok(())
    }
}

async fn model(
    extract::State(state): extract::State<Arc<Mutex<State>>>,
    Json(request): Json<Value>,
) -> std::result::Result<Json<Value>, (StatusCode, String)> {
    let messages: Vec<_> = request["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    let prompt = messages
        .iter()
        .map(|message| message["content"].as_str().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    let probe = request["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|tool| tool["function"]["name"] == "cdelivery_probe");
    let alert = prompt.contains("Alert batch: true");
    let tool_result = messages.iter().any(|message| message["role"] == "tool");
    let events: Value = if probe {
        json!([])
    } else {
        let events = prompt
            .split_once("New events:\n")
            .and_then(|(_, tail)| tail.split_once("\nRecorded diagnostics"))
            .map(|(events, _)| events)
            .ok_or_else(|| (StatusCode::BAD_REQUEST, "missing duty events".into()))?;
        serde_json::from_str(events)
            .map_err(|error| (StatusCode::BAD_REQUEST, error.to_string()))?
    };
    let mode = {
        let mut state = state.lock().await;
        let mode = state.mode.clone();
        state.calls.push(json!({
            "mode": mode, "alert": alert, "tool_result": tool_result, "events": events,
            "time": jiff::Timestamp::now(),
        }));
        mode
    };
    let mut content = "NOOP".to_owned();
    let mut call = None;
    if probe {
        call = Some(("cdelivery_probe", json!({"value":"ready"})));
    } else if alert {
        if mode == "model-error" {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "fixture unavailable".into(),
            ));
        }
        if mode == "timeout" {
            tokio::time::sleep(Duration::from_secs(70)).await;
        }
        if matches!(mode.as_str(), "cancel" | "demote") || mode == "receipt-cancel" && tool_result {
            tokio::time::sleep(Duration::from_secs(12)).await;
        }
        match mode.as_str() {
            "tool-error" => call = Some(("app_status", json!({"name":"cdelivery-insight"}))),
            "receipt" => call = Some(("app_restart", json!({"name":"missing-duty-app"}))),
            "receipt-cancel" if !tool_result => {
                call = Some(("app_restart", json!({"name":events[0]["subject"]})));
            }
            "read" if !tool_result => call = Some(("platform_overview", json!({}))),
            "empty" => content.clear(),
            "noop" => {}
            _ => content = format!("AI duty result: {events}"),
        }
    } else if events
        .as_array()
        .is_some_and(|events| events.iter().any(|event| event["kind"] != "Patrol"))
    {
        content = format!("Unexpected ordinary events: {events}");
    } else if mode == "patrol" {
        content = "AI patrol finding".into();
    }
    let mut message = json!({"role":"assistant", "content":content});
    if let Some((name, arguments)) = &call {
        message["content"] = Value::Null;
        message["tool_calls"] = json!([{
            "id":"fixture-call", "type":"function",
            "function":{"name":name,"arguments":arguments.to_string()},
        }]);
    }
    Ok(Json(json!({
        "id":"fixture", "object":"chat.completion", "created":jiff::Timestamp::now().as_second(),
        "model":"duty-fixture",
        "choices":[{"index":0,"message":message,"finish_reason":if call.is_some() {"tool_calls"} else {"stop"}}],
        "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2},
    })))
}

#[expect(clippy::print_stderr)]
async fn serve_smtp(
    listener: TcpListener,
    tls: TlsAcceptor,
    state: Arc<Mutex<State>>,
) -> Result<()> {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            incoming = listener.accept() => {
                let (stream, _) = incoming?;
                connections.spawn(smtp(stream, tls.clone(), state.clone()));
            }
            Some(result) = connections.join_next() => {
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => eprintln!("duty SMTP session failed: {error:?}"),
                    Err(error) => eprintln!("duty SMTP task failed: {error:?}"),
                }
            }
        }
    }
}

async fn smtp(stream: TcpStream, tls: TlsAcceptor, state: Arc<Mutex<State>>) -> Result<()> {
    let mut stream = BufStream::new(stream);
    stream.write_all(b"220 duty fixture\r\n").await?;
    let mut line = String::new();
    loop {
        stream.flush().await?;
        line.clear();
        if stream.read_line(&mut line).await? == 0 {
            return Ok(());
        }
        if line.trim() == "STARTTLS" {
            stream.write_all(b"220 TLS ready\r\n").await?;
            stream.flush().await?;
            break;
        }
        ensure(line.starts_with("EHLO "), "expected EHLO before STARTTLS")?;
        stream.write_all(b"250-fixture\r\n250 STARTTLS\r\n").await?;
    }
    let mut stream = BufStream::new(tls.accept(stream.into_inner()).await?);
    let mut recipients = Vec::new();
    loop {
        stream.flush().await?;
        line.clear();
        match stream.read_line(&mut line).await {
            Ok(0) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            result => {
                result?;
            }
        }
        let command = line.split_whitespace().next().unwrap_or_default();
        let response = match command {
            "EHLO" => "250-fixture\r\n250 AUTH PLAIN\r\n",
            "AUTH" => "235 Authenticated\r\n",
            "MAIL" | "RSET" => {
                recipients.clear();
                "250 OK\r\n"
            }
            "NOOP" => "250 OK\r\n",
            "RCPT" => {
                let (_, address) = line.split_once(':').ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "missing recipient")
                })?;
                recipients.push(address.trim().trim_matches(['<', '>']).to_owned());
                "250 Recipient accepted\r\n"
            }
            "DATA" => {
                let reject = state.lock().await.mode == "channel-error";
                stream.write_all(b"354 Send message\r\n").await?;
                stream.flush().await?;
                let mut data = String::new();
                loop {
                    line.clear();
                    ensure(
                        stream.read_line(&mut line).await? != 0,
                        "incomplete SMTP DATA",
                    )?;
                    if line == ".\r\n" {
                        break;
                    }
                    if let Some(tail) = line.strip_prefix("..") {
                        data.push('.');
                        data.push_str(tail);
                    } else {
                        data.push_str(&line);
                    }
                }
                let message = mail_parser::MessageParser::default()
                    .parse(data.as_bytes())
                    .ok_or_else(|| {
                        std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid email")
                    })?;
                let text = message.body_text(0).ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "missing email body")
                })?;
                let attempt = json!({"to":recipients,"text":text,"time":jiff::Timestamp::now()});
                let mut state = state.lock().await;
                state.attempts.push(attempt.clone());
                if !reject {
                    state.mail.push(attempt);
                }
                if reject {
                    "550 Fixture rejects delivery\r\n"
                } else {
                    "250 Stored\r\n"
                }
            }
            "QUIT" => {
                stream.write_all(b"221 Bye\r\n").await?;
                stream.flush().await?;
                return Ok(());
            }
            _ => "500 Unsupported command\r\n",
        };
        stream.write_all(response.as_bytes()).await?;
    }
}
