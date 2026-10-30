use std::borrow::Cow;

use async_trait::async_trait;
use pingora::http::{RequestHeader, ResponseHeader};
use pingora::proxy::{FailToProxy, ProxyHttp, Session};
use pingora::upstreams::peer::HttpPeer;
use pingora::{ErrorSource, ErrorType, Result};

use crate::access;
use crate::cert;
use crate::challenge;
use crate::response;
use crate::route;

const HSTS_ENABLED: &str = "max-age=15768000";
const HSTS_REVOKED: &str = "max-age=0";

fn hsts_header(https_enabled: bool, upstream_has_sts: bool) -> Option<&'static str> {
    match (https_enabled, upstream_has_sts) {
        (true, true) => None,
        (true, false) => Some(HSTS_ENABLED),
        (false, _) => Some(HSTS_REVOKED),
    }
}

fn parse_host(session: &Session) -> Option<String> {
    let raw = session.req_header().uri.host().or_else(|| {
        session
            .get_header(http::header::HOST)
            .and_then(|h| h.to_str().ok())
    })?;
    Some(access::Host::parse(raw)?.into_string())
}

fn is_plaintext(session: &Session) -> bool {
    session.digest().is_none_or(|d| d.ssl_digest.is_none())
}

fn header_matches(session: &Session, key: &str, expected: Option<&str>) -> bool {
    session
        .get_header(key)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|value| value == expected.unwrap_or(value))
}

fn cookie_matches(session: &Session, key: &str, expected: Option<&str>) -> bool {
    session
        .get_header(http::header::COOKIE)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|cookies| {
            cookies
                .split(';')
                .map(str::trim)
                .find_map(|cookie| {
                    let mut parts = cookie.splitn(2, '=');
                    let name = parts.next()?.trim();
                    let value = parts.next()?.trim();
                    (name == key).then(|| value == expected.unwrap_or(value))
                })
                .unwrap_or(false)
        })
}

fn client_ip(session: &Session) -> Option<String> {
    session
        .client_addr()
        .and_then(|addr| addr.as_inet())
        .map(|inet| inet.ip().to_canonical().to_string())
}

pub(crate) struct ProxyCtx {
    domain: Option<String>,
    unmatched_host: Option<String>,
    https_enabled: bool,
    matched_peer: Option<Box<HttpPeer>>,
    selected: Option<route::Candidate>,
    start: std::time::Instant,
    ttfb: Option<std::time::Duration>,
    outcome: access::Outcome,
}

pub(crate) struct Proxy {
    pub(crate) challenge: challenge::Challenge,
}

#[async_trait]
impl ProxyHttp for Proxy {
    type CTX = ProxyCtx;

    fn new_ctx(&self) -> Self::CTX {
        ProxyCtx {
            domain: None,
            unmatched_host: None,
            https_enabled: false,
            matched_peer: None,
            selected: None,
            start: std::time::Instant::now(),
            ttfb: None,
            outcome: access::Outcome::Aborted,
        }
    }

    async fn request_filter(&self, session: &mut Session, ctx: &mut Self::CTX) -> Result<bool> {
        let path = session.req_header().uri.path();

        if path.starts_with(pingress_wire::HTTP01_PATH_PREFIX) {
            if path == pingress_wire::HTTP01_HEALTHZ_PATH {
                response::text(session, None).await?;
                return Ok(true);
            }
            if let Some(host) = parse_host(session) {
                if route::is_managed(&host) {
                    ctx.domain = Some(host);
                } else {
                    ctx.unmatched_host = Some(host);
                }
            }
            ctx.outcome = access::Outcome::AcmeChallenge;
            let token = path.trim_start_matches(pingress_wire::HTTP01_PATH_PREFIX);
            match self.challenge.lookup(token).await {
                challenge::Lookup::Found(key_authorization) => {
                    response::text(session, Some(key_authorization.into())).await?;
                }
                challenge::Lookup::Absent => response::error(session, 404).await?,
                challenge::Lookup::Unavailable => response::error(session, 503).await?,
            }
            return Ok(true);
        }

        let Some(host) = parse_host(session) else {
            ctx.outcome = access::Outcome::InvalidHost;
            response::error(session, 400).await?;
            return Ok(true);
        };

        let header_val = |k: &str, v: Option<&str>| header_matches(session, k, v);
        let cookie_val = |k: &str, v: Option<&str>| cookie_matches(session, k, v);
        let Some((https_enabled, outcome)) = route::decide(&host, path, header_val, cookie_val)
        else {
            ctx.outcome = access::Outcome::NoRoute;
            ctx.unmatched_host = Some(host);
            response::error(session, 404).await?;
            return Ok(true);
        };
        ctx.domain = Some(host.clone());
        ctx.https_enabled = https_enabled;

        if is_plaintext(session) && https_enabled && cert::has_cert(&host) {
            if let route::RouteOutcome::Peer { candidate, .. } = outcome {
                ctx.selected = Some(candidate);
            }
            let path_query = session
                .req_header()
                .uri
                .path_and_query()
                .map_or("/", |pq| pq.as_str());
            ctx.outcome = access::Outcome::HttpsRedirect;
            let mut resp = ResponseHeader::build(http::StatusCode::PERMANENT_REDIRECT, Some(0))?;
            resp.insert_header("Cache-Control", "no-store")?;
            resp.insert_header("Location", format!("https://{host}{path_query}"))?;
            response::set_server(&mut resp)?;
            session.write_response_header(Box::new(resp), true).await?;
            return Ok(true);
        }

        match outcome {
            route::RouteOutcome::Peer { candidate, peer } => {
                ctx.outcome = access::Outcome::UpstreamFailed;
                ctx.selected = Some(candidate);
                ctx.matched_peer = Some(peer);
                Ok(false)
            }
            route::RouteOutcome::NoUpstream => {
                ctx.outcome = access::Outcome::NoUpstream;
                tracing::debug!("route for {host}{path} matched without resolved upstream");
                response::error(session, 503).await?;
                Ok(true)
            }
            route::RouteOutcome::NoRoute => {
                ctx.outcome = access::Outcome::NoRoute;
                response::error(session, 404).await?;
                Ok(true)
            }
        }
    }

