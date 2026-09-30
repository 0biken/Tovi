# TOVI
### Product Requirements Document
**Company:** KARMA Groups  
**Product:** TOVI  
**Working tagline:** Move without the middleman.  
**Document status:** Product definition / v0.1  
**Owner:** Head of Product, KARMA Groups  
**Technical owner:** CTO, KARMA Groups

---

# 1. Executive Summary

TOVI is a cross-platform personal device bridge designed to make moving information between a user's own devices as effortless as possible.

The product exists because modern device ecosystems remain fragmented:

- Android ↔ macOS
- Android ↔ Windows
- Android ↔ Linux
- iPhone ↔ Windows
- iPhone ↔ Linux
- Mac ↔ Android
- Mac ↔ Linux

Users regularly resort to WhatsApp, Telegram, email, cloud drives, USB cables, temporary file-sharing websites, or vendor-specific ecosystems simply to move a file from one screen to another.

TOVI removes that friction.

The core interaction is:

**Scan → Connect → Drop.**

The first-class transport is local/direct whenever technically possible. Internet connectivity is not required for local transfers. Cloud storage is optional rather than foundational.

TOVI should abstract networking complexity away from the user. The user should not need to understand IP addresses, ports, LANs, mDNS, NAT, Wi-Fi Direct, Wi-Fi Aware, or relay servers.

---

# 2. Product Vision

## Vision

Create the simplest universal bridge between a person's devices.

## Product promise

> **Anything on one device should be able to reach another device without needing to pass through a third party.**

## Long-term vision

TOVI becomes a personal device layer rather than merely a file transfer utility.

The eventual product should handle:

- files
- photos
- videos
- documents
- links
- text
- clipboard
- screenshots
- folders
- notifications
- app handoffs
- device-to-device streaming
- optional remote transfers
- optional encrypted cloud storage

---

# 3. Problem Statement

Users increasingly own multiple operating systems.

A developer can simultaneously have:

- Android phone
- iPhone
- MacBook
- Windows PC
- Linux workstation
- tablet

Each ecosystem has different transfer mechanisms.

The underlying problem is not "how do I upload a file?"

The actual problem is:

> **How do I move information from one of my devices to another with the least possible friction?**

Existing approaches create one or more problems:

| Method | Major friction |
|---|---|
| WhatsApp/Telegram | Upload/download overhead, compression, dependency on service |
| Email | Poor for large files and repeated transfers |
| Google Drive/OneDrive | Requires cloud upload and download |
| USB | Cable/device compatibility and physical friction |
| AirDrop | Primarily Apple ecosystem |
| Quick Share | Stronger within supported ecosystems |
| Temporary transfer sites | Internet dependency and privacy concerns |
| SMB/network shares | Powerful but too technical for ordinary users |

TOVI should make the underlying network effectively invisible.

---

# 4. Product Thesis

TOVI is built around five principles.

### 1. Local first

When devices can communicate locally, data should stay local.

### 2. User-owned infrastructure

The user's devices should do the work whenever possible.

### 3. Scan instead of configure

QR pairing should replace IP addresses, ports and manual setup.

### 4. Transport abstraction

The user should not care whether TOVI uses LAN, Wi-Fi Direct, Wi-Fi Aware, or a remote relay.

### 5. Cloud is optional

Cloud storage should enhance the product rather than become a requirement.

---

# 5. Target Users

## Primary segment: Cross-platform power users

Developers, designers, creators, students, engineers, researchers and professionals who regularly move files between different devices.

### Example

A developer has:

- Android phone
- MacBook
- Linux desktop

They take a screenshot on Android and want it on Linux.

Current workflow:

Screenshot → WhatsApp → open WhatsApp → download → locate file.

TOVI:

Screenshot → Share → TOVI → MacBook/Linux → Done.

---

## Secondary segment: Creators

Photographers, video editors, social media managers and content creators moving:

- photos
- recordings
- videos
- project assets
- scripts
- thumbnails
- documents

