# TOVI

Move without the middleman: a cross-platform bridge for moving files directly between your own
devices. See [`docs/prd/PRD.md`](docs/prd/PRD.md) for the product, [`docs/architecture/`](docs/architecture/)
for the design and decisions, and [`docs/TASKS.md`](docs/TASKS.md) for progress.

## Repository layout

```text
core/tovi-core   Rust core: identity, QUIC transport, pairing, protocol
core/tovi-cli    Headless spike CLI
docs/            PRD, architecture, decisions, task list
```

## Build and test

Requires the stable Rust toolchain (on Windows, also the Visual Studio C++ Build Tools).

```bash
cargo test --workspace
cargo build -p tovi-cli
```

## Try pairing two machines

Both machines must be on the same network, for example the same home Wi-Fi.

On the first machine (the "desktop"):

```bash
cargo run -p tovi-cli -- listen
```

It prints a QR code and a `tovi://pair/...` link, valid for 60 seconds. On the second machine,
paste the link:

```bash
cargo run -p tovi-cli -- pair "tovi://pair/..."
```

The first machine asks `Allow? [y/N]`. Answer `y` and both sides report the pairing.

Notes:

- Windows asks whether `tovi-cli` may use the network the first time `listen` runs. Allow it on
  **private** networks, or the other machine can't connect.
- Each pairing code works once. Run `listen` again for a new one.
- `tovi-cli id` shows this device's ID and the addresses it can be reached at.
- To run two instances on one machine, give each its own identity: `--data-dir <folder>`.
- Development builds keep the identity key **unencrypted** in the OS data folder (`%APPDATA%\TOVI`
  on Windows). Trusted devices are not saved yet, so pairings last only while `listen` runs.
- File transfer is not built yet.
