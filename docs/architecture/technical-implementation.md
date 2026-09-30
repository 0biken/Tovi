# TOVI
### Technical Implementation Document
**Organization:** KARMA Groups  
**Product:** TOVI  
**Status:** Architecture proposal / v0.1

---

# 1. Technical Objective

Build a cross-platform device-transfer system with:

- local-first networking
- QR-assisted bootstrap
- automatic discovery
- secure device identity
- encrypted transport
- resumable large-file transfer
- optional peer-to-peer transports
- optional remote relay
- optional cloud storage

The architecture must allow the transport layer to evolve without redesigning the application layer.

---

# 2. Key Technical Decision

TOVI should **not** implement one gigantic networking mechanism.

Instead, build a **Transport Abstraction Layer**.

```text
                    TOVI Client
                        │
                Transfer API
                        │
              Connection Manager
                        │
          ┌─────────────┼─────────────┐
          ↓             ↓             ↓
         LAN            P2P         Remote
          │             │             │
        mDNS        Wi-Fi Direct   Relay/ICE
        QR             Wi-Fi Aware
          │             │             │
          └─────────────┼─────────────┘
                        ↓
                 Secure Transport
                        ↓
                 Transfer Engine
```

This prevents the application from depending on one connectivity method.

---

# 3. Recommended Stack

## Mobile

### Android

**Kotlin + Jetpack Compose**

Native networking integrations are preferred because Android exposes Wi-Fi Direct and related networking APIs directly. Android 13+ also introduced/uses the `NEARBY_WIFI_DEVICES` permission for relevant Wi-Fi P2P operations.

### iOS

**SwiftUI**

Use native Apple networking APIs where direct peer-to-peer functionality is required.

Apple's current Network framework supports peer-to-peer Wi-Fi when explicitly enabled, and Apple is steering developers away from the older Multipeer Connectivity framework toward Network.

Wi-Fi Aware should be treated as an advanced transport on supported systems.

---

# 4. Desktop Stack

Recommended:

**Tauri 2 + React + TypeScript**

with:

**Rust core**

Why:

- small desktop footprint
- macOS
- Windows
- Linux
- strong native integration
- Rust is appropriate for network/crypto/file-transfer primitives
- same Rust transfer core can be shared conceptually across clients

---

# 5. Shared Core

Create a Rust package:

```text
tovi-core
```

Responsibilities:

- device identity
- session creation
- protocol framing
- transfer engine
- chunking
- hashing
- resume state
- cryptography
- connection state
- capability negotiation

Possible structure:

```text
tovi-core/
├── identity/
├── discovery/
├── pairing/
├── protocol/
├── transport/
│   ├── lan/
│   ├── wifi_direct/
│   ├── wifi_aware/
│   └── relay/
├── transfer/
├── crypto/
├── storage/
└── telemetry/
```

---

# 6. Networking Protocol

Recommended transport:

**QUIC over UDP**

QUIC is designed as a secure, multiplexed transport with low-latency connection establishment, flow control and stream support.

Advantages for TOVI:

- encrypted transport
- multiplexed streams
- efficient large-file transfers
- connection migration
- reliable delivery
- modern congestion control
- suitable foundation for future remote transfer

The application layer should remain transport-agnostic.

---

# 7. Application Protocol

Define a TOVI protocol:

**TVP — TOVI Transfer Protocol**

Version:

`TVP/1`

Conceptual flow:

```text
HELLO
CAPABILITIES
PAIR
AUTH
TRANSFER_INIT
TRANSFER_META
TRANSFER_CHUNK
TRANSFER_ACK
TRANSFER_COMPLETE
TRANSFER_VERIFY
```

---

# 8. Connection Negotiation

Example:

```json
{
  "protocol": "TVP/1",
  "device_id": "ed25519-public-key",
  "device_name": "Obioma's MacBook",
  "platform": "macos",
  "capabilities": [
    "lan",
    "qr",
    "quic",
    "resume"
  ]
}
```

A later version could advertise:

```text
wifi_direct
wifi_aware
relay
clipboard
folder_transfer
streaming
```

---

# 9. QR Bootstrap Protocol

QR payload should be short-lived: **60 seconds**, enforced by the desktop.

The QR encodes `tovi://pair/<base64url(CBOR)>` with:

```text
v   payload version (1)
k   desktop Ed25519 public key
s   one-time pairing secret (32 bytes)
x   expiry, absolute Unix seconds
e   candidate endpoints (address + port), several
```

The QR must not contain permanent secrets. Full specification: `decisions.md` D3.