---

## Tertiary segment: Teams

Small teams working in environments where local transfer is useful:

- production
- events
- media
- engineering labs
- classrooms
- studios
- offices
- field environments

This creates a possible future B2B product.

---

# 6. Jobs To Be Done

### Functional JTBD

> When I have something on one device and need it on another, I want to transfer it directly with almost no setup.

### Emotional JTBD

> I want the transfer to feel effortless and trustworthy.

### Privacy JTBD

> I want my files to move between my devices without unnecessarily passing through someone else's servers.

---

# 7. Product Positioning

## Category

**Personal Device Bridge**

Rather than positioning TOVI as:

> "Another file transfer application"

position it as:

> **The bridge between your devices.**

That allows the product to grow beyond file transfer.

---

# 8. Core Experience

The fundamental experience:

```text
OPEN TOVI
      ↓
SEE AVAILABLE DEVICES
      ↓
SCAN / TAP DEVICE
      ↓
CONNECTED
      ↓
DROP ANYTHING
      ↓
TRANSFER DIRECTLY
```

Alternative:

```text
Laptop → Show QR
           ↓
Phone → Scan
           ↓
Secure pairing
           ↓
Direct connection
           ↓
Transfer
```

---

# 9. Connection Philosophy

An important technical/product distinction:

**QR is not itself the data transport.**

QR is the bootstrap mechanism.

A QR code can tell TOVI:

- which device is advertising
- where its transfer endpoint is
- what protocol version it supports
- how to establish the session
- what temporary authentication material to use

The actual data then travels over a suitable transport.

Conceptually:

```text
QR
 ↓
Bootstrap
 ↓
Discovery / Authentication
 ↓
Transport selection
 ↓
Encrypted channel
 ↓
Transfer
```

---

# 10. Connection Modes

TOVI should have a connection manager that automatically selects the best available transport.

## Mode A — Local LAN

Phone and computer are connected to the same local network.

Example:

```text
Phone
  │
  │ local traffic
  │
Starlink Router
  │
  │ local traffic
  │
MacBook
```

No cloud is involved.

This is the primary MVP path.

Starlink's current documentation explicitly states that devices on its **Default** network can see and communicate with each other, while Guest networks isolate clients. That makes a normal Starlink Default network a valid local-transfer environment.

---

## Mode B — QR-assisted LAN connection

When automatic discovery fails:

```text
Laptop
  │
  │ generates QR
  ↓
QR
  │
  │ scan
  ↓
Phone
  │
  │ direct local connection
  ↓
Laptop
```

This is still a local transfer.

---

## Mode C — Direct peer-to-peer Wi-Fi

Where platform support permits, TOVI may establish a peer-to-peer Wi-Fi link without an access point.

Android provides Wi-Fi Direct APIs specifically for discovering and communicating with nearby devices without requiring a network or hotspot.

---

## Mode D — Wi-Fi Aware

On supported Apple hardware/software, Wi-Fi Aware can establish secure, peer-to-peer communication without an internet connection or access point. Apple's current documentation says iPhone 12 and later support the framework in iOS 26.

This is an important future transport for the "direct connection" vision.

---

## Mode E — Remote transfer

If devices cannot establish a local/direct connection and the user enables remote mode:

```text
Phone
  ↓
Internet
  ↓
Encrypted relay
  ↓
Internet
  ↓
Laptop
```

The relay should ideally handle encrypted packets rather than become the application's default storage layer.

---

## Mode F — Optional cloud

Cloud becomes a separate feature.

Possible uses:

- device unavailable
- asynchronous transfer
- backup
- remote file access
- history
- multi-device synchronization

---

# 11. MVP Scope

## MVP goal

Prove one thing:

> **TOVI can make cross-platform local transfer feel easier than WhatsApp.**

### Platforms

Recommended initial build:

**Desktop**
- macOS
- Windows

**Mobile**
- Android

Then:

