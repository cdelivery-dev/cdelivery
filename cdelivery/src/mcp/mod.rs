pub(crate) mod tool;

use std::borrow::Cow;
use std::convert::Infallible;
use std::future::{self, Future};
use std::sync::{Arc, LazyLock};

use axum::extract::Request;
use axum::response::IntoResponse;
use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, CompleteRequestMethod,
    CompleteRequestParams, CompleteResult, ContentBlock, DiscoverResult, ErrorData, Implementation,
    InitializeRequestParams, ListPromptsRequestMethod, ListPromptsResult,
    ListResourceTemplatesRequestMethod, ListResourceTemplatesResult, ListResourcesRequestMethod,
    ListResourcesResult, ListToolsResult, MetaObject, PaginatedRequestParams, ProtocolVersion,
    ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{RoleServer, ServerHandler};
use tokio_util::sync::CancellationToken;

use crate::boot::App;
use crate::concept;
use crate::error::{Error, ErrorKind};
use crate::operation;

const INSTRUCTIONS: &str = "cdelivery deploys apps in groups across Docker Swarm nodes. pingress is \
the ingress proxy; insight stores metrics, logs, access records, events and chat history; xhealth \
runs health checks; xjob runs scheduled jobs; guardian gates risky app updates. Patch tools keep \
omitted fields; null clears a nullable field and is rejected elsewhere; any other value, including \
a list or object, replaces the stored value. Timestamps are RFC 3339. Everyone reads shared data; \
Operators manage apps, health checks, jobs and guarded updates; Admins manage the platform.";

const SUPPORTED_VERSIONS: &[ProtocolVersion] = &[ProtocolVersion::V_2026_07_28];
const CACHE_TTL_MS: u64 = crate::config::MCP_CACHE_TTL.as_secs().unsigned_abs() * 1_000;

pub(crate) fn service(
    state: &Arc<App>,
    cancel: CancellationToken,
) -> impl tower::Service<Request, Response: IntoResponse, Error = Infallible, Future: Send>
+ Clone
+ Send
+ Sync
+ 'static {
    let app = state.clone();
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_cancellation_token(cancel)
        .with_max_request_body_bytes(crate::config::JSON_BODY_LIMIT)
        .with_stateless_protocol_metadata_required(true)
        .disable_allowed_hosts()
        .disable_allowed_origins();
    StreamableHttpService::new(
        move || Ok(Handler { app: app.clone() }),
        Arc::new(NeverSessionManager::default()),
        config,
    )
}

pub(crate) struct Handler {
    app: Arc<App>,
}

impl ServerHandler for Handler {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(server_info())
            .with_instructions(format!(
                "{INSTRUCTIONS}\n\n{}",
                operation::catalog::HISTORY_GUIDANCE
            ))
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(SUPPORTED_VERSIONS)
    }

    fn discover(
        &self,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<DiscoverResult, ErrorData>> {
        future::ready(Ok(DiscoverResult::from_server_info(
            SUPPORTED_VERSIONS.to_vec(),
            self.get_info(),
        )
        .with_ttl_ms(CACHE_TTL_MS)))
    }

    fn initialize(
        &self,
        request: InitializeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ServerConfig, ErrorData>> {
        future::ready(Err(ErrorData::unsupported_protocol_version(
            request.protocol_version,
            SUPPORTED_VERSIONS,
        )))
    }

    fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> {
        if request.and_then(|request| request.cursor).is_some() {
            return future::ready(Err(ErrorData::invalid_params("invalid cursor", None)));
        }
        future::ready(access(&context).map(|access| {
            let mut result = ListToolsResult::with_all_items(tool::definitions(&access))
                .with_ttl_ms(CACHE_TTL_MS)
                .with_cache_scope(CacheScope::Private);
            result.meta = Some(SERVER_INFO_META.clone());
            result
        }))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let access = access(&context)?;
        let Some(spec) = operation::catalog::find(&request.name) else {
            return Err(ErrorData::invalid_params(
                format!("Unknown tool: {}", request.name),
                None,
            ));
        };
        let mut result = if spec.allows(&access) {
            if request.input_responses.is_some() || request.request_state.is_some() {
                return Err(ErrorData::invalid_params(
                    "tool call continuations are not supported",
                    None,
                ));
            }
            let raw = serde_json::Value::Object(request.arguments.unwrap_or_default());
            match (spec.run)(&self.app, &access, raw, jiff::Timestamp::now()).await {
                Ok(payload) => tool::encode(payload),
                Err(err) => error_result(&err),
            }
        } else {
            error_result(&Error::new(ErrorKind::Forbidden))
        };
        result.meta = Some(SERVER_INFO_META.clone());
        Ok(result.into())
    }

    fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListPromptsResult, ErrorData>> {
        future::ready(Err(
            ErrorData::method_not_found::<ListPromptsRequestMethod>(),
        ))
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, ErrorData>> {
        future::ready(Err(
            ErrorData::method_not_found::<ListResourcesRequestMethod>(),
        ))
    }

    fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourceTemplatesResult, ErrorData>> {
        future::ready(Err(ErrorData::method_not_found::<
            ListResourceTemplatesRequestMethod,
        >()))
    }

    fn complete(
        &self,
        _request: CompleteRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CompleteResult, ErrorData>> {
        future::ready(Err(ErrorData::method_not_found::<CompleteRequestMethod>()))
    }
}

fn server_info() -> Implementation {
    Implementation::new("cdelivery", env!("CARGO_PKG_VERSION"))
}

static SERVER_INFO_META: LazyLock<MetaObject> = LazyLock::new(|| {
    let mut meta = MetaObject::default();
    meta.insert(
        "io.modelcontextprotocol/serverInfo".to_owned(),
        serde_json::json!(server_info()),
    );
    meta
});

fn access(context: &RequestContext<RoleServer>) -> Result<concept::permission::Access, ErrorData> {
    let access = context
        .extensions
        .get::<http::request::Parts>()
        .and_then(|parts| parts.extensions.get::<concept::permission::Access>())
        .cloned();
    let Some(access) = access else {
        tracing::error!("mcp authenticated access invariant failed");
        return Err(ErrorData::internal_error("internal server error", None));
    };
    Ok(access)
}

fn error_result(err: &Error) -> CallToolResult {
    if err.status().is_server_error() {
        tracing::error!(error = ?err, "mcp tool failed");
    }
    CallToolResult::error(vec![ContentBlock::text(
        operation::problem_json(err).to_string(),
    )])
}
