//! TLS certificates and key-pinning verifiers (decisions.md D1).
//!
//! Every device presents a self-signed certificate made from its Ed25519
//! identity key. Trust comes from the **key**, not a certificate chain:
//!
//! - The client pins the server's key: the connection fails unless the server
//!   certificate carries exactly the expected Ed25519 key (from a scanned QR or
//!   the trusted-device store).
//! - The server requires a client certificate and checks it is a well-formed
//!   Ed25519 certificate. Whether that key is *trusted* is decided afterwards by
//!   the application (trusted-device store or pairing flow); read it with
//!   [`public_key_from_cert`] on the peer certificate.
//!
//! In both directions rustls verifies the TLS 1.3 handshake signature, so the
//! peer must hold the private key. There is no "accept any certificate" mode.

use super::DeviceIdentity;
use anyhow::{anyhow, Result};
use ed25519_dalek::pkcs8::EncodePrivateKey;
use ed25519_dalek::VerifyingKey;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, DistinguishedName, PeerMisbehaved,
    ServerConfig, SignatureScheme,
};
use std::sync::Arc;

/// Server name used in every TOVI certificate and connection. It carries no
/// meaning: identity is checked by key, not by name.
pub const SERVER_NAME: &str = "tovi";

/// DER prefix of an Ed25519 SubjectPublicKeyInfo (RFC 8410); the 32-byte key follows.
const ED25519_SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

impl DeviceIdentity {
    /// Self-signed certificate and private key for this device's TLS endpoint
    pub fn tls_certificate(&self) -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>)> {
        let pkcs8 = self
            .signing_key()
            .to_pkcs8_der()
            .map_err(|e| anyhow!("encoding identity key as PKCS#8: {e}"))?;
        let pkcs8 = PrivatePkcs8KeyDer::from(pkcs8.as_bytes().to_vec());

        let key_pair = rcgen::KeyPair::from_pkcs8_der_and_sign_algo(&pkcs8, &rcgen::PKCS_ED25519)?;
        let cert =
            rcgen::CertificateParams::new(vec![SERVER_NAME.to_string()])?.self_signed(&key_pair)?;

        Ok((cert.der().clone(), PrivateKeyDer::Pkcs8(pkcs8)))
    }

    /// TLS config for accepting connections. Requires a client certificate.
    pub fn server_config(&self) -> Result<ServerConfig> {
        let provider = provider();
        let (cert, key) = self.tls_certificate()?;
        let config = ServerConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .with_client_cert_verifier(Arc::new(Ed25519ClientVerifier::new(&provider)))
            .with_single_cert(vec![cert], key)?;
        Ok(config)
    }

    /// TLS config for connecting to the device whose identity key is `server_key`
    pub fn client_config(&self, server_key: VerifyingKey) -> Result<ClientConfig> {
        let provider = provider();
        let (cert, key) = self.tls_certificate()?;
        let config = ClientConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .dangerous() // Custom verifier: pins the key instead of checking a CA chain
            .with_custom_certificate_verifier(Arc::new(PinnedServerVerifier::new(
                server_key, &provider,
            )))
            .with_client_auth_cert(vec![cert], key)?;
        Ok(config)
    }
}

/// Extract the Ed25519 identity key from a TOVI certificate
pub fn public_key_from_cert(cert: &CertificateDer<'_>) -> Result<VerifyingKey, rustls::Error> {
    let cert = webpki::EndEntityCert::try_from(cert)
        .map_err(|_| rustls::Error::InvalidCertificate(CertificateError::BadEncoding))?;
    let spki = cert.subject_public_key_info();
    let key = spki
        .as_ref()
        .strip_prefix(&ED25519_SPKI_PREFIX[..])
        .and_then(|key| <[u8; 32]>::try_from(key).ok())
        .ok_or_else(|| rustls::Error::General("certificate key is not Ed25519".into()))?;
    VerifyingKey::from_bytes(&key)
        .map_err(|_| rustls::Error::InvalidCertificate(CertificateError::BadEncoding))
}