**Phase 1.5**
- Linux

**Phase 2**
- iOS

The reason for sequencing is technical rather than market-driven: iOS has stricter platform networking constraints and its direct peer-to-peer capabilities require native integration.

---

# 12. MVP Features

## P0 — Device discovery

Devices on the same local network automatically discover TOVI instances.

Requirements:

- device name
- device type
- operating system
- online status
- approximate connection type
- local endpoint
- capability metadata

Example:

```text
Available Devices

🟢 Obioma's MacBook
   macOS
   Local connection

🟢 ThinkPad Linux
   Linux
   Local connection
```

---

## P0 — QR pairing

Desktop:

```text
Connect a phone

[ QR CODE ]

Scan this code with TOVI

Expires in 60 seconds
```

Phone:

```text
Connect device

[ Scan QR ]

Camera opens
```

---

## P0 — Secure device trust

First connection:

```text
MacBook Pro wants to connect.

Device:
Obioma's MacBook

Network:
Local

Encryption:
Enabled

[ Allow ]
[ Cancel ]
```

User may mark the device trusted.

---

## P0 — File transfer

Supported:

- images
- video
- audio
- PDF
- DOCX
- ZIP
- APK
- arbitrary files

Requirements:

- progress
- speed
- file size
- pause/cancel
- resume
- success
- failure
- checksum verification

---

## P0 — Drag and drop

Desktop:

```text
DROP FILES HERE
```

---

## P0 — Transfer history

```text
Today

✓ IMG_2048.PNG
  Android → MacBook
  8.4 MB

✓ project.zip
  MacBook → Android
  1.2 GB
```

History is local.

---

## P0 — Receive folder

User chooses:

```text
Downloads/TOVI
```

or a custom folder.

---

## P0 — Offline operation

Local mode must continue to work when internet connectivity is unavailable.

---

# 13. P1 Features

- folders
- multi-file batch transfers
- clipboard transfer
- text transfer
- link transfer
- share-sheet integration
- contextual OS integration
- auto-accept trusted devices
- transfer queue
- file preview
- QR-only fallback
- device aliases
- pairing management
- notifications
- transfer retry
- resumable transfers
- hotspot/direct-mode guidance

---

# 14. P2 Features

- iOS
- Linux
- Wi-Fi Direct
- Wi-Fi Aware
- remote transfer
- encrypted relay
- cloud storage
- encrypted personal vault
- device synchronization
- clipboard sync
- media streaming
- remote file browsing

---

# 15. P3 — Long-term Platform

TOVI evolves from:

**File transfer**

into:

**Personal device infrastructure**

Potential capabilities:

```text
Phone
 ↕
TOVI
 ↕
Laptop
 ↕
Tablet
 ↕
Desktop
```

Shared:

- files
- clipboard
- links
- notifications
- media
- device state
- app handoffs

---

# 16. User Flows

## Flow A — First-time setup

```text
Install TOVI
      ↓
Choose device name
      ↓
Grant required network permissions
      ↓
TOVI starts local discovery
      ↓
Device ready
```

No mandatory account.

---

## Flow B — Phone → Laptop

```text
Open image
    ↓
Share
    ↓
TOVI
    ↓
Choose device
    ↓
Transfer
    ↓
Saved locally
```

---

## Flow C — Laptop → Phone

```text
Drag file into TOVI
       ↓
Select phone
       ↓
Phone receives prompt
       ↓
Accept
       ↓
Transfer
       ↓
File saved
```

---

## Flow D — QR pairing

```text
Laptop
 ↓
Connect Phone
 ↓
Generate QR
 ↓
Phone scans
 ↓
Session validation
 ↓
Secure handshake
 ↓
Device paired
 ↓
Transfer
```

---

## Flow E — No internet

```text
Internet OFF
     ↓
Local network remains
     ↓
TOVI detects peer
     ↓
Transfer locally
```

The UI should explicitly say:

> **No internet required.**

---