---

# 10. QR Security

Recommended lifecycle:

```text
Desktop generates ephemeral session (secret + 60 s expiry)
        ↓
QR generated
        ↓
Phone scans
        ↓
Phone connects, pins desktop key from QR
        ↓
Phone proves secret, bound to this TLS session
        ↓
Desktop user approves (Allow / Cancel)
        ↓
QR session invalidated
```

The desktop enforces expiry; the phone's check is only for an early error message. Binding the
secret to the TLS session prevents replay and relay. Full handshake: `decisions.md` D2.

---

# 11. Cryptography

Recommended architecture:

### Device identity

**Ed25519**

### Key agreement

**X25519**, performed by the TLS 1.3 handshake inside QUIC. No separate key-agreement layer.

### Transport encryption

**TLS 1.3 through QUIC**, with certificates pinned to each device's Ed25519 key (`decisions.md` D1)

### File integrity

**BLAKE3**, per chunk and for the whole file (`decisions.md` D5)

---

# 12. Device Identity

Each TOVI installation gets a local identity:

```text
device_id
public_key
private_key
device_name
platform
created_at
```

Private key storage:

### macOS
Keychain

### Windows
Windows Credential Manager / DPAPI-backed storage

### Linux
Secret Service / OS keyring where available

### Android
Android Keystore

### iOS
Keychain

---

# 13. Discovery

## LAN discovery

Use service discovery through:

**mDNS / DNS-SD**

Service concept:

```text
_tovi._udp.local
```

Possible advertisement:

```text
_tovi._udp.local
name=Obioma-MacBook
port=48210
version=1
```

Android exposes Network Service Discovery for finding services on a local network.

Apple supports Bonjour/DNS-SD-style local service discovery, with current local-network permission requirements for applications.

---

# 14. Why QR Still Matters

mDNS is automatic but not universally reliable.

Routers can isolate devices.

Multicast can be blocked.

Enterprise networks can be unusual.

Therefore:

```text
Automatic discovery
       ↓ failure
QR bootstrap
       ↓ failure
Direct P2P
       ↓ failure
Remote transfer
```

This makes QR the deterministic fallback rather than the only connection technology.

---

# 15. LAN Connection

When both devices are on the same network:

```text
Phone
  ↓
Discover TOVI
  ↓
Resolve local endpoint
  ↓
Connect QUIC
  ↓
TLS handshake
  ↓
Device authentication
  ↓
Transfer
```

No cloud server is necessary.

---

# 16. Starlink Case

For a standard Starlink setup using the **Default** Wi-Fi network:

```text
             Starlink
                 │
        ┌────────┴────────┐
        │                 │
     Android            MacBook
```

Starlink documents Default mode as allowing connected devices to communicate, specifically describing it as suitable for file sharing and related local-network use cases. Guest mode isolates clients and is therefore inappropriate for local TOVI transfers.

TOVI should therefore first attempt local LAN communication.

The user does not need to disable internet access.

TOVI simply routes the payload locally.

---

# 17. Direct P2P Layer

Android:

**Wi-Fi Direct**

Android officially supports Wi-Fi P2P connections without an intermediate access point.

Apple:

**Wi-Fi Aware / peer-to-peer Wi-Fi**

Apple's current documentation describes Wi-Fi Aware as supporting nearby device discovery and secure peer-to-peer connections without internet or an access point.

Implementation must be capability-driven.

Never assume every device supports the same transport.

---

# 18. Transport Selection Algorithm

Conceptually:

```text
getTargetDevice()

if local_lan_reachable:          # endpoint from mDNS, or from a scanned QR
    use LAN
else if direct_p2p_supported:
    use best available P2P
else if remote_mode_enabled:
    use relay
else:
    show connection assistance
```

Later:

```text
selectTransport()
{
    score each transport based on:

    availability
    bandwidth
    latency
    battery impact
    reliability
    user settings

    choose highest viable score
}
```

---

# 19. Transfer Engine

Files should not be loaded entirely into memory.

Use streaming I/O.

```text
File
 ↓
Chunker
 ↓
Encryption / transport
 ↓
QUIC streams
 ↓
Receiver
 ↓
Chunk writer
 ↓
Hash verification
 ↓
Atomic finalize
```

---

# 20. Chunking

Recommended initial chunk size:

**4–16 MiB**

The exact value should be benchmarked.

Each transfer:

```text
file_id
file_size
chunk_size
chunk_count
blake3
```

Each chunk:

```text
chunk_index
offset
length
payload
hash
```

