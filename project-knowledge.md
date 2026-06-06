# Project Knowledge: borehole

Internal notes for the agent. Keep concise (<= 1000 lines). English only.

## Overview

`borehole` is a self-hosted reverse-tunnel tool (think ngrok-like) organized as
a Cargo workspace with three crates: `common` (wire protocol), `server`
(`borehole-server`, TLS control plane) and `cli` (`borehole`). All three are
implemented and the workspace builds/tests end to end.

## Structure

Workspace members (declared in root `Cargo.toml`):

- `crates/common` — shared library crate. `lib.rs` re-exports the `proto`
  module. Depends on `serde`, `serde_json` and `uuid` (all workspace). Pure
  library, no binaries.
- `crates/cli` — command-line interface binary `borehole`. `main` (clap, async,
  returns `anyhow::Result`) parses a `Cli` enum: `config { --server, --token }`
  (no flags -> interactive stdin wizard; loads `config::load().unwrap_or_default`,
  overwrites provided fields, `save`s, prints "✓ Config saved") and `start`
  with a `StartCmd` subcommand (`tcp`/`http`, positional `local_port: u16`,
  `--remote-port: Option<u16>`) that loads the config and calls `tunnel::run`.
  Modules:
  - `config`: `BoreholeConfig { server_addr, token }` (Serialize/Deserialize/
    Debug/Clone/Default; Default = empty strings). `config_path()` ->
    `~/.borehole.json` (via `dirs::home_dir()` or `$HOME`, panics otherwise).
    `load() -> anyhow::Result<BoreholeConfig>` (friendly "No config found. Run
    `borehole config` first." on missing file). `save(&cfg)` writes pretty JSON.
    `normalize_server_addr(addr)` appends `:DEFAULT_SERVER_PORT` (7000) when the
    host has no `:` (no bracketless IPv6 support). Used by the `config` command.
  - `tunnel`: `run(cfg, protocol, local_port, remote_port)` builds a client
    `TlsConnector` (system roots via `webpki_roots::TLS_SERVER_ROOTS` +
    `RootCertStore::extend`), TLS-connects to `cfg.server_addr` (ServerName =
    host before `:`), sends `Register`, reads the reply (`Error` -> `anyhow`
    err; `Registered` -> assigned port), prints a colored banner, then loops on
    `NewConn` spawning `serve_conn` (fresh TLS conn + `DataConn` +
    `copy_bidirectional` to `127.0.0.1:{local_port}`). Any other message or read
    error ends the loop with `Ok(())`. NOTE: writes are explicitly `flush`ed
    (TLS buffering). `run` first calls `ensure_local_service(local_port)`
    (TCP connect to `127.0.0.1:port`) and aborts if nothing is listening.
    `check_server(cfg)` does a `Ping`/`Pong` round-trip (TLS + token) and is
    used by `borehole config` (failure is reported as a non-fatal warning).
  Depends on `common` (path) plus `tokio`, `tokio-rustls`, `rustls`, `serde`,
  `serde_json`, `clap`, `owo-colors`, `anyhow`, `webpki-roots` (workspace) and
  `dirs` (direct, cli-only). No `uuid` dependency.
- `crates/server` — server binary `borehole-server`. `main` loads a JSON
  `Config` { `bind_control: String`, `port_range: [u16; 2]`, `tokens:
  Vec<String>`, `tls: { cert, key }` }, builds a `TlsAcceptor` (rustls
  `ServerConfig` + `with_single_cert`, certs/keys via
  `pki_types::pem::PemObject::from_pem_file` — the `PemObject` trait MUST be
  imported), then accepts control connections, TLS-handshakes each on its own
  task and calls `handle_control`.   Config path is argv[1] or
  `/etc/borehole/config.json`. Fatal errors -> `eprintln!` + `exit(1)`.
  GOTCHA (fixed): `build_tls_acceptor` must use
  `CertificateDer::pem_file_iter` + collect, NOT `from_pem_file` (first block
  only). LE `fullchain.pem` has leaf + intermediate; serving only the leaf
  makes rustls clients fail with `UnknownIssuer` / server logs `UnknownCA`.
  Modules:
  - `control`: TLS control plane + token validation. `ServerState` (shared,
    `Arc`-wrapped) holds `tokens: HashSet<String>`, `port_mgr:
    Mutex<PortManager>` and `pending_conns: Mutex<HashMap<String,
    oneshot::Sender<TcpStream>>>` (conn_id -> channel handing the data socket to
    the waiting handler). `ServerState::new(tokens, port_range) -> Arc<Self>`.
    Uses `std::sync::Mutex` (not tokio's).
    `handle_control(stream, state)`: reads one framed `ClientMsg`. On `Register`
    -> validate token, `acquire` port, reply `Registered`, bind
    `0.0.0.0:{port}`, then loop `accept()`ing visitors: each gets a `conn_id`
    (UUID v4), a `NewConn` notice, a `oneshot` channel stored in
    `pending_conns`, and a spawned task that awaits the data socket then calls
    `proxy::pipe`. On `DataConn` -> remove the `oneshot::Sender` for `conn_id`
    and `send` the raw socket (via `BufReader::into_inner`). Private helper
    `write_msg` encodes+writes a `ServerMsg`.
    Control and client data connections are TLS: `handle_control` takes
    `tokio_rustls::server::TlsStream<TcpStream>` and `pending_conns` stores
    `oneshot::Sender<TlsStream<TcpStream>>`. The external visitor (public port)
    is plain `TcpStream`.
    GOTCHA: `into_inner()` on the `BufReader` discards any bytes buffered past
    the `DataConn` line; fine as long as the client sends nothing before the
    socket becomes a raw pipe.
    NOTE: std `Mutex` guards are taken in single statements so they are never
    held across an `.await` (required since std guards are not `Send`).
  - `port_mgr`: `PortManager`, a `std`-only port pool. `new(range)` fills the
    pool; `acquire(Option<u16>)` reserves a specific or the lowest free port;
    `release(port)` returns it (no-op if not in use). Uses `BTreeSet` (free) +
    `HashSet` (in use). Not thread-safe: wrap in a `Mutex` when shared.
  - `proxy`: `pipe<A, B>(a, b)` (generic over `AsyncRead + AsyncWrite + Unpin`)
    splices two streams with `tokio::io::copy_bidirectional`, logging
    transferred bytes / errors via `eprintln!`. Generic so it can join the
    plain-TCP visitor with the TLS data stream. Errors logged, not propagated.
  Depends on `common` (path) plus `tokio`, `tokio-rustls`, `rustls`, `serde`,
  `serde_json`, `uuid` (workspace).

## Build configuration

- `resolver = "2"`.
- Shared dependencies live in `[workspace.dependencies]`: `anyhow`, `tokio`
  (full), `tokio-rustls`, `rustls`, `clap` (derive), `serde` (derive),
  `serde_json`, `uuid` (v4), `owo-colors`, `webpki-roots`. Member crates should
  inherit these via `dependency.workspace = true`.
- `[profile.release]`: `opt-level = 3`, `strip = "symbols"`, `lto = true`.

## Docker

- `docker/Dockerfile`: multi-stage build for `borehole-server`.
  - Builder: `rust:1.95-alpine`, target `x86_64-unknown-linux-musl`. Installs
    `musl-dev build-base pkgconfig openssl-dev cmake perl`. The cmake/perl/
    build-base trio is MANDATORY because `rustls 0.23` pulls `aws-lc-sys`
    (native, cmake-built); the spec's original `musl-dev/pkgconfig/openssl-dev`
    alone fails.
  - Runtime: `scratch` + the static binary + `ca-certificates.crt`. ~4.5 MB.
    `EXPOSE 7000` and `30000-40000`. ENTRYPOINT runs with
    `/etc/borehole/config.json`, so a config must be mounted there.
  - Build: `docker build -f docker/Dockerfile -t borehole-server .` (~2 min).
- `.dockerignore` excludes `target/`, `.git/`, docs and local `*.json` (keeps
  crate-local json) from the build context.
- `docker/docker-compose.yml`: VPS deployment. `build.context: ..` (repo root) +
  `dockerfile: docker/Dockerfile`, `restart: unless-stopped`, publishes
  `7000` and `30000-40000`, mounts `/etc/borehole:/etc/borehole:ro` (must hold
  `config.json`, `cert.pem`, `key.pem`), `RUST_LOG=info`. No `version:` key (v2
  compose). Validate with `docker compose config --quiet`.
  CAVEAT: `RUST_LOG` has NO effect yet — the server logs via `eprintln!`, not a
  `log`/`tracing` backend, so the env var is currently inert.
  CAVEAT: publishing the 30000-40000 range spawns one userland docker-proxy
  forward per port (~10k), which is slow to start and memory-heavy; consider
  `network_mode: host` on the VPS for production.
- `borehole.example.json` (repo root): sample server config for
  `/etc/borehole/config.json`. CAVEAT: it includes a `bind_http` key that the
  server's `Config` struct does NOT define; serde ignores unknown fields (no
  `deny_unknown_fields`), so it parses fine but `bind_http` is inert — there is
  no HTTP plane yet (the `http` protocol only changes the CLI banner URL).
- `.github/ISSUE_TEMPLATE/bug_report.md`: bug template. CAVEAT: it references
  `borehole --version`, but the clap command has no `version` set yet, so that
  flag is unsupported until `#[command(version)]` is added.

## Docs

- `README.md`: user-facing guide (English). Covers architecture, server via
  Docker/GHCR (`--network host` recommended), Let's Encrypt certs, CLI install
  via `install.sh`/`install.ps1`, config reference, building, release flow and
  troubleshooting. Keep in sync when behavior changes.

## Installers

- `install.sh` (POSIX sh, Linux/macOS) and `install.ps1` (Windows PowerShell)
  at the repo root install the `borehole` CLI from GitHub Releases so it runs
  as `borehole` (not `./borehole-...`). One-liners:
  `curl -fsSL .../install.sh | sh` and `irm .../install.ps1 | iex`.
  - Asset mapping: Linux x86_64 -> musl build; macOS -> per-arch darwin;
    Windows x86_64 -> `.exe`. Linux aarch64 is NOT published yet (errors out).
  - `install.sh` dir resolution: `BOREHOLE_INSTALL_DIR` override, else
    `/usr/local/bin` (writable / root / sudo), else `~/.local/bin`. Overrides:
    `BOREHOLE_REPO`, `BOREHOLE_VERSION`. Default repo `llinguini/borehole`.
  - GOTCHA: `resolve_install_dir` echoes "dir wrapper" consumed via
    `set -- $(...)`; the wrapper is `sudo` or empty (handled with `${2:-}`).

## CI/CD

- `.github/workflows/release.yml`: triggers on `v*` tags only. Three jobs:
  - `build-cli` (matrix, native builds, `fail-fast: false`, `contents: write`):
    builds ONLY the `cli` crate (`cargo build --release --locked -p cli
    --target <t>`) for 5 targets and each job uploads its OWN binary
    (`borehole-<target>`, `.exe` on Windows) straight to the Release via
    `softprops/action-gh-release` (idempotent: first job creates the Release,
    rest add assets). No separate `release` job and no upload/download-artifact:
    this stops a slow runner (macOS waiting for a worker) from blocking the
    other uploads. Native (not cross) on purpose:
    `aws-lc-sys` (via `rustls 0.23`) needs cmake/perl, and NASM on Windows
    (installed via `ilammy/setup-nasm`). musl target installs `musl-tools`.
    GOTCHA: the ubuntu->musl build of `aws-lc-sys` is the most fragile leg; if
    it ever fails it may need `TARGET_CC=musl-gcc` / a musl cross toolchain.
  - `docker`: pushes `borehole-server` to `ghcr.io/<owner>/borehole-server`
    (Packages tab) via `docker/metadata-action` (semver + `latest`) and
    `build-push-action` (context `.`, file `docker/Dockerfile`, gha cache).
    Auth uses the built-in `GITHUB_TOKEN` (`packages: write`); no extra secret.
    NOTE: the server is Docker-only — it is NOT shipped as a Release binary.
  - `release` (`needs: build-cli`, `contents: write`): downloads all artifacts
    (`merge-multiple: true`) and attaches them DIRECTLY (uncompressed) to the
    GitHub Release via `softprops/action-gh-release` with auto-generated notes.
  CAVEAT: the first GHCR push creates a private package; make it public in the
  repo's package settings if anonymous `docker pull` is desired.

## Conventions

- C++ Core Guidelines / Stroustrup style influence; English code comments,
  variables and identifiers.
- Max 100 chars/line (aim for 80). Break long expressions with the operator at
  the start of the new line.

## Protocol (`common::proto`)

- Wire format: newline-delimited JSON over TLS. Each message has a `"type"`
  discriminant field.
- `ClientMsg` (client -> server) is an internally-tagged enum
  (`#[serde(tag = "type", rename_all = "snake_case")]`):
  - `Register`: `token`, `protocol` ("tcp"|"http"), `local_port: u16`,
    `remote_port: Option<u16>` (null => server assigns a random port). Tag:
    `"register"`.
  - `DataConn`: `conn_id: String` (UUID v4 echoed from server's `NewConn`). Tag:
    `"data_conn"`.
  - `Ping`: `token`. Side-effect-free probe: server validates the token and
    replies `Pong` or `Error`, WITHOUT acquiring a port or opening a listener.
    Tag: `"ping"`. Used by `borehole config` to validate TLS + token.
- `ServerMsg` (server -> client) is an internally-tagged enum
  (`#[serde(tag = "type", rename_all = "snake_case")]`):
  - `Registered`: `remote_port: u16` (port assigned to the tunnel). Tag:
    `"registered"`.
  - `NewConn`: `conn_id: String` (UUID v4 for the incoming connection). Tag:
    `"new_conn"`.
  - `Pong`: unit variant, serializes as `{"type":"pong"}`. Success reply to a
    `Ping` (connectivity + token valid).
  - `Error(ServerError)`: `reason: String`. Tag forced to `"error"` via
    `#[serde(rename = "error")]` on the variant (otherwise it would be
    `"server_error"`).
- Framing helpers (generic, not tied to the enums):
  - `encode<T: Serialize>(&T) -> Result<String, serde_json::Error>`: JSON +
    trailing `'\n'`.
  - `decode<T: DeserializeOwned>(&str) -> Result<T, serde_json::Error>`: parse a
    single line (newline already stripped).

## Notes / gotchas

- The repository is not yet a git repo. All three members now exist, so a
  full-workspace `cargo build` / `cargo test` works without isolating members.
- The build sandbox cannot reach `index.crates.io`; running `cargo` that needs
  to fetch the registry requires running outside the sandbox (full filesystem
  permissions).
- `clippy` is not installed in the current toolchain (`rustup component add
  clippy` to enable it); plain `cargo build` is currently warning-free.
- MSRV: deps require Rust >= 1.85 (`getrandom 0.4` needs the `edition2024` Cargo
  feature). Local toolchain is 1.95. Do NOT pin Docker/CI to older Rust (1.78
  fails to even parse manifests).
- Cargo's default target dir here is OUTSIDE the repo
  (`/tmp/cursor-sandbox-cache/.../cargo-target`), so `./target/release/...` may
  not exist after a plain build. Use `CARGO_TARGET_DIR=./target cargo build ...`
  to place artifacts in the repo (the Docker build is unaffected: it builds
  inside `/app`).
