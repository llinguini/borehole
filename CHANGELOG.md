# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `.github/workflows/release.yml`: tag-triggered (`v*`) release pipeline.
  Builds the `borehole` CLI natively per target
  (`x86_64-unknown-linux-gnu`, `x86_64-unknown-linux-musl`,
  `aarch64-apple-darwin`, `x86_64-apple-darwin`,
  `x86_64-pc-windows-msvc`) and attaches each binary directly to the
  GitHub Release; builds and pushes the `borehole-server` Docker image to
  `ghcr.io/<owner>/borehole-server` (GitHub Packages) using the
  repository-scoped `GITHUB_TOKEN`.

- Initial Rust workspace `Cargo.toml` defining the `borehole` workspace with
  three members (`crates/common`, `crates/cli`, `crates/server`), shared
  `[workspace.dependencies]` and an optimized `[profile.release]`.
- Minimal scaffold for the `common` library crate (`serde` and `uuid` inherited
  from the workspace) exposing an empty `proto` module.
- Client-to-server protocol messages in `common::proto`: `Register` and
  `DataConn` structs wrapped by the internally-tagged `ClientMsg` enum
  (`#[serde(tag = "type", rename_all = "snake_case")]`), plus unit tests.
- Server-to-client protocol messages in `common::proto`: `Registered`,
  `NewConn` and `ServerError` structs wrapped by the internally-tagged
  `ServerMsg` enum (the `Error` variant serializes as `"type":"error"`), plus
  unit tests.
- Generic newline-delimited JSON framing helpers `common::proto::encode` and
  `decode`, with a round-trip test. `serde_json` is now a regular dependency of
  the `common` crate.
- Scaffold for the `server` crate: `borehole-server` binary with empty
  `#[tokio::main]` entry point and `control`, `port_mgr` and `proxy` module
  stubs. Depends on `common` plus the shared async/TLS dependencies.
- `server::port_mgr::PortManager`: a `std`-only TCP port pool with
  `new`/`acquire`/`release`, backed by a `BTreeSet` (free) and `HashSet` (in
  use), with unit tests. Not thread-safe by design (caller wraps in a `Mutex`).
- `server::proxy::pipe`: async bidirectional TCP splice via
  `tokio::io::copy_bidirectional`, logging bytes transferred or errors.
- `server::control::ServerState`: shared, `Arc`-wrapped server state holding the
  valid tokens, a `Mutex<PortManager>` and a `Mutex<HashMap>` of pending data
  connections (`conn_id` -> `oneshot::Sender<TcpStream>`), with a `new`
  constructor.
- `server::control::handle_control`: handles a control connection. On
  `Register` it validates the token, reserves a port, replies `Registered`,
  binds a public `TcpListener` and, per visitor, announces `NewConn` and waits
  (via a `oneshot` channel) for the matching client data connection to splice
  them with `proxy::pipe`. On `DataConn` it delivers the raw socket to the
  waiting handler.
- `borehole-server` `main`: loads a JSON `Config` (control bind address, port
  range, tokens, TLS cert/key paths), builds a `rustls`/`tokio-rustls`
  `TlsAcceptor`, and runs the accept loop that TLS-handshakes each connection
  and dispatches it to `handle_control`. Control and client data connections
  now flow over TLS (`TlsStream<TcpStream>`); `proxy::pipe` was made generic to
  splice the plain-TCP visitor with the TLS data stream.
- Scaffold for the `cli` crate: `borehole` binary with empty `#[tokio::main]`
  entry point and `config`/`tunnel` module stubs. With this, the full workspace
  builds end to end.
- `cli::config`: `BoreholeConfig` (`server_addr`, `token`) with
  `config_path`/`load`/`save` helpers persisting `~/.borehole.json` (pretty
  JSON). Added `anyhow` to the workspace dependencies and `anyhow` + `dirs` to
  the `cli` crate.
- `cli::tunnel::run`: establishes the TLS control connection (system roots via
  `webpki-roots`), registers the tunnel, prints a colored success banner
  (`owo-colors`) and, per `NewConn`, spawns a task that opens a fresh TLS data
  connection (`DataConn`) and splices it to the local service with
  `copy_bidirectional`. Added `webpki-roots` to the workspace and `cli`
  dependencies.
- `borehole` CLI `main` (clap): `config` subcommand (flags or interactive
  wizard, persists `~/.borehole.json`) and `start tcp|http <local_port>
  [--remote-port]` which loads the config and runs `tunnel::run`. The cli is now
  feature-complete end to end.
- `docker/Dockerfile`: multi-stage musl build producing a ~4.5 MB `scratch`
  image for `borehole-server` (static binary + CA certs, `EXPOSE 7000` and
  `30000-40000`). Plus a `.dockerignore` to keep the build context clean.
- `docker/docker-compose.yml`: deploys `borehole-server` on a VPS (builds from
  the repo root, `restart: unless-stopped`, publishes 7000 and 30000-40000,
  mounts `/etc/borehole` read-only, sets `RUST_LOG=info`).
- `borehole.example.json`: sample server config to copy to
  `/etc/borehole/config.json`.
- `.github/ISSUE_TEMPLATE/bug_report.md`: minimal bug report template
  (description, steps, version, OS).
