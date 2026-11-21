mod error;
mod local;
mod neon;
mod planetscale;
mod sql;

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{
    Router,
    routing::{get, post},
};
use cdelivery_wire::postgres::{self, Command, Target};
use tracing_subscriber::util::SubscriberInitExt as _;
use zeroize::Zeroizing;

use crate::error::{ErrorKind, Result};

#[derive(Debug)]
struct Executor {
    cipher: cipher::Cipher,
    client: reqwest::Client,
    preview: tokio::sync::Mutex<BTreeMap<String, cipher::Secret>>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .finish()
        .try_init()?;
    let key = Zeroizing::new(std::fs::read(postgres::KEY_PATH)?);
    let key: &[u8; 32] = key.as_slice().try_into()?;
    let executor = Arc::new(Executor {
        cipher: cipher::Cipher::from_key(key),
        client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()?,
        preview: tokio::sync::Mutex::new(BTreeMap::new()),
    });
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", postgres::PORT)).await?;
    axum::serve(
        listener,
        Router::new()
            .route("/", post(handle))
            .route("/health", get(|| async { StatusCode::NO_CONTENT }))
            .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
            .with_state(executor),
    )
    .with_graceful_shutdown(logging::shutdown_signal())
    .await?;
    Ok(())
}

async fn handle(State(executor): State<Arc<Executor>>, body: String) -> Response {
    let Ok(command) = executor.cipher.open::<Command>(&body) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let task_executor = executor.clone();
    let result = tokio::spawn(async move { run(&task_executor, command).await }).await;
    let result = match result {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => postgres::Response::Failed {
            message: error.to_string(),
        },
        Err(error) => postgres::Response::Failed {
            message: error.to_string(),
        },
    };
    match executor.cipher.seal(&result) {
        Ok(body) => body.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn run(executor: &Executor, command: Command) -> Result<postgres::Response> {
    let mut preview = executor
        .preview
        .try_lock()
        .map_err(|_| ErrorKind::Invalid("executor is busy".to_owned()))?;
    let dependency = match &command {
        Command::Prepare { dependency, .. }
        | Command::Execute { dependency, .. }
        | Command::Cleanup { dependency, .. } => dependency,
    };
    if !cdelivery_wire::name::shape_ok(dependency) {
        return Err(ErrorKind::Invalid("invalid mounted dependency name".to_owned()).into());
    }
    let bytes = Zeroizing::new(std::fs::read(format!(
        "{}/{dependency}",
        postgres::SECRET_DIRECTORY
    ))?);
    let fields = postgres::parse_fields(&bytes)?;
    match command {
        Command::Prepare {
            dependency,
            name,
            host,
            size,
            planetscale_sizes,
        } => {
            valid_name(&name)?;
            if preview.contains_key(&dependency) {
                return Err(
                    ErrorKind::Invalid("dependency was already prepared".to_owned()).into(),
                );
            }
            let dsn = match field(&fields, "provider")? {
                "Local" => local::prepare(&fields, host.as_deref())?,
                "Neon" => neon::prepare(&executor.client, &fields, &name, size.as_deref()).await?,
                "PlanetScale" => {
                    planetscale::prepare(
                        &executor.client,
                        &fields,
                        &name,
                        size.as_deref(),
                        planetscale_sizes.as_deref(),
                    )
                    .await?
                }
                _ => {
                    return Err(
                        ErrorKind::Invalid("unsupported dependency provider".to_owned()).into(),
                    );
                }
            };
            preview.insert(dependency, dsn.clone());
            Ok(postgres::Response::Prepared { dsn })
        }
        Command::Execute {
            dependency,
            target,
            sql,
        } => {
            let dsn = match target {
                Target::Production => cipher::Secret::new(field(&fields, "dsn")?.to_owned()),
                Target::Preview => preview.get(&dependency).cloned().ok_or_else(|| {
                    ErrorKind::Invalid(
                        "preview connection is unavailable; discard this rehearsal".to_owned(),
                    )
                })?,
            };
            tokio::task::spawn_blocking(move || {
                sql::execute(
                    dsn.reveal(),
                    sql.reveal(),
                    matches!(target, Target::Preview),
                )
            })
            .await??;
            Ok(postgres::Response::Done)
        }
        Command::Cleanup { dependency, name } => {
            valid_name(&name)?;
            match field(&fields, "provider")? {
                "Local" => {}
                "Neon" => {
                    neon::cleanup(&executor.client, &fields, &name).await?;
                }
                "PlanetScale" => planetscale::cleanup(&executor.client, &fields, &name).await?,
                _ => {
                    return Err(
                        ErrorKind::Invalid("unsupported dependency provider".to_owned()).into(),
                    );
                }
            }
            preview.remove(&dependency);
            Ok(postgres::Response::Done)
        }
    }
}

fn field<'a>(fields: &'a BTreeMap<String, String>, name: &str) -> Result<&'a str> {
    fields
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| ErrorKind::Invalid(format!("dependency field {name} is missing")).into())
}

fn valid_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 63
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || name.starts_with('-')
        || name.ends_with('-')
    {
        return Err(ErrorKind::Invalid("invalid rehearsal branch name".to_owned()).into());
    }
    Ok(())
}

async fn json(request: reqwest::RequestBuilder) -> Result<serde_json::Value> {
    let response = request.send().await?;
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        return Err(ErrorKind::Provider {
            status: status.as_u16(),
            message: text,
        }
        .into());
    }
    if text.is_empty() {
        return Ok(serde_json::Value::Null);
    }
    Ok(serde_json::from_str(&text)?)
}

fn string<'a>(value: &'a serde_json::Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            ErrorKind::Invalid(format!("provider response field {key} is missing")).into()
        })
}

fn member<'a>(value: &'a serde_json::Value, key: &str) -> Result<&'a serde_json::Value> {
    value.get(key).ok_or_else(|| {
        ErrorKind::Invalid(format!("provider response field {key} is missing")).into()
    })
}

fn endpoint(base: &str, segments: &[&str]) -> Result<url::Url> {
    let mut url = url::Url::parse(base)?;
    url.path_segments_mut()
        .map_err(|()| ErrorKind::Invalid("invalid provider base URL".to_owned()))?
        .pop_if_empty()
        .extend(segments);
    Ok(url)
}