## Flow F — Devices are on Starlink

```text
Phone ─────┐
           │
           ↓
     Starlink Router
           ↑
           │
Laptop ────┘
```

TOVI discovers the devices over the LAN.

The user should never need to know that this is happening.

If discovery is blocked, QR pairing should provide a fallback.

---

# 17. Failure Flows

## Device unavailable

> We can't reach this device right now.

Actions:

**Retry**  
**Show QR connection**

---

## Network isolation

> Your devices are online but cannot communicate locally.

Actions:

**Try Direct Connection**  
**Connect to same network**  
**Use Remote Transfer**

---

## Transfer interrupted

```text
Transfer interrupted.

1.34 GB / 2.10 GB transferred.

[ Resume ]
[ Restart ]
[ Cancel ]
```

---

## Destination unavailable

TOVI keeps the transfer in a local queue.

```text
Waiting for Obioma's MacBook

[ Keep waiting ]
[ Cancel ]
```

---

# 18. Security Requirements

Security is a product requirement, not an infrastructure afterthought.

## No mandatory account

Local mode should work without:

- email
- phone number
- password
- cloud account

---

## Device identity

Each device generates a long-term cryptographic identity.

Recommended:

- Ed25519 identity key
- X25519 ephemeral key agreement
- TLS 1.3 / QUIC transport

---

## Pairing

Each pairing generates:

- device identity
- trusted relationship
- cryptographic binding

QR pairing should use short-lived sessions.

---

## Session security

Conceptually:

```text
Discovery
   ↓
Identity verification
   ↓
Ephemeral key exchange
   ↓
Authenticated encrypted channel
   ↓
Transfer
```

---

## File integrity

Every transferred file should be verified using a cryptographic hash.

Recommended:

**SHA-256**

For extremely large transfers, chunk-level hashes may also be used.

---

## Trust controls

Users can:

- revoke device
- forget device
- require approval every time
- automatically accept trusted devices
- disable clipboard sharing
- disable background transfers

---

# 19. Privacy Model

### Local mode

Default expectation:

> **Payload never leaves the local/direct connection.**

### Remote mode

User explicitly enables it.

The UI must say:

> **This transfer will use the internet.**

### Cloud mode

User explicitly activates cloud storage.

---

# 20. Analytics Philosophy

Because this is a privacy-first product, TOVI should avoid turning the application into spyware.

Default telemetry should be minimal.

Possible anonymous metrics:

- app version
- platform
- connection mode
- transfer success/failure
- aggregate transfer duration
- crash diagnostics

Never collect file contents.

Never collect:

- filenames by default
- file previews
- file hashes for analytics
- clipboard contents
- personal documents

---

# 21. Product Metrics

## North Star Metric

**Successful device-to-device transfers completed with ≤3 user actions after pairing.**

Supporting metrics:

### Activation

Percentage of installed users completing their first transfer.

### Time to first transfer

Install → first successful transfer.

### Connection success

Discovery/pairing success rate.

### Transfer reliability

Percentage of transfers completed without restart.

### Repeat usage

Transfers per active device pair per week.

### Local transfer ratio

Percentage of all transfers completed without internet.

### Pairing persistence

Percentage of paired devices still actively used after 30 days.

---

# 22. Product Success Criteria

MVP should demonstrate:

- <30 seconds from opening app to first transfer for already-discovered devices
- QR pairing that completes in seconds
- reliable transfers of large files
- automatic reconnect after temporary interruption
- local mode works with internet disconnected
- cross-platform transfer does not require an account
- users understand the basic experience without technical instructions

---

# 23. Non-Goals for MVP

Do not initially build:

- social sharing
- public file links
- team storage
- collaboration workspaces
- complex account systems
- large cloud storage backend
- media editing
- file synchronization engine
- enterprise admin

TOVI wins first by being extremely good at one thing.

---

# 24. Product Principles

### Invisible complexity

Users should see:

> Connect

not:

> Discover mDNS service on 192.168.x.x.

