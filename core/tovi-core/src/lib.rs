pub mod discovery;
pub mod identity;
pub mod pairing;
pub mod protocol;
pub mod storage;
pub mod transfer;
pub mod transport;
pub mod trust;

pub fn init() {
    tracing::info!("Initializing tovi-core engine...");
}