---

# 21. Resumability

The receiver keeps a local transfer manifest.

Example:

```json
{
  "transfer_id": "abc",
  "file_size": 2147483648,
  "chunk_size": 8388608,
  "received": [
    0,
    1,
    2,
    3,
    4
  ]
}
```

After reconnection:

```text
Sender → resume manifest
Receiver → missing chunks
Sender → remaining chunks
```

This is important for large video and ZIP transfers.

---

# 22. Atomic File Writes

Never write directly to the final destination.

Use:

```text
filename.tovi.part
```

Then:

```text
transfer
 ↓
hash verification
 ↓
rename
 ↓
filename.ext
```

This prevents corrupted partial files from appearing as completed transfers.

---

# 23. Transfer State Machine

```text
QUEUED
  ↓
CONNECTING
  ↓
AUTHENTICATING
  ↓
TRANSFERRING
  ↓
VERIFYING
  ↓
COMPLETED
```

Error path:

```text
TRANSFERRING
     ↓
INTERRUPTED
     ↓
WAITING
     ↓
RECONNECTING
     ↓
RESUMING
```

---

# 24. Local Storage Model

No central database is required for local mode.

Use a local embedded database.

Recommended:

**SQLite**

Tables:

```text
devices
transfers
transfer_chunks
settings
sessions
```

---

# 25. Device Record

```text
devices
-------
id
public_key
name
platform
last_seen
trusted
created_at
updated_at
```

---

# 26. Transfer Record

```text
transfers
---------
id
device_id
direction
file_name
file_size
mime_type
status
source_path
destination_path
blake3
created_at
completed_at
```

---

# 27. Privacy Boundary

The transfer engine should know:

- bytes
- metadata necessary for transfer
- destination
- session identity

Analytics should not know:

- file contents
- clipboard contents
- document contents

Keep the two systems separate.

---

# 28. Remote Mode Architecture

TOVI should eventually support NAT traversal.

Possible architecture:

```text
Phone
  │
  ├──── local candidate
  │
  ├──── peer candidate
  │
  └──── relay candidate
               │
             Server
               │
             Laptop
```

WebRTC-style ICE concepts can inform candidate gathering for remote mode, but local transfers should not unnecessarily involve the cloud.

---

# 29. Relay Server

For remote transfer:

```text
relay.tovi.io
```

The relay should ideally operate as an encrypted packet forwarder.

The relay should not automatically persist files.

Possible model:

```text
Encrypted stream
Phone → Relay → Laptop
```

The server sees:

- connection metadata
- encrypted payload

but not plaintext file data.

---

# 30. Optional Cloud Storage

Cloud storage is a separate subsystem:

```text
TOVI Core
   │
   ├── Local transfer
   │
   ├── Remote relay
   │
   └── Cloud storage
```

Do not make:

```text
Mobile → Cloud → Desktop
```

the underlying architecture of the local-transfer feature.

---

# 31. API Surface

Internal Transfer API:

```text
discoverDevices()
pairDevice()
trustDevice()
revokeDevice()

createTransfer()
startTransfer()
pauseTransfer()
resumeTransfer()
cancelTransfer()

getTransfer()
getTransferHistory()

selectTransport()
getConnectionStatus()
```

---

# 32. Share Sheet Integration

Android:

```text
Share → TOVI
```

iOS:

```text
Share → TOVI
```

Desktop:

```text
Right click → Send with TOVI
```

This is strategically important because users should not have to open TOVI manually every time.

---

# 33. Desktop Architecture

```text
┌─────────────────────────────┐
│ React / Tauri UI            │
├─────────────────────────────┤
│ Device Manager              │
│ Transfer UI                 │
│ Settings                    │
├─────────────────────────────┤
│ TOVI Rust Core              │
├─────────────────────────────┤
│ Discovery / Pairing         │
│ QUIC / Crypto               │
│ Transfer Engine             │
├─────────────────────────────┤
│ OS networking / filesystem  │
└─────────────────────────────┘
```

Desktop should be capable of running as a background service.

---

# 34. Mobile Architecture

Android:

```text
Compose UI
    ↓
TOVI Mobile Service
    ↓
Rust Core
    ↓
Android Transport Adapter
    ↓
Network
```

iOS:

```text
SwiftUI
   ↓
TOVI Service Layer
   ↓
Rust Core
   ↓
Apple Network / Wi-Fi Aware Adapter
   ↓
Network
```

---

# 35. Platform Abstraction

The Rust core should expose a stable interface for:

