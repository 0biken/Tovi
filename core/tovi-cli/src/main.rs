use tovi_core::init;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!("Starting TOVI Phase 1 Headless Spike (CLI)");

    // Initialize the core transfer/networking engine
    init();

    // Keep alive for testing
    tokio::signal::ctrl_c().await?;
    tracing::info!("Shutting down...");

    Ok(())
}
