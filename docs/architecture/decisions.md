# TOVI — Architecture Decisions

Decisions that block implementation work. Each one records what was decided, why, and
what it means for the code. Changing a decision means updating this file first.

Status key: **Accepted** · **Provisional** (default chosen, to be confirmed by benchmark or spike)

---

## D1. Certificate verification: pin the peer's Ed25519 key — Accepted

**Decision**

- Every device's QUIC/TLS certificate is self-signed with its long-term Ed25519 identity key.
- Both sides present certificates (mutual TLS).
- Custom rustls verifiers check the **key**, not a certificate chain:
  - The client verifies that the server certificate's public key equals the key it expected
    (from the QR code during pairing, or from the trusted-device store afterwards).
  - The server accepts any client certificate whose handshake signature is valid, then the
    application layer checks the client key against the trusted-device store, or runs the
    pairing flow (D2) if it is unknown.
- The TLS 1.3 handshake signature is **always** verified. There is no "accept any cert" mode,
  including in development builds.

**Why** — There is no certificate authority for devices on a home network. Pinning the identity
key gives the same MITM protection without one, and ties "device identity" (Ed25519) to "encrypted
transport" (QUIC) directly.

**Consequences** — `identity` owns certificate generation and both verifiers. `device_id` is
derived from the public key, so a device's ID can't be claimed without its private key.

---

## D2. Pairing: key pin + TLS channel binding — Accepted

**Decision**

1. Desktop creates a pairing session: 32-byte random `secret`, absolute expiry, single use.
2. QR code carries the desktop's public key, the secret, the expiry and candidate endpoints (D3).
3. Phone connects over QUIC and pins the desktop key from the QR (D1). Anyone intercepting the
   connection cannot present that key, so the phone knows it reached the right desktop.
4. Phone sends `PAIR` containing
   `MAC(key = secret, "TOVI-PAIR-v1" ‖ exporter ‖ phone_pubkey ‖ desktop_pubkey)`, where
   `MAC` is BLAKE3 in keyed mode (`blake3::keyed_hash`, consistent with D5) and `exporter` is
   32 bytes of TLS keying material exported from this connection (label
   `EXPORTER-TOVI-PAIR-v1`). The desktop recomputes and compares in constant time.
5. On a match, the desktop shows **Allow / Cancel** with the phone's name. Only on Allow are the
   keys stored as trusted, on both sides.
6. The session is invalidated on success, on expiry, and after 3 failed attempts.

**Why** — The exporter value is unique to this TLS connection, so a valid `PAIR` message can't be
replayed on, or relayed through, another connection. The phone's own key is inside the MAC, so
the desktop knows which key it is trusting. Uses only primitives rustls/quinn already provide
(`Connection::export_keying_material`).

**Known limit** — Someone who photographs the QR and connects before the real phone would pass
steps 3–4. Step 5 (explicit approval showing the phone's name) is the mitigation, so it must never
be skipped for first-time pairing. A PAKE (SPAKE2) can be added later if typed short codes are
needed; it is not needed for QR.

---

## D3. QR payload — Accepted

**Decision** — The QR encodes a URI: `tovi://pair/<base64url(CBOR)>`, so the phone's system
camera can open TOVI directly. The CBOR map contains:

| Key | Content |
|---|---|
| `v` | Payload version (`1`) |
| `k` | Desktop Ed25519 public key (32 bytes) |
| `s` | Pairing secret (32 bytes) |
| `x` | Expiry, Unix seconds (absolute) |
| `e` | Candidate endpoints: list of IPv4/IPv6 address + port |

**Lifetime: 60 seconds.** The desktop enforces expiry; the phone only uses `x` to show "code
expired" early without connecting.

**Why** — Absolute expiry because a relative "30" means nothing to the phone after scanning.
Several endpoints because Windows machines often have Hyper-V/WSL/VPN adapters and the first IP is
frequently wrong. The phone tries the candidates in parallel and keeps the first that answers with
the pinned key. Roughly 110 bytes of CBOR, about 150 URI characters: easily scannable.

---

## D4. Mobile bridge: UniFFI — Accepted

**Decision** — Expose `tovi-core` to Kotlin (Android) and Swift (iOS) through UniFFI. The public
API is async-friendly and callback-based for events (discovery, progress).

**Why** — Generates both Kotlin and Swift bindings from one definition, is maintained by Mozilla
and widely used in production. Hand-written JNI + C FFI would be two separate bindings to maintain.

**Consequences** — Public core types must be UniFFI-compatible (no generic or lifetime-bound types
at the boundary). Keep the boundary in a thin `tovi-ffi` crate so the core API stays idiomatic Rust.

---

## D5. Hashing and chunk size — Accepted (hash) / Provisional (chunk size)

**Hash: BLAKE3**, replacing SHA-256 from the original docs.

- Per-chunk hash, checked as each chunk arrives (a corrupt chunk is re-requested, not the file).
- Whole-file hash computed while streaming, checked before the atomic rename.
- Device fingerprints shown to users (if ever) use BLAKE3 of the public key.

**Why** — Several times faster than SHA-256 on phones without SHA extensions, which matters for
2 GB files. Same 256-bit security level.

**Chunk size: 8 MiB default, provisional.** Benchmark 4 / 8 / 16 MiB during spike validation
(TASKS §10) and update this entry.

---

## D6. Wire encoding: length-prefixed CBOR — Accepted

**Decision**

- Control messages (`HELLO`, `PAIR`, `TRANSFER_INIT`, …) are CBOR maps, framed as a 4-byte
  big-endian length followed by the CBOR body. Maximum control frame: 64 KiB; larger frames are a
  protocol error and close the connection.
- File data is **not** wrapped in CBOR. Each chunk is sent on its own QUIC stream: a small CBOR
  header (transfer ID, chunk index, length, hash) followed by the raw bytes.
- Rust implementation: `serde` + `ciborium`.

**Why** — CBOR is compact and readable from Kotlin, Swift and TypeScript without Rust, which keeps
a browser fallback or third-party SDK possible. Raw chunk bytes avoid copying gigabytes through an
encoder. The frame limit protects against malformed or hostile peers (Tech doc §42).

---

## D7. First milestone targets Windows and macOS together — Accepted

**Decision** — The spike milestone is phone → desktop on **both** Windows and macOS. Development
happens on Windows; CI already builds and tests macOS; real-device testing on a Mac is required
before the milestone is called done.

**Consequences**

- Windows-specific work moves earlier: firewall prompt/rule for inbound UDP, and filtering
  virtual adapters from QR endpoints (D3).
- Needs access to a Mac for testing. Until then, macOS coverage is CI only.
- The PRD already lists Windows in the MVP; the Tech doc's sprint order (Windows in Sprint 5) is
  superseded.
