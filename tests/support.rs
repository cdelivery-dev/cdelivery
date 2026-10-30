use std::future::Future;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use reqwest::{Client, Method, RequestBuilder, Response};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::environment::{Credentials, Environment};

#[path = "process.rs"]
mod process;

#[derive(Debug, thiserror::Error)]
pub(crate) enum ErrorKind {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("{0}: {1}")]
    ProbeJson(String, #[source] serde_json::Error),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Process(#[from] process::Error),
    #[error(transparent)]
    Certificate(#[from] rcgen::Error),
    #[error(transparent)]
    Base64(#[from] base64::DecodeError),
    #[error(transparent)]
    Timeout(#[from] tokio::time::error::Elapsed),
    #[error("{1}")]
    Status(u16, String),
    #[error("{0}")]
    Api(String),
    #[error("{0}")]
    Check(String),
}

macros::located_error!(Error => ErrorKind, from {
    std::io::Error => Io, serde_json::Error => Json, reqwest::Error => Http,
    process::Error => Process, rcgen::Error => Certificate,
    base64::DecodeError => Base64,
});

pub(crate) struct Context {
    pub(crate) environment: Environment,
    pub(crate) credentials: Credentials,
    pub(crate) client: Client,
    pub(crate) root: PathBuf,
}

impl Context {
    fn read(artifacts: &Path) -> Result<Self> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/tests/current");
        ensure(
            root.join("env.json").is_file(),
            "no current environment; run cargo xtask env up single or cluster",
        )?;
        let environment: Environment =
            serde_json::from_slice(&std::fs::read(root.join("env.json"))?)?;
        ensure(
            environment.ready && matches!(environment.profile.as_str(), "single" | "cluster"),
            "run cargo xtask env up single or cluster before this target",
        )?;
        let credentials = serde_json::from_slice(&std::fs::read(root.join("credentials.json"))?)?;
        ensure(
            !environment.machines.is_empty(),
            "environment has no manager",
        )?;
        for component in [
            "cdelivery",
            "cdelivery-agent",
            "insight",
            "insight-agent",
            "pingress",
            "hurl",
            "fixture",
        ] {
            ensure(
                environment.images.contains_key(component)
                    && environment.versions.contains_key(component),
                &format!("environment is missing {component}"),
            )?;
        }
        Ok(Self {
            environment,
            credentials,
            client: client()?,
            root: artifacts.to_owned(),
        })
    }

    pub(crate) fn request(&self, method: &str, path: &str) -> Result<reqwest::RequestBuilder> {
        let method = method
            .parse::<Method>()
            .map_err(|error| ErrorKind::Check(error.to_string()))?;
        Ok(self
            .client
            .request(method, format!("{}{path}", self.environment.base_url))
            .bearer_auth(&self.credentials.api_key))
    }

    pub(crate) async fn api(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
        let mut request = self.request(method, path)?;
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await?;
        let code = response.status();
        if !code.is_success() {
            let body = diagnostic(response)
                .await
                .unwrap_or_else(|error| format!("body read failed: {error:?}"));
            let message = format!("{method} {path}: {code}: {body}");
            return Err(if matches!(code.as_u16(), 502..=504) {
                ErrorKind::Status(code.as_u16(), message)
            } else {
                ErrorKind::Api(message)
            }
            .into());
        }
        let bytes = response.bytes().await?;
        if bytes.is_empty() {
            return Ok(Value::Null);
        }
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub(crate) async fn login(&self, email: &str, password: &str) -> Result<Client> {
        let client = client()?;
        let request = client
            .post(format!("{}/login", self.environment.base_url))
            .header("Origin", &self.environment.base_url)
            .form(&[(
                "payload",
                json!({"email":email,"password_sha256":hash(password),"totp_code":null})
                    .to_string(),
            )]);
        send(request, 303).await?;
        Ok(client)
    }

    pub(crate) fn docker(&self, args: &[&str]) -> Result<String> {
        self.machine(&self.environment.machines[0], args)
    }

    pub(crate) fn machine(&self, name: &str, args: &[&str]) -> Result<String> {
        Ok(process::run(
            self.command(name)?.arg("/usr/bin/docker").args(args),
            b"",
            Duration::from_secs(180),
            None,
        )?)
    }

    fn command(&self, name: &str) -> Result<Command> {
        ensure(
            self.environment.machines.iter().any(|owned| owned == name),
            "machine must belong to this environment",
        )?;
        let mut command = Command::new("orb");
        command.args(["-m", name, "-u", "root"]);
        Ok(command)
    }

    #[expect(clippy::print_stderr)]
    pub(crate) fn curl(&self, machine: &str, args: &[&str]) -> Result<(String, Value)> {
        let mut command = self.command(machine)?;
        command
            .args([
                "/usr/bin/curl",
                "--disable",
                "--silent",
                "--show-error",
                "--noproxy",
                "*",
                "--max-time",
                "10",
                "--write-out",
                "\n%{json}",
            ])
            .args(args);
        let path = self.root.join(format!("curl-{machine}.txt"));
        let mut file = std::fs::File::create(&path)?;
        writeln!(file, "machine: {machine}\ncommand: {command:?}")?;
        let output = match process::output(&mut command, b"", Duration::from_secs(20), None) {
            Ok(output) => output,
            Err(error) => {
                eprintln!("{machine}: curl command failed; see {}", path.display());
                if let Err(write_error) = writeln!(file, "process error: {error:?}") {
                    eprintln!(
                        "recording probe failure in {} failed: {write_error:?}",
                        path.display()
                    );
                }
                return Err(error.into());
            }
        };
        writeln!(
            file,
            "status: {}\nstdout: {} bytes",
            output.status,
            output.stdout.len()
        )?;
        file.write_all(&output.stdout)?;
        writeln!(file, "\nstderr: {} bytes", output.stderr.len())?;
        file.write_all(&output.stderr)?;
        let diagnostic = || {
            format!(
                "{machine}: curl {}; see {}; stderr: {}",
                output.status,
                path.display(),
                String::from_utf8_lossy(&output.stderr)
                    .chars()
                    .take(4096)
                    .collect::<String>(),
            )
        };
        let invalid = |message| ErrorKind::Api(format!("{message}; {}", diagnostic()));
        let newline = output
            .stdout
            .iter()
            .rposition(|byte| *byte == b'\n')
            .ok_or_else(|| invalid("curl did not return JSON metadata"))?;
        let (body, metadata) = output.stdout.split_at(newline);
        let metadata: Value = serde_json::from_slice(metadata)
            .map_err(|error| ErrorKind::ProbeJson(diagnostic(), error))?;
        let code = metadata["exitcode"]
            .as_u64()
            .ok_or_else(|| invalid("curl metadata is missing exitcode"))?;
        if output
            .status
            .code()
            .and_then(|code| u64::try_from(code).ok())
            != Some(code)
        {
            return Err(invalid("curl process status disagrees with JSON exitcode").into());
        }
        if !metadata["http_code"].is_u64() || !metadata["http_version"].is_string() {
            return Err(invalid("curl metadata is missing HTTP results").into());
        }
        match code {
            0 | 35 => Ok((String::from_utf8_lossy(body).into_owned(), metadata)),
            7 | 28 | 52 | 55 | 56 | 60 => {
                Err(ErrorKind::Check(format!("{}; curl metadata: {metadata}", diagnostic())).into())
            }
            _ => Err(ErrorKind::Api(format!("{}; curl metadata: {metadata}", diagnostic())).into()),
        }
    }

    pub(crate) fn control_machine(&self, action: &str, name: &str) -> Result<()> {
        ensure(
            self.environment.machines.iter().any(|owned| owned == name),
            "machine must belong to this environment",
        )?;
        process::run(
            Command::new("orbctl").args([action, name]),
            b"",
            Duration::from_secs(180),
            None,
        )?;
        Ok(())
    }

    pub(crate) async fn app(&self, name: &str, body: &str) -> Result<Value> {
        self.api("POST", "/api/apps", Some(json!({
            "name":name,"group":"tests","image":self.environment.images["fixture"],"replicas":{"Fixed":1},
            "container_options":{"command":["sh","-c",format!("mkdir -p /www; printf '%s' '{body}' > /www/index.html; echo '{body}'; exec httpd -f -p 8080 -h /www")]}
        }))).await
    }

    pub(crate) async fn ready(&self, name: &str, replicas: u64) -> Result<()> {
        eventually("app replicas", || async {
            let value = self
                .api("GET", &format!("/api/apps/{name}/status"), None)
                .await?;
            ensure(
                value["instances"]["running"].as_u64() == Some(replicas),
                &format!("{name} status: {value}"),
            )
        })
        .await
    }

    pub(crate) fn diagnostics(&self, name: &str) -> Result<()> {
        let directory = self.root.join(name);
        std::fs::create_dir_all(&directory)?;
        for (file, args) in [
            ("nodes", vec!["node", "ls"]),
            ("services", vec!["service", "ls"]),
            (
                "cdelivery",
                vec!["service", "logs", "--tail", "200", "cdelivery"],
            ),
            (
                "insight",
                vec!["service", "logs", "--tail", "200", "cdelivery-insight"],
            ),
            (
                "pingress",
                vec!["service", "logs", "--tail", "200", "cdelivery-pingress"],
            ),
        ] {
            let path = directory.join(format!("{file}.log"));
            let output = match process::run(
                Command::new("orb")
                    .args([
                        "-m",
                        &self.environment.machines[0],
                        "-u",
                        "root",
                        "/usr/bin/docker",
                    ])
                    .args(&args),
                b"",
                Duration::from_secs(30),
                Some(&path),
            ) {
                Ok(_) => std::fs::read_to_string(&path)?,
                Err(error) => format!("{}\n{error:?}", std::fs::read_to_string(&path)?),
            };
            std::fs::write(
                path,
                output
                    .replace(&self.credentials.api_key, "[redacted]")
                    .replace(&self.credentials.password, "[redacted]"),
            )?;
        }
        Ok(())
    }
}

#[expect(clippy::print_stderr)]
pub(crate) async fn run(name: &str, test: impl AsyncFnOnce(&Context) -> Result<()>) -> Result<()> {
    let state = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/tests");
    let root = state
        .join("results")
        .join(format!("{name}-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&root)?;
    std::fs::set_permissions(
        state.join("results"),
        std::fs::Permissions::from_mode(0o700),
    )?;
    let started = Instant::now();
    let result = match Context::read(&root) {
        Err(error) => Err(error),
        Ok(context) => {
            std::fs::copy(state.join("current/env.json"), root.join("env.json"))?;
            let mut result = match tokio::time::timeout(Duration::from_mins(20), async {
                context.api("GET", "/api/me", None).await?;
                test(&context).await
            })
            .await
            {
                Ok(result) => result,
                Err(source) => Err(ErrorKind::Timeout(source).into()),
            };
            if result.is_err()
                && let Err(error) = context.diagnostics("failure")
            {
                eprintln!("collecting diagnostics failed: {error:?}");
            }
            if name == "recovery" {
                for worker in context.environment.machines.iter().skip(1) {
                    if let Err(error) = context.control_machine("start", worker) {
                        eprintln!("restoring worker failed: {error:?}");
                        if let Err(write_error) = std::fs::write(
                            root.join(format!("restore-{worker}.txt")),
                            format!("{error:?}"),
                        ) {
                            eprintln!("recording restoration failure failed: {write_error:?}");
                        }
                        if result.is_ok() {
                            result = Err(error);
                        }
                    }
                }
            }
            result
        }
    };
    let report = match &result {
        Ok(()) => "PASS".to_owned(),
        Err(error) => redact(format!("FAIL {error:?}")),
    };
    if let Err(error) = std::fs::write(
        root.join("result.txt"),
        format!("{report}\nelapsed {:?}\n", started.elapsed()),
    ) {
        eprintln!("recording test result failed: {error:?}");
        if result.is_ok() {
            return Err(error.into());
        }
    }
    result
}

pub(crate) fn client() -> Result<Client> {
    Ok(Client::builder()
        .no_proxy()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .timeout(Duration::from_secs(60))
        .build()?)
}

#[track_caller]
pub(crate) fn ensure(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(ErrorKind::Check(message.into()).into())
    }
}

pub(crate) async fn send(request: RequestBuilder, expected: u16) -> Result<Response> {
    let response = request.send().await?;
    let code = response.status();
    if code.as_u16() != expected {
        let url = response.url().to_string();
        let body = diagnostic(response)
            .await
            .unwrap_or_else(|error| format!("body read failed: {error:?}"));
        return Err(ErrorKind::Status(
            code.as_u16(),
            format!("expected HTTP {expected}, got {code} for {url}: {body}"),
        )
        .into());
    }
    Ok(response)
}

fn redact(mut value: String) -> String {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/tests/current/credentials.json");
    if let Ok(bytes) = std::fs::read(path)
        && let Ok(credentials) = serde_json::from_slice::<Credentials>(&bytes)
    {
        for secret in [&credentials.api_key, &credentials.password] {
            if !secret.is_empty() {
                value = value.replace(secret, "[redacted]");
            }
        }
    }
    value
}

async fn diagnostic(mut response: Response) -> Result<String> {
    let mut bytes = Vec::new();
    while bytes.len() < 8192 {
        let Some(chunk) = response.chunk().await? else {
            break;
        };
        bytes.extend(chunk.iter().take(8192 - bytes.len()));
    }
    let body = if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
        let fields: serde_json::Map<_, _> = ["code", "message", "detail", "error", "type"]
            .into_iter()
            .filter_map(|name| {
                value
                    .get(name)
                    .map(|value| (name.to_owned(), value.clone()))
            })
            .collect();
        Value::Object(fields).to_string()
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    if [
        "private key",
        "password",
        "secret_access_key",
        "authorization",
        "bearer ",
    ]
    .iter()
    .any(|word| body.to_ascii_lowercase().contains(word))
    {
        return Ok("[sensitive response body redacted]".into());
    }
    Ok(redact(body).chars().take(2048).collect())
}

pub(crate) fn string<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value[field]
        .as_str()
        .ok_or_else(|| ErrorKind::Check(format!("missing string field {field}")).into())
}

pub(crate) fn hash(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .filter_map(|nibble| char::from_digit(nibble.into(), 16))
        .collect()
}

pub(crate) fn name(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        uuid::Uuid::new_v4()
            .simple()
            .to_string()
            .chars()
            .take(6)
            .collect::<String>()
    )
}

pub(crate) async fn eventually<T, F: Future<Output = Result<T>>>(
    description: &str,
    mut check: impl FnMut() -> F,
) -> Result<T> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    let mut pending = String::from("probe did not complete");
    loop {
        match tokio::time::timeout_at(deadline, check()).await {
            Ok(Ok(value)) => return Ok(value),
            Err(_) => {
                return Err(ErrorKind::Check(format!(
                    "timed out waiting for {description}: {pending}"
                ))
                .into());
            }
            Ok(Err(error)) => {
                let retry = match &error.kind {
                    ErrorKind::Check(_) | ErrorKind::Status(200 | 404 | 502..=504, _) => true,
                    ErrorKind::Http(error) => {
                        error.is_connect() || error.is_timeout() || error.is_body()
                    }
                    _ => false,
                };
                if !retry {
                    return Err(error);
                }
                pending = format!("{error:?}");
                tokio::time::sleep_until(
                    deadline.min(tokio::time::Instant::now() + Duration::from_secs(2)),
                )
                .await;
            }
        }
    }
}