fn verify_ed25519_tls13(
    message: &[u8],
    cert: &CertificateDer<'_>,
    dss: &DigitallySignedStruct,
    algorithms: &WebPkiSupportedAlgorithms,
) -> Result<HandshakeSignatureValid, rustls::Error> {
    // We only advertise Ed25519, so any other scheme is a protocol violation
    if dss.scheme != SignatureScheme::ED25519 {
        return Err(PeerMisbehaved::SignedHandshakeWithUnadvertisedSigScheme.into());
    }
    rustls::crypto::verify_tls13_signature(message, cert, dss, algorithms)
}

fn tls12_unsupported() -> rustls::Error {
    rustls::Error::General("TOVI requires TLS 1.3".into())
}

/// Client-side verifier: accepts only the pinned server key
#[derive(Debug)]
struct PinnedServerVerifier {
    expected: VerifyingKey,
    algorithms: WebPkiSupportedAlgorithms,
}

impl PinnedServerVerifier {
    fn new(expected: VerifyingKey, provider: &CryptoProvider) -> Self {
        Self {
            expected,
            algorithms: provider.signature_verification_algorithms,
        }
    }
}

impl ServerCertVerifier for PinnedServerVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if public_key_from_cert(end_entity)? != self.expected {
            return Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ));
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(tls12_unsupported())
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_ed25519_tls13(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }
}

/// Server-side verifier: requires a well-formed Ed25519 client certificate.
/// Trust in the key itself is decided by the application after the handshake.
#[derive(Debug)]
struct Ed25519ClientVerifier {
    algorithms: WebPkiSupportedAlgorithms,
}

impl Ed25519ClientVerifier {
    fn new(provider: &CryptoProvider) -> Self {
        Self {
            algorithms: provider.signature_verification_algorithms,
        }
    }
}

impl ClientCertVerifier for Ed25519ClientVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        public_key_from_cert(end_entity)?;
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(tls12_unsupported())
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_ed25519_tls13(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::{ClientConnection, ServerConnection};

    /// Run a TLS handshake entirely in memory
    fn handshake(
        client: ClientConfig,
        server: ServerConfig,
    ) -> Result<(ClientConnection, ServerConnection), rustls::Error> {
        let name = ServerName::try_from(SERVER_NAME).unwrap();
        let mut client = ClientConnection::new(Arc::new(client), name)?;
        let mut server = ServerConnection::new(Arc::new(server))?;
        let mut buf = Vec::new();

        for _ in 0..10 {
            if !client.is_handshaking() && !server.is_handshaking() {
                return Ok((client, server));
            }
            buf.clear();
            client.write_tls(&mut buf).unwrap();
            server.read_tls(&mut buf.as_slice()).unwrap();
            server.process_new_packets()?;

            buf.clear();
            server.write_tls(&mut buf).unwrap();
            client.read_tls(&mut buf.as_slice()).unwrap();
            client.process_new_packets()?;
        }
        panic!("handshake did not finish");
    }

    #[test]
    fn certificate_carries_identity_key() {
        let identity = DeviceIdentity::generate_new();
        let (cert, _) = identity.tls_certificate().unwrap();
        assert_eq!(public_key_from_cert(&cert).unwrap(), identity.public_key());
    }

    #[test]
    fn handshake_succeeds_with_pinned_key() {
        let desktop = DeviceIdentity::generate_new();
        let phone = DeviceIdentity::generate_new();

        let (_, server) = handshake(
            phone.client_config(desktop.public_key()).unwrap(),
            desktop.server_config().unwrap(),
        )
        .unwrap();

        // Server learns the client's identity key from its certificate
        let client_cert = &server.peer_certificates().unwrap()[0];
        assert_eq!(
            public_key_from_cert(client_cert).unwrap(),
            phone.public_key()
        );
    }

    #[test]
    fn handshake_fails_when_server_key_does_not_match_pin() {
        let desktop = DeviceIdentity::generate_new();
        let impostor = DeviceIdentity::generate_new();
        let phone = DeviceIdentity::generate_new();

        let result = handshake(
            phone.client_config(desktop.public_key()).unwrap(),
            impostor.server_config().unwrap(),
        );

        assert!(matches!(
            result,
            Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure
            ))
        ));
    }

    #[test]
    fn rejects_non_ed25519_certificate() {
        let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let cert = rcgen::CertificateParams::new(vec![SERVER_NAME.to_string()])
            .unwrap()
            .self_signed(&key_pair)
            .unwrap();

        assert!(public_key_from_cert(cert.der()).is_err());
    }
}
