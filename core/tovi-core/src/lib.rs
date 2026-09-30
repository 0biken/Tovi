pub mod discovery;
pub mod identity;
pub mod protocol;
pub mod transfer;
pub mod transport;

pub fn init() {
    tracing::info!("Initializing tovi-core engine...");
}
