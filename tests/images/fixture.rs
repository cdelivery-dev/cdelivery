use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use axum::{Json, Router, routing};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::task::JoinSet;

use crate::support::{Context, Result};

pub(super) const PASSWORD: &str = "image-preflight-test-password";
const REGISTRY: &str = "host.orb.internal:19502";
const OTHER_REGISTRY: &str = "host.orb.internal:19503";

#[derive(Clone, Copy)]
pub(super) enum Mode {
    Private,
    Anonymous,
    Unavailable,
    Slow,
}

struct Access {
    mode: Mode,
    tokens: BTreeMap<String, String>,
    calls: Vec<(String, String)>,
    requests: usize,
}

#[derive(Clone)]
struct Server {
    repository: String,
    tag: String,
    client: reqwest::Client,
    access: Arc<Mutex<Access>>,
}

pub(super) struct Fixture {
    server: Server,
    tasks: JoinSet<()>,
}

impl Fixture {
    #[expect(clippy::print_stderr)]
    pub(super) async fn start(context: &Context) -> Result<Self> {
        let reference = &context.environment.images["fixture"].reference;
        let (_, reference) = reference.split_once('/').ok_or_else(|| {
            crate::support::ErrorKind::Check("fixture image has no repository".into())
        })?;
        let (repository, tag) = reference
            .rsplit_once(':')
            .ok_or_else(|| crate::support::ErrorKind::Check("fixture image has no tag".into()))?;
        let server = Server {
            repository: repository.to_owned(),
            tag: tag.to_owned(),
            client: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(15))
                .build()?,
            access: Arc::new(Mutex::new(Access {
                mode: Mode::Private,
                tokens: BTreeMap::new(),
                calls: Vec::new(),
                requests: 0,
            })),
        };
        let mut fixture = Self {
            server,
            tasks: JoinSet::new(),
        };
        for port in [19502, 19503] {
            let listener = TcpListener::bind(("0.0.0.0", port)).await?;
            let router = Router::new()
                .route("/token", routing::get(token))
                .fallback(proxy)
                .with_state(fixture.server.clone());
            fixture.tasks.spawn(async move {
                if let Err(error) = axum::serve(listener, router).await {
                    eprintln!("image registry fixture failed: {error:?}");
                }
            });
        }
        Ok(fixture)
    }

    pub(super) fn image(&self, tag: &str, other: bool) -> String {
        let registry = if other { OTHER_REGISTRY } else { REGISTRY };
        format!("{registry}/{}:{tag}", self.server.repository)
    }

    pub(super) async fn mode(&self, mode: Mode) {
        let mut access = self.server.access.lock().await;
        access.mode = mode;
        access.tokens.clear();
        access.calls.clear();
        access.requests = 0;
    }

    pub(super) async fn requests(&self) -> usize {
        self.server.access.lock().await.requests
    }

    pub(super) async fn calls(&self) -> Vec<(String, String)> {
        self.server.access.lock().await.calls.clone()
    }

    pub(super) async fn stop(mut self) -> Result<()> {
        self.tasks.shutdown().await;
        for port in [19502, 19503] {
            TcpListener::bind(("0.0.0.0", port)).await?;
        }
        Ok(())
    }
}

async fn token(
    State(server): State<Server>,
    Query(query): Query<BTreeMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let credentials = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Basic "))
        .and_then(|value| STANDARD.decode(value).ok())
        .and_then(|value| String::from_utf8(value).ok());
    let (user, password) = credentials
        .as_deref()
        .and_then(|value| value.split_once(':'))
        .unwrap_or(("anonymous", ""));
    let scope = query.get("scope").cloned().unwrap_or_default();
    let mut access = server.access.lock().await;
    access.calls.push((user.to_owned(), scope.clone()));
    let authenticated = match access.mode {
        Mode::Private => matches!(user, "reader" | "denied") && password == PASSWORD,
        Mode::Anonymous => credentials.is_none(),
        Mode::Unavailable | Mode::Slow => false,
    };
    if !authenticated {
        return (
            StatusCode::UNAUTHORIZED,
            Json(
                json!({"errors":[{"code":"UNAUTHORIZED","message":"probe credentials rejected"}]}),
            ),
        )
            .into_response();
    }
    let token = uuid::Uuid::new_v4().to_string();
    if user != "denied"
        && (scope.is_empty() || scope == format!("repository:{}:pull", server.repository))
    {
        access.tokens.insert(token.clone(), scope);
    }
    Json(json!({"token":token,"expires_in":60})).into_response()
}

async fn proxy(State(server): State<Server>, request: axum::extract::Request) -> Response {
    let (request, _) = request.into_parts();
    let path = request.uri.path();
    let repository = path
        .strip_prefix("/v2/")
        .and_then(|path| {
            path.split_once("/manifests/")
                .or_else(|| path.split_once("/blobs/"))
        })
        .map_or("", |(repository, _)| repository);
    let scope = format!("repository:{repository}:pull");
    let token = request
        .headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let mut access = server.access.lock().await;
    access.requests += 1;
    let mode = access.mode;
    let allowed = token
        .and_then(|token| access.tokens.get(token))
        .is_some_and(|granted| repository.is_empty() || granted == &scope);
    drop(access);
    if matches!(mode, Mode::Slow) {
        tokio::time::sleep(Duration::from_secs(12)).await;
    }
    if matches!(mode, Mode::Unavailable | Mode::Slow) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "probe registry unavailable",
        )
            .into_response();
    }
    if !allowed {
        let scope = if repository.is_empty() {
            String::new()
        } else {
            format!(",scope=\"{scope}\"")
        };
        let challenge =
            format!("Bearer realm=\"http://{REGISTRY}/token\",service=\"cdelivery-tests\"{scope}");
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, challenge)],
            Json(json!({"errors":[{"code":"UNAUTHORIZED","message":"probe pull permission required"}]})),
        )
            .into_response();
    }
    if path == "/v2/" {
        return Json(json!({})).into_response();
    }
    let path = match path.rsplit_once("/manifests/") {
        Some((prefix, "exists" | "next" | "public")) => {
            format!("{prefix}/manifests/{}", server.tag)
        }
        _ => path.to_owned(),
    };
    let response = async {
        let upstream = server
            .client
            .request(
                request.method.clone(),
                format!("http://localhost:19500{path}"),
            )
            .header(
                header::ACCEPT,
                request
                    .headers
                    .get(header::ACCEPT)
                    .cloned()
                    .unwrap_or_else(|| axum::http::HeaderValue::from_static("*/*")),
            )
            .send()
            .await?;
        let status = upstream.status();
        let headers = upstream.headers().clone();
        let body = upstream.bytes().await?;
        Ok::<_, reqwest::Error>((status, headers, body).into_response())
    }
    .await;
    match response {
        Ok(response) => response,
        Err(error) => (StatusCode::BAD_GATEWAY, error.to_string()).into_response(),
    }
}