    async fn upstream_peer(
        &self,
        _session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>> {
        ctx.matched_peer.clone().ok_or_else(|| {
            pingora::Error::new(ErrorType::HTTPStatus(
                http::StatusCode::BAD_GATEWAY.as_u16(),
            ))
        })
    }

    async fn upstream_request_filter(
        &self,
        session: &mut Session,
        upstream_request: &mut RequestHeader,
        _ctx: &mut Self::CTX,
    ) -> Result<()> {
        if let Some(client_ip) = client_ip(session) {
            upstream_request.insert_header("X-Real-IP", &client_ip)?;
            upstream_request.insert_header("X-Forwarded-For", client_ip)?;
        } else {
            upstream_request.remove_header("X-Real-IP");
            upstream_request.remove_header("X-Forwarded-For");
        }
        let proto = if is_plaintext(session) {
            "http"
        } else {
            "https"
        };
        upstream_request.insert_header("X-Forwarded-Proto", proto)?;
        if let Some(host) = upstream_request.headers.get(http::header::HOST).cloned() {
            upstream_request.insert_header("X-Forwarded-Host", host)?;
        } else {
            upstream_request.remove_header("X-Forwarded-Host");
        }
        Ok(())
    }

    async fn response_filter(
        &self,
        session: &mut Session,
        upstream_response: &mut ResponseHeader,
        ctx: &mut Self::CTX,
    ) -> Result<()> {
        if !upstream_response.status.is_informational()
            || upstream_response.status == http::StatusCode::SWITCHING_PROTOCOLS
        {
            ctx.outcome = access::Outcome::Proxied;
        }
        if ctx.ttfb.is_none() {
            ctx.ttfb = Some(ctx.start.elapsed());
        }
        if !is_plaintext(session) {
            let upstream_has_sts = upstream_response
                .headers
                .contains_key(http::header::STRICT_TRANSPORT_SECURITY);
            if let Some(hsts) = hsts_header(ctx.https_enabled, upstream_has_sts) {
                upstream_response.insert_header("Strict-Transport-Security", hsts)?;
            }
        }
        response::set_server(upstream_response)
    }

    async fn fail_to_proxy(
        &self,
        session: &mut Session,
        error: &pingora::Error,
        _ctx: &mut Self::CTX,
    ) -> FailToProxy {
        let code = match (error.etype(), error.esource()) {
            (ErrorType::HTTPStatus(code), _) => *code,
            (_, ErrorSource::Upstream) => 502,
            (
                ErrorType::WriteError | ErrorType::ReadError | ErrorType::ConnectionClosed,
                ErrorSource::Downstream,
            ) => 0,
            (_, ErrorSource::Downstream) => 400,
            (_, ErrorSource::Internal | ErrorSource::Unset) => 500,
        };
        if code > 0
            && let Err(err) = response::error(session, code).await
        {
            tracing::error!(error = ?err, "failed to send error response to downstream");
        }
        FailToProxy {
            error_code: code,
            can_reuse_downstream: false,
        }
    }

    async fn logging(
        &self,
        session: &mut Session,
        e: Option<&pingora::Error>,
        ctx: &mut Self::CTX,
    ) {
        let req = session.req_header();
        if req.uri.path() == pingress_wire::HTTP01_HEALTHZ_PATH {
            return;
        }
        access::write_to_stdout(&access::AccessRecord {
            timestamp: jiff::Timestamp::now(),
            domain: ctx.domain.as_deref().map(Cow::Borrowed),
            unmatched_host: ctx.unmatched_host.as_deref().map(Cow::Borrowed),
            client_ip: client_ip(session),
            method: Cow::Borrowed(access::Method::normalize(req.method.as_str()).as_str()),
            path: access::truncated_path(req.uri.path()),
            status: session.response_written().map(|resp| resp.status.as_u16()),
            outcome: ctx.outcome,
            duration_us: access::duration_micros(ctx.start.elapsed()),
            upstream_ttfb_us: ctx.ttfb.map(access::duration_micros),
            request_body_bytes: access::saturating_u64(session.body_bytes_read()),
            response_body_bytes: access::saturating_u64(session.body_bytes_sent()),
            app_name: ctx
                .selected
                .as_ref()
                .map(|candidate| Cow::Borrowed(candidate.app_name.as_str())),
            target_port: ctx.selected.as_ref().map(|candidate| candidate.target),
            service_id: ctx
                .selected
                .as_ref()
                .map(|candidate| Cow::Borrowed(candidate.service_id.as_str())),
            error: e.map(|err| Cow::Borrowed(err.etype().as_str())),
        });
    }
}
