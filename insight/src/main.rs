#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> insight::error::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("healthz") {
        return insight::healthz().await;
    }
    logging::init()?;
    tracing::info!("starting insight server");
    insight::run().await
}
