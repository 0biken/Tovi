# TOVI

Move without the middleman: a cross-platform bridge for moving files directly between your own
devices. See [`docs/prd/PRD.md`](docs/prd/PRD.md) for the product, [`docs/architecture/`](docs/architecture/)
for the design and decisions, and [`docs/TASKS.md`](docs/TASKS.md) for progress.

## Repository layout

```text
core/tovi-core   Rust core: identity, QUIC transport, pairing, protocol
core/tovi-cli    Headless spike CLI
apps/desktop     Desktop app (Tauri 2 + React), work in progress
docs/            PRD, architecture, decisions, task list
```

## Build and test

Requires the stable Rust toolchain (on Windows, also the Visual Studio C++ Build Tools).

```bash
cargo test
cargo build -p tovi-cli
```

Plain `cargo` commands cover the core and the CLI. The desktop app is built separately (see below).

## Try it on two machines

Both machines must be on the same network, for example the same home Wi-Fi. Use `--release`
builds when measuring speed.

On the first machine (the "desktop"):

```bash
cargo run --release -p tovi-cli -- listen
```

It prints a QR code and a `tovi://pair/...` link, valid for 60 seconds. On the second machine,
send a file using that link:

```bash
cargo run --release -p tovi-cli -- send path/to/file "tovi://pair/..."
```

The first machine asks `Allow? [y/N]`. Answer `y`: the devices pair, the file is sent, and it is
saved in `Downloads/TOVI` (change with `listen --receive-dir <folder>`) once its BLAKE3 hash
matches. Both sides show progress and speed.

The pairing is saved on both machines, so after that just use the desktop's name:

```bash
cargo run --release -p tovi-cli -- send path/to/file "DESKTOP-NAME"
```

`listen` must be running on the desktop. To pair without sending anything, use
`pair "tovi://pair/..."`.

| Command | What it does |
|---|---|
| `id` | This device's name, ID and addresses |
| `listen` | Receive files; shows a QR code to pair a new device |
| `pair <link>` | Pair using a QR link |
| `send <file> <link or device>` | Send a file, pairing first if given a link |
| `devices` | Paired devices and where they were last seen |
| `forget <device>` | Stop trusting a device; it must pair again |
| `history` | Recent transfers, both directions |

Notes:

- Windows asks whether `tovi-cli` may use the network the first time `listen` runs. Allow it on
  **private** networks, or the other machine can't connect.
- Each pairing code works once. Run `listen` again for a new one.
- To run two instances on one machine, give each its own data folder: `--data-dir <folder>`.
- The identity key and the database (`tovi.db`: paired devices, history) live in the OS data
  folder (`%APPDATA%\TOVI` on Windows). Development builds keep the key **unencrypted**.
- A paired device is reached at its last known address. If that changes (for example a new
  address from the router), pair again; automatic discovery is not built yet.
- If the connection drops mid-transfer, `send` reconnects for up to 2 minutes and resumes where it
  left off. If it gives up, running the same `send` again resumes too. The partial file waits in
  the receive folder as `<name>.<id>.tovi.part` with a `.tovi.state` file beside it.

## Desktop app (work in progress)

Needs [Node.js](https://nodejs.org) 22+, [pnpm](https://pnpm.io), and on Linux the
[Tauri system libraries](https://v2.tauri.app/start/prerequisites/). The screens exist, but most
actions are still placeholders and are not connected to the core yet.

```bash
cd apps/desktop
pnpm install
pnpm tauri dev
```

To compile or lint the Rust side (`cargo clippy -p tovi-desktop`), build the frontend once first
with `pnpm build`; the Rust build embeds `apps/desktop/dist`.