### Local by default

Try local first.

### Fast by default

Optimize the transfer path, not the dashboard.

### Trust before convenience

Device trust is explicit.

### One action beats five

The ideal transfer experience should feel almost instantaneous.

---

# 25. Brand Strategy

## Brand name

**TOVI**

Pronunciation:

**TOH-vee**

The name is intentionally abstract enough to support expansion beyond file transfer.

It should not lock the company into "files" because the eventual product is a device bridge.

---

# 26. Brand Positioning

### Category

Personal device bridge

### Positioning statement

> TOVI is the private bridge between your devices, moving files and information directly whenever possible — without cloud upload, cables, or ecosystem lock-in.

---

# 27. Brand Promise

> **What you own should move with you.**

---

# 28. Tagline

Primary:

> **Move without the middleman.**

Alternative product messaging:

> **Scan. Connect. Move.**

> **Your devices. Directly.**

> **Nothing in the middle.**

---

# 29. Brand Personality

TOVI should feel:

- intelligent
- calm
- fast
- modern
- trustworthy
- slightly playful
- technically sophisticated without looking complicated

Avoid:

- overly cyberpunk aesthetics
- enterprise security clichés
- generic blue SaaS visuals
- cluttered dashboards
- overly technical terminology

---

# 30. Visual Direction

KARMA's existing premium/minimal direction translates well here.

### Base

- off-white
- graphite
- near-black
- soft gray

### Accent

One high-energy digital accent, potentially electric violet or cyan.

### UI

Large whitespace.

Large device icons.

Strong typography.

Very few controls.

---

# 31. Logo Concept

Recommended mark:

Two independent points connected by a subtle bridge.

Conceptually:

```text
● ──── ◇ ──── ●
```

or:

```text
●  ↔  ●
```

The negative space between the two nodes forms a stylized "T".

The identity should communicate:

**two devices → one connection**

rather than:

**cloud → devices**

---

# 32. Brand Language

Instead of:

> Uploading

say:

> **Sending**

Instead of:

> Server

say:

> **Device**

Instead of:

> Cloud sync

say:

> **Direct transfer**

Instead of:

> Pairing token

say:

> **Connection code**

The product can remain technically sophisticated underneath without forcing that complexity onto the user.

---

# 33. Launch Narrative

The strongest launch story isn't:

> "We built a file transfer app."

It is:

> **Why are we still sending files to ourselves through WhatsApp?**

Then demonstrate:

```text
Phone
 ↓
Scan
 ↓
Laptop
 ↓
1.8 GB file
 ↓
Done
```

Then:

> No upload.  
> No cloud.  
> No cable.  
> Just your devices.

---

# 34. Example Homepage

## Hero

**Move without the middleman.**

TOVI connects your phone, laptop and desktop directly so your files can move where you need them — without unnecessary cloud uploads.

**Scan. Connect. Move.**

[Get TOVI]

---

# 35. Product Expansion Strategy

### Phase 1

Transfer anything.

### Phase 2

Transfer information.

- links
- text
- clipboard
- screenshots

### Phase 3

Connect workflows.

- notifications
- media
- shared folders
- remote access

### Phase 4

Become the device layer.

TOVI becomes infrastructure that other applications can use.

Possible future developer product:

**TOVI Connect API / SDK**

Apps could invoke:

```text
Send this asset to my Mac.
```

This turns TOVI into potentially more than a consumer utility.

---

# 36. MVP Release Definition

Release MVP when:

1. Android can discover macOS/Windows TOVI instances.
2. QR pairing works reliably.
3. Files can transfer in both directions.
4. Local transfer works without internet.
5. Transfer integrity is verified.
6. Trusted device relationships persist locally.
7. Interrupted transfers can resume.
8. No mandatory account exists.
9. Security model has been reviewed.
10. Product can be explained in one sentence.

That one sentence:

> **TOVI lets your devices move files directly, starting with a simple QR scan.**