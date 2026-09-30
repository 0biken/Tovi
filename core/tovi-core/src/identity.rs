//! Cryptographic device identity using Ed25519.
//!
//! Each installation has one long-term Ed25519 keypair. The public key *is* the
//! device's identity: [`DeviceId`] is derived from it, and the QUIC/TLS
//! certificate is signed with it (see [`tls`], decisions.md D1).

pub mod tls;

use anyhow::{Context, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey, KEYPAIR_LENGTH};
use rand_core::OsRng;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

/// Stable identifier for a device: its Ed25519 public key.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceId([u8; 32]);

impl DeviceId {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl From<&VerifyingKey> for DeviceId {
    fn from(key: &VerifyingKey) -> Self {
        Self(key.to_bytes())
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DeviceId({self})")
    }
}

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

    /// Load the identity from `store`, generating and saving one on first run
    pub fn load_or_generate(store: &dyn KeyStore) -> Result<Self> {
        if let Some(bytes) = store.load()? {
            return Self::from_bytes(&bytes);
        }
        let identity = Self::generate_new();
        store.store(&identity.to_bytes())?;
        tracing::info!("Generated new device identity {}", identity.device_id());
        Ok(identity)
    }

    /// Reconstruct identity from stored bytes (e.g. from Keychain / Credential Manager)
    pub fn from_bytes(bytes: &[u8; KEYPAIR_LENGTH]) -> Result<Self> {
        let signing_key = SigningKey::from_keypair_bytes(bytes)?;
        Ok(Self { signing_key })
    }

    /// Get the public verifying key
    pub fn public_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    /// The TOVI device ID, derived from the public key
    pub fn device_id(&self) -> DeviceId {
        DeviceId::from(&self.public_key())
    }

    /// Serialize the identity to save in OS secure storage
    pub fn to_bytes(&self) -> [u8; KEYPAIR_LENGTH] {
        self.signing_key.to_keypair_bytes()
    }

    /// Sign a payload (used to prove identity during the connection handshake)
    pub fn sign(&self, message: &[u8]) -> Signature {
        self.signing_key.sign(message)
    }

    pub(crate) fn signing_key(&self) -> &SigningKey {
        &self.signing_key
    }
}

/// Persistent storage for the device keypair.
///
/// Production implementations wrap the OS keychain (Keychain, DPAPI /
/// Credential Manager, Secret Service, Android Keystore).
pub trait KeyStore {
    /// Returns `None` if no identity has been stored yet
    fn load(&self) -> Result<Option<[u8; KEYPAIR_LENGTH]>>;
    fn store(&self, keypair: &[u8; KEYPAIR_LENGTH]) -> Result<()>;
}

/// Development-only key store: keeps the keypair **unencrypted** in a file.
/// On Unix the file is created with mode 0600. Do not ship this.
pub struct FileKeyStore {
    path: PathBuf,
}

impl FileKeyStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl KeyStore for FileKeyStore {
    fn load(&self) -> Result<Option<[u8; KEYPAIR_LENGTH]>> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).context(format!("reading {}", self.path.display())),
        };
        let keypair = bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("{} is not a valid keypair file", self.path.display()))?;
        Ok(Some(keypair))
    }

    fn store(&self, keypair: &[u8; KEYPAIR_LENGTH]) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&self.path)
            .with_context(|| format!("writing {}", self.path.display()))?;
        file.write_all(keypair)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keypair_bytes_round_trip() {
        let identity = DeviceIdentity::generate_new();
        let restored = DeviceIdentity::from_bytes(&identity.to_bytes()).unwrap();
        assert_eq!(identity.device_id(), restored.device_id());
    }

    #[test]
    fn device_id_displays_as_64_hex_chars() {
        let id = DeviceIdentity::generate_new().device_id().to_string();
        assert_eq!(id.len(), 64);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn file_key_store_persists_identity() {
        let path = std::env::temp_dir()
            .join(format!(
                "tovi-test-{}",
                DeviceIdentity::generate_new().device_id()
            ))
            .join("identity.key");
        let store = FileKeyStore::new(&path);

        assert!(store.load().unwrap().is_none());
        let first = DeviceIdentity::load_or_generate(&store).unwrap();
        let second = DeviceIdentity::load_or_generate(&store).unwrap();
        assert_eq!(first.device_id(), second.device_id());

        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn file_key_store_rejects_corrupt_file() {
        let dir = std::env::temp_dir().join(format!(
            "tovi-test-{}",
            DeviceIdentity::generate_new().device_id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("identity.key");
        fs::write(&path, b"too short").unwrap();

        assert!(FileKeyStore::new(&path).load().is_err());

        fs::remove_dir_all(dir).unwrap();
    }
}
