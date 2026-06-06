# borehole

Self-hosted reverse tunnel (an ngrok-like tool you fully control). Expose a
local TCP service to the public internet through a server you run on a VPS, over
an authenticated TLS control channel.

```
visitor ──► server:public_port ──TLS──► borehole CLI ──► 127.0.0.1:local_port
```

## Contents

- [How it works](#how-it-works)
- [Quick start](#quick-start)
  - [1. Server (Docker)](#1-server-docker)
  - [2. TLS certificates](#2-tls-certificates)
  - [3. Client (CLI)](#3-client-cli)
- [Configuration reference](#configuration-reference)
- [Building from source](#building-from-source)
- [Releases & CI](#releases--ci)
- [Troubleshooting](#troubleshooting)
- [License](#license)

## How it works

The project is a Cargo workspace with three crates:

- **`common`** — the wire protocol (newline-delimited JSON over TLS).
- **`server`** (`borehole-server`) — runs on your VPS. Validates tokens, assigns
  public ports and proxies visitor traffic to the connected client.
- **`cli`** (`borehole`) — runs on your machine. Registers a tunnel and forwards
  incoming connections to a local port.

When a visitor hits the public port, the server tells the client over the
control connection; the client opens a fresh TLS data connection and the two
streams are spliced together.

## Quick start

You need two things: the **server** running on a public host (with a TLS
certificate), and the **CLI** on the machine whose service you want to expose.

### 1. Server (Docker)

The server is distributed **only** as a Docker image, published to the GitHub
Container Registry:

```
ghcr.io/llinguini/borehole-server:latest
```

On your VPS, prepare the config directory (see
[Configuration reference](#configuration-reference) and
[TLS certificates](#2-tls-certificates)):

```bash
sudo mkdir -p /etc/borehole
# Put config.json, cert.pem and key.pem in /etc/borehole/
```

Then run it. On a Linux VPS, prefer host networking:

```bash
docker run -d \
  --name borehole-server \
  --restart unless-stopped \
  --network host \
  -v /etc/borehole:/etc/borehole:ro \
  ghcr.io/llinguini/borehole-server:latest
```

> **Why `--network host`?** Publishing the full `30000-40000` range with `-p`
> spawns ~10,000 `docker-proxy` processes (one per port), which is slow and can
> exhaust memory (OOM). Host networking avoids that. If you shrink `port_range`
> in `config.json` to something small, you can instead use
> `-p 7000:7000 -p 30000-30100:30000-30100`.

If the package is **private** (the default on first push), authenticate first:

```bash
echo <GITHUB_TOKEN_WITH_read:packages> | docker login ghcr.io -u <user> --password-stdin
```

#### Or with Docker Compose

A [`docker/docker-compose.yml`](docker/docker-compose.yml) is provided (it builds
from source). After placing the files in `/etc/borehole`:

```bash
docker compose -f docker/docker-compose.yml up -d
```

### 2. TLS certificates

The CLI verifies the server certificate against the system root CAs, so a
**self-signed certificate will be rejected**. Use a certificate from a public CA
(e.g. Let's Encrypt), which requires a domain pointing at your VPS.

```bash
# DNS A record: tunnel.example.com -> <VPS IP>
sudo systemctl stop nginx 2>/dev/null   # free port 80 if needed
sudo certbot certonly --standalone -d tunnel.example.com

sudo cp /etc/letsencrypt/live/tunnel.example.com/fullchain.pem /etc/borehole/cert.pem
sudo cp /etc/letsencrypt/live/tunnel.example.com/privkey.pem  /etc/borehole/key.pem
docker restart borehole-server
```

> Use **`fullchain.pem`** (leaf + intermediates), not Let's Encrypt's bare
> `cert.pem`, or clients fail the handshake with `UnknownIssuer`.
>
> Certificates expire after 90 days. After `certbot renew`, copy the files again
> and restart the container (or mount the letsencrypt live directory directly).

### 3. Client (CLI)

#### Install

Linux / macOS:

```bash
curl -fsSL https://raw.githubusercontent.com/llinguini/borehole/main/install.sh | sh
```

Windows (PowerShell):

```powershell
irm https://raw.githubusercontent.com/llinguini/borehole/main/install.ps1 | iex
```

The installer downloads the right binary from the latest GitHub Release and puts
`borehole` on your PATH. Overrides: `BOREHOLE_VERSION`, `BOREHOLE_INSTALL_DIR`,
`BOREHOLE_REPO`.

#### Configure

```bash
borehole config
# Server address (host:port): tunnel.example.com:7000   (port defaults to 7000)
# Token: <one of the server's tokens>
```

After saving, the CLI validates the connection (TLS + token) and reports `✓` or
a warning. Settings are stored at `~/.borehole.json`.

#### Expose a port

```bash
# Forward a local service on 127.0.0.1:8000 to a public port on the server
borehole start tcp 8000

# Request a specific public port
borehole start tcp 8000 --remote-port 35000
```

The CLI checks that something is actually listening on the local port before it
opens the tunnel. On success it prints the public address to share.

## Updating

Check your installed version with `borehole --version`. On `borehole start` the
CLI also warns when its version differs from the server's, and notifies you when
a newer release is available.

### CLI

```bash
borehole update
```

Downloads the latest matching binary from GitHub Releases and replaces the
running executable in place (Linux, macOS and Windows). Pin a version with
`BOREHOLE_VERSION=v0.1.2 borehole update`.

### Server

The server runs as an immutable Docker image, so it is updated by pulling the
new image and recreating the container:

```bash
docker pull ghcr.io/llinguini/borehole-server:latest
docker rm -f borehole-server
docker run -d \
  --name borehole-server \
  --restart unless-stopped \
  --network host \
  -v /etc/borehole:/etc/borehole:ro \
  ghcr.io/llinguini/borehole-server:latest
```

> Keep the server and CLI on the same version. The `Cargo.toml` version must
> match the release tag; the pipeline injects the tag at build time so published
> artifacts always report the right version.

## Configuration reference

### Server — `/etc/borehole/config.json`

See [`borehole.example.json`](borehole.example.json).

| Field          | Type       | Description                                       |
| -------------- | ---------- | ------------------------------------------------- |
| `bind_control` | string     | Control-plane listen address, e.g. `0.0.0.0:7000` |
| `port_range`   | `[u16,u16]`| Inclusive public port range for tunnels           |
| `tokens`       | string[]   | Valid authentication tokens                       |
| `tls.cert`     | string     | Path to the PEM certificate chain (`fullchain`)   |
| `tls.key`      | string     | Path to the PEM private key                       |

### Client — `~/.borehole.json`

| Field         | Type   | Description                                  |
| ------------- | ------ | -------------------------------------------- |
| `server_addr` | string | `host:port` of the server (port defaults 7000)|
| `token`       | string | Token presented to the server                |

## Building from source

Requires Rust >= 1.85 (some dependencies need the 2024 edition feature).

```bash
cargo build --release            # all crates
cargo build --release -p cli     # just the borehole CLI
cargo test --workspace           # run the test suite
```

The server Docker image is built with:

```bash
docker build -f docker/Dockerfile -t borehole-server .
```

## Releases & CI

Pushing a version tag triggers
[`.github/workflows/release.yml`](.github/workflows/release.yml):

```bash
git tag v0.1.0 && git push origin v0.1.0
```

It builds the CLI for Linux (gnu + musl), macOS (Intel + Apple Silicon) and
Windows, attaches the binaries to a **GitHub Release**, and builds and pushes the
`borehole-server` image to **GHCR**.

Make sure *Settings → Actions → General → Workflow permissions* is set to
"Read and write". The GHCR package is private on first push; make it public in
the package settings if you want anonymous `docker pull`.

## Troubleshooting

| Symptom                                   | Cause / fix                                                                 |
| ----------------------------------------- | -------------------------------------------------------------------------- |
| CLI: `invalid peer certificate: UnknownIssuer` | Server is not sending the full chain. Use `fullchain.pem` as `cert.pem`. |
| Server log: `UnknownCA`                   | Same as above, seen from the server side.                                  |
| CLI: `no local service is listening ...`  | Start your local service first, or use the right local port.               |
| `config` warns it cannot connect          | Wrong host/port, port 7000 firewalled, or invalid token.                   |
| Server killed / SSH drops on start        | OOM from publishing the huge port range with `-p`. Use `--network host`.    |
| `docker pull` denied                      | Private GHCR package: `docker login ghcr.io`, or make the package public.   |

## License

Distributed under the GNU Affero General Public License v3.0. See
[`LICENSE`](LICENSE).
