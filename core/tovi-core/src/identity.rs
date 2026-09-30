//! Cryptographic device identity using Ed25519.

use anyhow::Result;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey, KEYPAIR_LENGTH};
use rand_core::OsRng;

/// Represents a device's cryptographic identity.
pub struct DeviceIdentity {
    signing_key: SigningKey,
}

impl DeviceIdentity {
    /// Generate a new, secure random device identity
    pub fn generate_new() -> Self {
        let mut csprng = OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        Self { signing_key }
    }

    /// Reconstruct identity from stored bytes (e.g. from Keychain / Credential Manager)
    pub fn from_bytes(bytes: &[u8; KEYPAIR_LENGTH]) -> Result<Self> {
        let signing_key = SigningKey::from_keypair_bytes(bytes)?;
        Ok(Self { signing_key })
    }

    /// Get the public verifying key (used as the TOVI device ID)
    pub fn public_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    /// Serialize the identity to save in OS secure storage
    pub fn to_bytes(&self) -> [u8; KEYPAIR_LENGTH] {
        self.signing_key.to_keypair_bytes()
    }

    /// Sign a payload (used to prove identity during the connection handshake)
    pub fn sign(&self, message: &[u8]) -> Signature {
        self.signing_key.sign(message)
    }
}
