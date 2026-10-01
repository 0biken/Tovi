//! TVP/1 (TOVI Transfer Protocol) message framing and version negotiation.
//!
//! Control messages are CBOR, framed as a 4-byte big-endian length followed by
//! the CBOR body, at most [`MAX_FRAME_LEN`] bytes (decisions.md D6). Anything
//! larger, or anything that fails to decode, is a protocol error and the
//! caller should close the connection. File data is not sent through here.

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Highest protocol version this build speaks
pub const PROTOCOL_VERSION: u32 = 1;
/// Lowest protocol version this build still accepts
pub const MIN_PROTOCOL_VERSION: u32 = 1;

/// Largest control frame accepted, excluding the 4-byte length prefix
pub const MAX_FRAME_LEN: usize = 64 * 1024;

pub const MAX_DEVICE_NAME_LEN: usize = 64;
const MAX_PLATFORM_LEN: usize = 16;
const MAX_CAPABILITIES: usize = 32;
const MAX_CAPABILITY_LEN: usize = 32;
const MAX_REASON_LEN: usize = 256;
/// Longest file name accepted on the wire, in bytes, before sanitising
const MAX_FILE_NAME_LEN: usize = 1024;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Message {
    /// First message from each side: versions, name and capabilities.
    /// Sent only inside the encrypted connection, so names never leak to the LAN.
    Hello(Hello),
    /// Phone → desktop: proof of the QR secret, bound to this TLS session (D2)
    Pair(Pair),
    /// Desktop → phone: outcome of pairing, after the user's Allow / Cancel
    PairResult(PairResult),
    /// Sender → receiver, on a new bidirectional stream: a file to send
    TransferOffer(TransferOffer),
    /// Receiver → sender: accept or decline the offer
    TransferResponse(TransferResponse),
    /// Sender → receiver, first frame of each chunk's unidirectional stream;
    /// the raw chunk bytes follow the frame
    Chunk(ChunkHeader),
    /// Sender → receiver, after all chunks: hash of the whole file
    TransferComplete(TransferComplete),
    /// Receiver → sender: whether the file arrived intact and was saved
    TransferResult(TransferResult),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct TransferOffer {
    #[serde(with = "serde_bytes")]
    pub transfer_id: [u8; 16],
    /// As named on the sender; the receiver sanitises it before use
    pub file_name: String,
    pub file_size: u64,
    pub chunk_size: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct TransferResponse {
    #[serde(with = "serde_bytes")]
    pub transfer_id: [u8; 16],
    pub accepted: bool,
    pub reason: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ChunkHeader {
    #[serde(with = "serde_bytes")]
    pub transfer_id: [u8; 16],
    /// Chunk number; its offset and length follow from the offer
    pub index: u64,
    /// BLAKE3 hash of this chunk's bytes
    #[serde(with = "serde_bytes")]
    pub hash: [u8; 32],
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct TransferComplete {
    #[serde(with = "serde_bytes")]
    pub transfer_id: [u8; 16],
    /// BLAKE3 hash of the whole file
    #[serde(with = "serde_bytes")]
    pub file_hash: [u8; 32],
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct TransferResult {
    #[serde(with = "serde_bytes")]
    pub transfer_id: [u8; 16],
    pub ok: bool,
    pub reason: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    pub protocol: u32,
    pub min_protocol: u32,
    pub device_name: String,
    pub platform: String,
    pub capabilities: Vec<String>,
}

impl Hello {
    /// A `Hello` for this build with the given device name
    pub fn new(device_name: impl Into<String>) -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
            min_protocol: MIN_PROTOCOL_VERSION,
            device_name: device_name.into(),
            platform: std::env::consts::OS.to_string(),
            capabilities: vec!["lan".into(), "quic".into(), "qr".into()],
        }
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.device_name.chars().count() <= MAX_DEVICE_NAME_LEN,
            "device name too long"
        );
        ensure!(self.platform.len() <= MAX_PLATFORM_LEN, "platform too long");
        ensure!(
            self.capabilities.len() <= MAX_CAPABILITIES
                && self
                    .capabilities
                    .iter()
                    .all(|c| c.len() <= MAX_CAPABILITY_LEN),
            "too many or too long capabilities"
        );
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    #[serde(with = "serde_bytes")]
    pub proof: [u8; 32],
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PairResult {
    pub accepted: bool,
    /// Why pairing was refused; `None` when accepted
    pub reason: Option<String>,
}

impl Message {
    fn validate(&self) -> Result<()> {
        match self {
            Message::Hello(hello) => hello.validate(),
            Message::PairResult(PairResult { reason, .. })
            | Message::TransferResponse(TransferResponse { reason, .. })
            | Message::TransferResult(TransferResult { reason, .. }) => validate_reason(reason),
            Message::TransferOffer(offer) => {
                ensure!(
                    offer.file_name.len() <= MAX_FILE_NAME_LEN,
                    "file name too long"
                );
                Ok(())
            }
            Message::Pair(_) | Message::Chunk(_) | Message::TransferComplete(_) => Ok(()),
        }
    }
}

fn validate_reason(reason: &Option<String>) -> Result<()> {
    let too_long = reason.as_ref().is_some_and(|r| r.len() > MAX_REASON_LEN);
    ensure!(!too_long, "reason too long");
    Ok(())
}

/// Encode one message as a CBOR body (no length prefix)
pub fn encode(message: &Message) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    ciborium::into_writer(message, &mut body).context("encoding message")?;
    ensure!(body.len() <= MAX_FRAME_LEN, "message exceeds frame limit");
    Ok(body)
}

/// Decode and validate one CBOR body (no length prefix)
pub fn decode(body: &[u8]) -> Result<Message> {
    ensure!(body.len() <= MAX_FRAME_LEN, "frame exceeds limit");
    let message: Message = ciborium::from_reader(body).context("malformed message")?;
    message.validate()?;
    Ok(message)
}

/// Write one length-prefixed message
pub async fn write_message<W: AsyncWrite + Unpin>(writer: &mut W, message: &Message) -> Result<()> {
    let body = encode(message)?;
    // Cannot overflow: encode() caps the body at MAX_FRAME_LEN
    writer.write_u32(body.len() as u32).await?;
    writer.write_all(&body).await?;
    Ok(())
}

/// Read one length-prefixed message. Rejects oversized frames before
/// allocating for them.
pub async fn read_message<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Message> {
    let len = reader.read_u32().await.context("reading frame length")? as usize;
    ensure!(len <= MAX_FRAME_LEN, "frame of {len} bytes exceeds limit");
    let mut body = vec![0u8; len];
    reader
        .read_exact(&mut body)
        .await
        .context("reading frame body")?;
    decode(&body)
}

/// How long [`finish_with`] waits for the peer to take a final message
pub const FINISH_TIMEOUT: Duration = Duration::from_secs(5);

/// Send a final message, close the stream, and wait (up to [`FINISH_TIMEOUT`])
/// until the peer has read it, so the caller can't drop the connection while
/// the message is still in flight
pub async fn finish_with(send: &mut quinn::SendStream, message: &Message) -> Result<()> {
    write_message(send, message).await?;
    send.finish()?;
    let _ = tokio::time::timeout(FINISH_TIMEOUT, send.stopped()).await;
    Ok(())
}

/// Read a message and require it to be a `Hello`
pub async fn read_hello<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Hello> {
    match read_message(reader).await? {
        Message::Hello(hello) => Ok(hello),
        other => bail!("expected HELLO, got {other:?}"),
    }
}

/// Pick the protocol version both sides speak: the highest version within
/// both ranges
pub fn negotiate(ours: &Hello, theirs: &Hello) -> Result<u32> {
    let version = ours.protocol.min(theirs.protocol);
    let floor = ours.min_protocol.max(theirs.min_protocol);
    ensure!(
        version >= floor,
        "no common protocol version (we speak {}-{}, peer speaks {}-{})",
        ours.min_protocol,
        ours.protocol,
        theirs.min_protocol,
        theirs.protocol
    );
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_core::{OsRng, RngCore};

    fn hello_with_versions(min: u32, max: u32) -> Hello {
        Hello {
            protocol: max,
            min_protocol: min,
            ..Hello::new("test")
        }
    }

    #[tokio::test]
    async fn messages_round_trip_through_framing() {
        let messages = [
            Message::Hello(Hello::new("Obioma's MacBook")),
            Message::Pair(Pair { proof: [7; 32] }),
            Message::PairResult(PairResult {
                accepted: false,
                reason: Some("declined".into()),
            }),
        ];
        let (mut a, mut b) = tokio::io::duplex(4096);
        for message in &messages {
            write_message(&mut a, message).await.unwrap();
        }
        for message in &messages {
            assert_eq!(&read_message(&mut b).await.unwrap(), message);
        }
    }

    #[test]
    fn keys_and_proofs_encode_as_compact_byte_strings() {
        let body = encode(&Message::Pair(Pair { proof: [0xAB; 32] })).unwrap();
        // Map overhead + "type"/"PAIR"/"proof" keys + a 34-byte byte string,
        // not an array of 32 separate integers
        assert!(body.len() < 60, "PAIR encoded to {} bytes", body.len());
    }

    #[tokio::test]
    async fn oversized_frame_is_rejected_before_reading_body() {
        let (mut a, mut b) = tokio::io::duplex(64);
        a.write_u32(u32::MAX).await.unwrap();
        let err = read_message(&mut b).await.unwrap_err();
        assert!(err.to_string().contains("exceeds limit"));
    }

    #[test]
    fn wrong_length_proof_is_rejected() {
        #[derive(Serialize)]
        struct BadPair<'a> {
            r#type: &'a str,
            #[serde(with = "serde_bytes")]
            proof: &'a [u8],
        }
        let mut body = Vec::new();
        ciborium::into_writer(
            &BadPair {
                r#type: "PAIR",
                proof: &[1; 31],
            },
            &mut body,
        )
        .unwrap();
        assert!(decode(&body).is_err());
    }

    #[test]
    fn overlong_device_name_is_rejected() {
        let hello = Hello::new("x".repeat(MAX_DEVICE_NAME_LEN + 1));
        let mut body = Vec::new();
        ciborium::into_writer(&Message::Hello(hello), &mut body).unwrap();
        assert!(decode(&body).is_err());
    }

    #[test]
    fn decoder_survives_random_input() {
        // Lightweight stand-in for a fuzz target: must never panic
        let mut buf = [0u8; 256];
        for _ in 0..20_000 {
            let len = (OsRng.next_u32() as usize) % buf.len();
            OsRng.fill_bytes(&mut buf[..len]);
            let _ = decode(&buf[..len]);
        }
    }

    #[test]
    fn negotiation_picks_highest_common_version() {
        assert_eq!(
            negotiate(&hello_with_versions(1, 3), &hello_with_versions(2, 5)).unwrap(),
            3
        );
        assert_eq!(
            negotiate(&hello_with_versions(1, 1), &hello_with_versions(1, 1)).unwrap(),
            1
        );
    }

    #[test]
    fn negotiation_fails_without_overlap() {
        assert!(negotiate(&hello_with_versions(1, 1), &hello_with_versions(2, 3)).is_err());
    }
}