```text
discover
advertise
connect
send
receive
pause
resume
cancel
verify
```

Platform-specific code should implement:

```text
LAN adapter
P2P adapter
filesystem adapter
background execution
permission handling
secure key storage
```

---

# 36. Browser Product

TOVI should support a browser fallback, but the browser should not be the core.

Potential experience:

```text
Laptop TOVI
 ↓
Generate local session
 ↓
QR
 ↓
Phone camera
 ↓
Browser opens local transfer page
 ↓
Transfer
```

The browser version can be useful when the user does not want to install the mobile application.

However, browser networking restrictions should be treated as a secondary path.

---

# 37. Performance Targets

For a modern local Wi-Fi environment:

### Small files

<2 seconds perceived interaction time.

### Large files

Transfer speed should be constrained primarily by the network and storage hardware, not application-level overhead.

### CPU overhead

Encryption should be stream-based and hardware-accelerated where possible.

### Memory

No full-file memory buffering.

---

# 38. Battery

TOVI should not aggressively scan continuously.

Use:

- passive discovery where possible
- bounded scan windows
- event-driven network callbacks
- background execution only when justified

---

# 39. Permissions

Android may require:

- `INTERNET`
- `ACCESS_NETWORK_STATE`
- `NEARBY_WIFI_DEVICES`
- relevant Wi-Fi P2P permissions

The exact set should track the Android API levels supported.

Android's current documentation states that Wi-Fi P2P operations on Android 13+ require `NEARBY_WIFI_DEVICES`, with older versions using different permission behavior.

iOS:

- Local Network permission
- appropriate networking entitlements for supported P2P functionality
- Wi-Fi Aware entitlement where applicable

Apple requires local-network usage declarations for apps using the local network and Bonjour service declarations for Bonjour discovery.

---

# 40. QA Strategy

TOVI should be treated as a networking product, so QA must focus heavily on real-world topology.

### Test matrix

| Source | Target | Network |
|---|---|---|
| Android | macOS | Starlink Default |
| Android | Windows | Starlink Default |
| Android | Linux | Starlink Default |
| macOS | Android | Home Wi-Fi |
| macOS | Android | No internet |
| Windows | Android | Home Wi-Fi |
| Linux | Android | Home Wi-Fi |
| Android | macOS | QR |
| Android | Windows | QR |
| Android | Android | Direct P2P |

---

# 41. Adversarial Network Testing

Test:

- internet disconnected
- weak Wi-Fi
- network switching
- router restart
- device sleep
- laptop lid close
- phone screen lock
- IP address changes
- packet loss
- client isolation
- VPN active
- firewall active
- large files
- duplicate filenames
- low disk space

---

# 42. Security Testing

Required:

- MITM testing
- QR replay testing
- expired QR testing
- malicious device impersonation
- malformed packet testing
- path traversal testing
- file overwrite testing
- partial-transfer corruption
- checksum mismatch
- revoked-device access

---

# 43. Threat Model

Threat actors:

### Nearby attacker

Attempts to connect to a user's TOVI instance.

Mitigation:

- authenticated pairing
- cryptographic identity
- user approval
- ephemeral sessions

### Malicious router

Could inspect metadata or interfere with local traffic.

Mitigation:

- end-to-end encryption

### Stolen QR screenshot

Mitigation:

- short-lived QR sessions
- single-use tokens
- cryptographic handshake

---

# 44. Engineering Repositories

Suggested structure:

```text
tovi/
├── apps/
│   ├── android/
│   ├── ios/
│   └── desktop/
│
├── packages/
│   ├── protocol/
│   ├── ui/
│   └── sdk/
│
├── core/
│   └── tovi-core/
│
├── services/
│   ├── relay/
│   └── cloud/
│
├── docs/
│   ├── prd/
│   ├── architecture/
│   ├── protocol/
│   └── security/
│
└── tests/
```

---

# 45. CI/CD

### Desktop

Build:

- `.dmg`
- `.msi`
- `.deb`
- `.AppImage`

### Android

- APK
- AAB

### iOS

- TestFlight
- App Store

Automated:

- unit tests
- integration tests
- protocol compatibility tests
- security tests
- transfer tests

---

# 46. Protocol Compatibility

Every TOVI instance must advertise:

```text
protocol_version
minimum_protocol_version
capabilities
```

Example:

```json
{
  "protocol": "TVP/1",
  "min_protocol": "1",
  "capabilities": [
    "lan",
    "quic",
    "resume"
  ]
}
```

This allows old clients to continue working after protocol upgrades.

