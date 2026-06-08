# borehole

Self-hosted TCP/HTTP reverse tunnel. An open-source alternative to ngrok: expose
a local port to the public internet through a server you run yourself, over an
authenticated TLS control channel.

## Quick start (30 seconds)

```sh
# 1. Install the client (Linux/macOS)
curl -fsSL https://raw.githubusercontent.com/llinguini/borehole/main/scripts/install.sh | sh

# 2. Point it at your server and authenticate
borehole config --server your-vps:7000 --token your-token

# 3. Expose local port 22 (SSH)
borehole start tcp 22
# ✓ Túnel activo
#   local  → localhost:22
#   remoto → your-vps:32847

# 4. Connect through the assigned remote port
ssh -p 32847 user@your-vps
```

The remote port is assigned from the server's pool. Use `--remote-port` to ask
for a fixed one.

## Installation

### Client (`borehole`)

**Linux / macOS** — installs to `/usr/local/bin` (falls back to `sudo` or
`~/.local/bin`):

```sh
curl -fsSL https://raw.githubusercontent.com/llinguini/borehole/main/scripts/install.sh | sh
```

**Windows** — installs to `%LOCALAPPDATA%\borehole` and adds it to your PATH:

```powershell
irm https://raw.githubusercontent.com/llinguini/borehole/main/scripts/install.ps1 | iex
```

**From source** (requires Rust ≥ 1.85):

```sh
cargo install --path crates/cli
```

### Server (`borehole-server`)

The server runs on a host with a public IP. It needs a config file and a TLS
certificate/key pair.

#### Docker (recommended)

```sh
# From the repo root, prepare the mounted files:
mkdir -p docker/config docker/certs

# 1. Generate a self-signed TLS certificate (see below)
openssl req -x509 -newkey rsa:4096 -nodes \
  -keyout docker/certs/key.pem -out docker/certs/cert.pem \
  -days 365 -subj "/CN=your-vps"

# 2. Write docker/config/config.json (see the minimal config below)

# 3. Build and run
docker compose -f docker/docker-compose.yml up --build -d
```

The compose file publishes `7000` (control TLS), `7001` (data connections) and
the tunnel range `30000-40000`, and mounts `docker/config` and `docker/certs`
into the container.

#### Direct binary

```sh
cargo install --path crates/server
borehole-server --config /etc/borehole/config.json
```

#### Minimal `/etc/borehole/config.json`

```json
{
  "bind_control": "0.0.0.0:7000",
  "bind_http": "0.0.0.0:8080",
  "port_range": [30000, 40000],
  "tokens": ["your-token"],
  "tls": {
    "cert": "certs/cert.pem",
    "key": "certs/key.pem"
  }
}
```

- `bind_control` — control plane (TLS). The data plane listens on the next port
  (`7001` here), so open both plus the `port_range` on your firewall.
- `tokens` — accepted client tokens; a client must present one to register.
- `tls.cert` / `tls.key` — PEM paths, relative to the working directory (under
  Docker that is `/etc/borehole`).

#### Generate a TLS certificate

A self-signed certificate is enough — the client does not verify the chain in
v1 (see [Roadmap](#roadmap)):

```sh
openssl req -x509 -newkey rsa:4096 -nodes \
  -keyout certs/key.pem -out certs/cert.pem \
  -days 365 -subj "/CN=your-vps"
```

## Usage

### `borehole config`

Stores the server address and token in `~/.borehole.json`.

```sh
# Non-interactive
borehole config --server your-vps:7000 --token your-token

# Interactive wizard (prompts for both)
borehole config
```

### `borehole start tcp <port> [--remote-port <port>]`

Exposes a local TCP port. The server assigns a random public port unless
`--remote-port` is given.

```sh
borehole start tcp 22                    # random remote port
borehole start tcp 5432 --remote-port 35432   # fixed remote port
```

### `borehole start http <port> [--remote-port <port>]`

Same as `tcp`, but prints a ready-to-use `http://` URL in the banner.

```sh
borehole start http 3000
# ✓ Túnel activo
#   url → http://your-vps:31180
```

Press `Ctrl+C` to close the tunnel. If the server drops unexpectedly the client
retries up to 3 times with a 3-second backoff before exiting.

## Use cases

**1. Remote SSH access to a machine without a public IP**

```sh
borehole start tcp 22 --remote-port 32222
# then, from anywhere:
ssh -p 32222 user@your-vps
```

**2. Share a local HTTP dev server**

```sh
python3 -m http.server 8080        # or your framework's dev server
borehole start http 8080
# open the printed http://your-vps:<port> URL
```

**3. Receive GitHub webhooks on localhost**

```sh
borehole start http 4000 --remote-port 34000
# set the GitHub webhook URL to: http://your-vps:34000/webhook
```

## Architecture

borehole has two planes. The **control plane** is one persistent TLS connection
from the client to the server (`:7000`); the server uses it to push
notifications when a visitor arrives. The **data plane** is a plain-TCP listener
on the control port + 1 (`:7001`): for each visitor the client opens a fresh
data connection, identifies it, and the server splices the visitor's socket to
it. The client in turn splices that to the local service.

```
CLI ──────TLS──────▶ server:7000   (control: register, notifications)
visitor ───────────▶ server:32847  (public tunnel port)
server ──notify───▶ CLI
CLI ──────TCP──────▶ server:7001   (data connection for that visitor)
server ──splice───▶ visitor ⇄ data ⇄ CLI ──▶ localhost:port
```

## Roadmap

- **v1**: TCP + HTTP tunnels, CLI, self-hosted server ✓
- **v2**: web dashboard, multi-node edges, JWT auth
- **v3**: ACME / Let's Encrypt certificates, metrics, rate limiting

## License

GNU AGPL-3.0. See [LICENSE](LICENSE).
