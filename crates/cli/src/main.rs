// borehole CLI entry point.
//
// Commands:
//   config  — set the server/token (and dashboard URL) for this machine
//   login   — authenticate against the web dashboard via a browser callback
//   start   — open a TCP/HTTP tunnel (foreground, or background with --detach)
//   list    — show background tunnels
//   stop    — stop a background tunnel
//
// Manual end-to-end test (foreground tunnel):
//
//   cargo run -p borehole-server -- --config borehole.example.json
//   python3 -m http.server 8080
//   cargo run -p borehole -- config --server 127.0.0.1:7000 --token dev-token-1234
//   cargo run -p borehole -- start http 8080
//   curl http://localhost:<assigned_port>

mod config;
mod tunnel;
mod tunnels;

use std::io::Write;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use owo_colors::OwoColorize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use borehole_common::proto::Protocol;
use config::CliConfig;
use tunnels::{TunnelRecord, STATE_ACTIVE, STATE_STARTING};

#[derive(Parser)]
#[command(name = "borehole", about = "Self-hosted reverse tunnel")]
enum Cli {
    /// Configure the server, access token and dashboard URL.
    Config {
        #[arg(long)]
        server: Option<String>,
        #[arg(long)]
        token: Option<String>,
        #[arg(long)]
        url: Option<String>,
    },
    /// Authenticate against the web dashboard.
    Login,
    /// Start a tunnel.
    Start {
        #[command(subcommand)]
        protocol: StartCmd,
        /// Run the tunnel in the background instead of the foreground.
        #[arg(long)]
        detach: bool,
        /// Preferred edge node to host the tunnel.
        #[arg(long)]
        node: Option<String>,
    },
    /// List background tunnels.
    List,
    /// Stop a background tunnel.
    Stop { tunnel_id: String },
    /// Internal: serve a single background tunnel (spawned by `start --detach`).
    #[command(name = "_tunnel-worker", hide = true)]
    TunnelWorker { id: String },
}

