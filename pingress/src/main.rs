mod access;
mod cert;
mod challenge;
mod config;
mod error;
mod fetch;
mod proxy;
mod response;
mod route;
mod sentry;

use std::time::Duration;

use pingora::listeners::TcpSocketOptions;
use pingora::listeners::tls::TlsSettings;
use pingora::protocols::TcpKeepalive;
use pingora::proxy::http_proxy_service;
use pingora::server::Server;
use pingora::server::configuration::ServerConf;
use pingora::services::ServiceWithDependents;
use pingora::services::background::background_service;

#[expect(clippy::exit)]
fn healthz() -> ! {
    let addr = std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 80));
    match std::net::TcpStream::connect_timeout(&addr, config::HEALTHZ_CONNECT_TIMEOUT) {
        Ok(_) => std::process::exit(0),
        Err(_) => std::process::exit(1),
    }
}

fn dual_stack() -> bool {
    std::net::TcpListener::bind((std::net::Ipv6Addr::UNSPECIFIED, 0)).is_ok()
}

fn boot(client: &fetch::Client) -> error::Result<String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    Ok(runtime.block_on(async {
        let snapshot = sentry::boot_fetch(client).await;
        let etag = snapshot.etag.clone();
        sentry::apply(snapshot).await;
        etag
    }))
}

fn main() -> error::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("healthz") {
        healthz();
    }
    logging::init()?;

    let endpoint = config::endpoint();
    let http = config::http_client()?;
    let client = fetch::Client::new(http.clone(), &endpoint)?;
    let boot_etag = boot(&client)?;

    let mut tcp_options = TcpSocketOptions::default();
    tcp_options.tcp_fastopen = Some(10);
    tcp_options.tcp_keepalive = Some(TcpKeepalive {
        idle: Duration::from_mins(1),
        interval: Duration::from_secs(5),
        count: 5,
        #[cfg(target_os = "linux")]
        user_timeout: Duration::ZERO,
    });
    let (plain_addr, tls_addr) = if dual_stack() {
        tcp_options.ipv6_only = Some(false);
        (config::HTTP_ADDR, config::HTTPS_ADDR)
    } else {
        tracing::warn!("ipv6 unavailable, listening on ipv4 only");
        (config::HTTP_ADDR_V4, config::HTTPS_ADDR_V4)
    };

    let threads = std::thread::available_parallelism().map_or_else(
        |error| {
            tracing::warn!(error = ?error, "failed to detect CPU parallelism, using 1 worker");
            1
        },
        std::num::NonZeroUsize::get,
    );
    tracing::info!("starting proxy with {threads} workers");
    let mut server = Server::new_with_opt_and_conf(
        None,
        ServerConf {
            threads,
            grace_period_seconds: Some(20),
            graceful_shutdown_timeout_seconds: Some(5),
            ..ServerConf::default()
        },
    );
    server.bootstrap();
    let proxy = proxy::Proxy {
        challenge: challenge::Challenge::new(http, &endpoint),
    };
    let mut proxy_service = http_proxy_service(&server.configuration, proxy);
    let mut tls_settings = TlsSettings::with_callbacks(Box::new(cert::DynamicCert))?;
    tls_settings.enable_h2();
    proxy_service.add_tcp_with_settings(plain_addr, tcp_options.clone());
    proxy_service.add_tls_with_settings(tls_addr, Some(tcp_options), tls_settings);
    let poller = background_service("sentry", sentry::Sentry::new(client, boot_etag));

    let services: Vec<Box<dyn ServiceWithDependents>> =
        vec![Box::new(proxy_service), Box::new(poller)];
    server.add_services(services);
    server.run_forever()
}