---

# 47. Feature Flags

Important flags:

```text
enable_wifi_direct
enable_wifi_aware
enable_remote_relay
enable_cloud
enable_clipboard
enable_background_receive
```

This allows controlled rollout.

---

# 48. Recommended Development Sequence

## Sprint 0 — Architecture spike

Prove:

```text
Android ↔ macOS
```

with:

- LAN discovery
- QR pairing
- QUIC
- one file transfer

This is the critical technical spike.

---

## Sprint 1 — Transfer core

Build:

- protocol
- pairing
- transfer engine
- hashing
- local database
- resume

---

## Sprint 2 — Desktop product

Build:

- Tauri UI
- device list
- QR
- drag and drop
- transfer history

---

## Sprint 3 — Android

Build:

- Android client
- share sheet
- QR scanning
- receiving
- local notifications

---

## Sprint 4 — Hardening

Focus on:

- interrupted transfers
- sleep/wake
- network changes
- large files
- security
- battery

---

## Sprint 5 — Windows

Windows is now part of the first milestone (`decisions.md` D7). This sprint covers the remaining OS integration only.

Reuse core.

Add:

- Windows filesystem
- firewall handling
- startup service
- context-menu integration

---

## Sprint 6 — Linux

Add:

- system service
- filesystem integration
- desktop notifications
- distro packaging

---

## Sprint 7 — iOS

Implement:

- Local Network
- native peer networking
- Wi-Fi Aware where supported
- share sheet
- background limitations

---

# 49. Critical Technical Spike

Before committing to the full product roadmap, engineering must answer five questions.

### Question 1

Can Android and macOS reliably establish a local TOVI QUIC connection on a normal Starlink Default network?

### Question 2

Can QR pairing recover reliably when mDNS discovery is unavailable?

### Question 3

Can a transfer resume after Wi-Fi interruption?

### Question 4

Can the same Rust transfer core work cleanly across Android, iOS and desktop?

### Question 5

What percentage of target devices support a genuinely direct P2P transport that avoids the access point?

The fifth question determines how aggressively the product can market the "direct/no network" capability.

---

# 50. Important Product/Technical Constraint

TOVI must **not** promise:

> "Scan a QR code and transmit data with literally no wireless/network transport."

A QR code is an optical bootstrap mechanism. Something still has to carry the data.

The accurate product promise is:

> **No internet required.**

And, where native peer-to-peer transport is available:

> **Connect directly without a router.**

That distinction should remain technically honest throughout marketing.

---

# 51. Architecture Diagram

```text
                         ┌──────────────┐
                         │ TOVI Client  │
                         └──────┬───────┘
                                │
                        Device Controller
                                │
                   ┌────────────┴────────────┐
                   │                         │
              Discovery                 Connection
                   │                         │
             ┌─────┴─────┐           ┌─────┴────────┐
             │           │           │              │
            mDNS        QR          LAN             P2P
             │           │           │              │
             └───────────┴───────────┴──────────────┘
                                │
                           QUIC / TLS
                                │
                       Transfer Protocol
                                │
                      Transfer Engine
                                │
                     ┌──────────┴──────────┐
                     │                     │
                  Storage              Clipboard
                     │
                Local filesystem
```

---

# 52. Product Architecture Principle

The most important architectural rule:

> **Transport is replaceable. The user experience is not.**

Today:

```text
LAN
```

Tomorrow:

```text
Wi-Fi Direct
Wi-Fi Aware
Remote relay
Cloud
```

But the user still sees:

```text
Scan → Connect → Move
```

---

# 53. Long-Term Technical Opportunity

Once the transfer protocol is stable, KARMA Groups can expose:

**TOVI SDK**

Possible API:

```typescript
await tovi.send({
  target: "Obioma's MacBook",
  file: photo
})
```

Applications could integrate directly with TOVI.

That moves TOVI from being merely a consumer app to becoming a **device-to-device infrastructure layer**.

---

# 54. Final CTO Recommendation

The first engineering prototype should intentionally be narrow:

```text
Android
   ↕
Starlink LAN
   ↕
macOS
```

with exactly four capabilities:

```text
mDNS discovery
QR pairing
QUIC transfer
resume
```

Do not start with cloud.

Do not start with accounts.

Do not start with AI.

Do not start with social features.

Prove that **a 2 GB file can move from Android to Mac on a normal home network with a QR scan and no internet dependency**.

Once that works beautifully, the rest of TOVI becomes an exercise in expanding the transport matrix and platform reach rather than reinventing the core product.