#[derive(Subcommand)]
enum StartCmd {
    Tcp {
        port: u16,
        #[arg(long)]
        remote_port: Option<u16>,
    },
    Http {
        port: u16,
        #[arg(long)]
        remote_port: Option<u16>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Keep the banner clean: only surface warnings unless RUST_LOG overrides it.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    match Cli::parse() {
        Cli::Config { server, token, url } => run_config(server, token, url),
        Cli::Login => run_login().await,
        Cli::Start {
            protocol,
            detach,
            node,
        } => run_start(protocol, detach, node).await,
        Cli::List => run_list(),
        Cli::Stop { tunnel_id } => run_stop(&tunnel_id),
        Cli::TunnelWorker { id } => run_worker(id).await,
    }
}

/// Handles `borehole config`: direct save when any flag is given, interactive
/// wizard (server + token) when none are.
fn run_config(server: Option<String>, token: Option<String>, url: Option<String>) -> Result<()> {
    let mut cfg = CliConfig::load()?;

    if server.is_none() && token.is_none() && url.is_none() {
        cfg.server_addr = Some(prompt("Server address (host:port): ")?);
        cfg.token = Some(prompt("Token: ")?);
    } else {
        if let Some(server) = server {
            cfg.server_addr = Some(server);
        }
        if let Some(token) = token {
            cfg.token = Some(token);
        }
        if let Some(url) = url {
            cfg.server_url = Some(url);
        }
    }

    cfg.save()?;
    println!("{} Config saved to ~/.borehole.json", "✓".green());
    Ok(())
}

/// Handles `borehole start`: foreground (with reconnection) or, with `--detach`,
/// spawns a background worker and returns immediately.
async fn run_start(cmd: StartCmd, detach: bool, node: Option<String>) -> Result<()> {
    let (local_port, remote_port, protocol) = match cmd {
        StartCmd::Tcp { port, remote_port } => (port, remote_port, Protocol::Tcp),
        StartCmd::Http { port, remote_port } => (port, remote_port, Protocol::Http),
    };

    if detach {
        return detach_tunnel(protocol, local_port, remote_port, node);
    }

    run_tunnel_loop(local_port, remote_port, protocol, node, None).await
}

/// Runs a tunnel, retrying up to 3 times with a 3s backoff on unexpected
/// disconnects. A clean Ctrl+C shutdown returns `Ok`. When `ready_tx` is set,
/// the assigned remote port is reported through it on the first registration.
async fn run_tunnel_loop(
    local_port: u16,
    remote_port: Option<u16>,
    protocol: Protocol,
    node: Option<String>,
    mut ready_tx: Option<tokio::sync::oneshot::Sender<u16>>,
) -> Result<()> {
    let mut attempts = 0;
    loop {
        let result = tunnel::run(
            local_port,
            remote_port,
            protocol.clone(),
            node.clone(),
            ready_tx.take(),
        )
        .await;
        match result {
            Ok(()) => break,
            Err(e) if attempts < 3 => {
                eprintln!("{}", format!("✗ Desconectado: {e}. Reconectando en 3s...").red());
                tokio::time::sleep(Duration::from_secs(3)).await;
                attempts += 1;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Writes a `starting` record and spawns a detached worker process to serve it.
fn detach_tunnel(
    protocol: Protocol,
    local_port: u16,
    remote_port: Option<u16>,
    node: Option<String>,
) -> Result<()> {
    let id = Uuid::new_v4().simple().to_string()[..8].to_string();
    let record = TunnelRecord {
        id: id.clone(),
        pid: 0,
        protocol: protocol_name(&protocol).to_string(),
        local_port,
        remote_port,
        node,
        state: STATE_STARTING.to_string(),
    };
    record.save()?;

    let exe = std::env::current_exe().context("cannot locate the borehole executable")?;
    std::process::Command::new(exe)
        .arg("_tunnel-worker")
        .arg(&id)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to spawn background worker")?;

    println!("{} Túnel iniciado en background [{}]", "✓".green(), id);
    Ok(())
}

/// Background worker: serves one tunnel until stopped, keeping its record in
/// sync and cleaning it up on exit.
async fn run_worker(id: String) -> Result<()> {
    let mut record = TunnelRecord::load(&id)?;
    record.pid = std::process::id();
    record.state = STATE_STARTING.to_string();
    record.save()?;

    let protocol = match record.protocol.as_str() {
        "http" => Protocol::Http,
        _ => Protocol::Tcp,
    };
    let local_port = record.local_port;
    let remote_port = record.remote_port;
    let node = record.node.clone();

    // When the server assigns a port, flip the record to `active`.
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<u16>();
    {
        let id = id.clone();
        tokio::spawn(async move {
            if let Ok(port) = ready_rx.await {
                if let Ok(mut r) = TunnelRecord::load(&id) {
                    r.remote_port = Some(port);
                    r.state = STATE_ACTIVE.to_string();
                    let _ = r.save();
                }
            }
        });
    }

    tokio::select! {
        _ = shutdown_signal() => {}
        _ = run_tunnel_loop(local_port, remote_port, protocol, node, Some(ready_tx)) => {}
    }

    let _ = TunnelRecord::remove(&id);
    Ok(())
}

/// Lists background tunnels as an aligned table.
fn run_list() -> Result<()> {
    let records = TunnelRecord::list()?;
    if records.is_empty() {
        println!("No hay túneles en background.");
        return Ok(());
    }

    println!("{:<11} {:<6} {:<8} {:<8} ESTADO", "ID", "PROTO", "LOCAL", "REMOTO");
    for r in records {
        let local = format!(":{}", r.local_port);
        let remote = r
            .remote_port
            .map(|p| format!(":{p}"))
            .unwrap_or_else(|| "-".to_string());
        println!(
            "{:<11} {:<6} {:<8} {:<8} {}",
            r.id, r.protocol, local, remote, r.state
        );
    }
    Ok(())
}

/// Stops a background tunnel: signals its worker and removes the record.
fn run_stop(id: &str) -> Result<()> {
    let record = TunnelRecord::load(id)?;
    if record.pid != 0 {
        kill_process(record.pid)?;
    }
    TunnelRecord::remove(id)?;
    println!("{} Túnel detenido", "✓".green());
    Ok(())
}

/// Authenticates against the dashboard: opens the browser to a callback URL and
/// waits for the local HTTP callback carrying the JWT.
async fn run_login() -> Result<()> {
    let mut cfg = CliConfig::load()?;
    let server_url = cfg.require_server_url()?;

    // Bind an ephemeral local port for the OAuth-style callback.
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("cannot bind a local callback port")?;
    let port = listener.local_addr()?.port();
    let callback = format!("http://localhost:{port}");
    let url = format!(
        "{}/api/auth/cli?callback={}",
        server_url.trim_end_matches('/'),
        callback
    );

    println!("Abriendo el navegador para autenticar...");
    println!("  {}", url.dimmed());
    if open::that(&url).is_err() {
        println!("No se pudo abrir el navegador automáticamente. Abre el enlace de arriba.");
    }

    // Wait (bounded) for the browser to hit our callback.
    let (mut stream, _) = tokio::time::timeout(Duration::from_secs(300), listener.accept())
        .await
        .context("timed out waiting for authentication")??;

    // Read only the HTTP request line; the query carries token + expiry.
    let request_line = {
        let mut reader = BufReader::new(&mut stream);
        let mut line = String::new();
        reader.read_line(&mut line).await?;
        line
    };

    let body = "OK, puedes cerrar esta pestaña.";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;

    let (token, expires) =
        parse_callback(&request_line).context("authentication callback carried no token")?;
    cfg.jwt = Some(token);
    cfg.jwt_expires_at = expires;
    cfg.save()?;

    println!("{} Autenticado correctamente", "✓".green());
    Ok(())
}

/// Parses the HTTP request line of the auth callback, extracting `token` and the
/// optional `expires` query parameters. Returns `None` if no token is present.
fn parse_callback(request_line: &str) -> Option<(String, Option<String>)> {
    let target = request_line.split_whitespace().nth(1)?;
    let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");

    let mut token = None;
    let mut expires = None;
    for pair in query.split('&') {
        if let Some((key, value)) = pair.split_once('=') {
            match key {
                "token" => token = Some(percent_decode(value)),
                "expires" => expires = Some(percent_decode(value)),
                _ => {}
            }
        }
    }
    token.map(|t| (t, expires))
}

/// Minimal `application/x-www-form-urlencoded` decoder (`%XX` and `+`).
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push(h * 16 + l);
                    i += 3;
                }
                _ => {
                    out.push(bytes[i]);
                    i += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Converts a single hex digit to its value.
fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Terminates a worker process by PID, using the platform's native tool to avoid
/// pulling in a libc/`nix` dependency.
fn kill_process(pid: u32) -> Result<()> {
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .arg(pid.to_string())
            .status()
            .context("failed to signal the worker process")?;
    }
    #[cfg(windows)]
    {
        std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .status()
            .context("failed to signal the worker process")?;
    }
    Ok(())
}

/// Resolves once a termination signal (SIGTERM or Ctrl+C) is received.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = term.recv() => {}
                    _ = tokio::signal::ctrl_c() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// Wire name of a protocol (`"tcp"`/`"http"`).
fn protocol_name(protocol: &Protocol) -> &'static str {
    match protocol {
        Protocol::Tcp => "tcp",
        Protocol::Http => "http",
    }
}

/// Prints a prompt and reads one trimmed line from stdin.
fn prompt(label: &str) -> Result<String> {
    print!("{label}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_token_and_expires() {
        let line = "GET /?token=abc123&expires=2026-06-08T00%3A00%3A00Z HTTP/1.1";
        let (token, expires) = parse_callback(line).expect("should parse");
        assert_eq!(token, "abc123");
        assert_eq!(expires.as_deref(), Some("2026-06-08T00:00:00Z"));
    }

    #[test]
    fn parses_token_without_expires() {
        let line = "GET /?token=xyz HTTP/1.1";
        let (token, expires) = parse_callback(line).expect("should parse");
        assert_eq!(token, "xyz");
        assert!(expires.is_none());
    }

    #[test]
    fn missing_token_yields_none() {
        assert!(parse_callback("GET /?foo=bar HTTP/1.1").is_none());
    }

    #[test]
    fn percent_decode_handles_escapes_and_plus() {
        assert_eq!(percent_decode("a%2Bb+c"), "a+b c");
    }
}
