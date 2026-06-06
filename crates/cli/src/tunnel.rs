// Connect to server and maintain tunnel

use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use owo_colors::OwoColorize;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, RootCertStore};
use tokio::io::{copy_bidirectional, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use common::proto::{
    decode, encode, ClientMsg, DataConn, NewConn, Ping, Register, Registered, ServerError,
    ServerMsg,
};

use crate::config::BoreholeConfig;

/// Connects to the server, registers the tunnel and keeps it alive, spawning a
/// data connection for every visitor the server announces.
pub async fn run(
    cfg: &BoreholeConfig,
    protocol: &str,
    local_port: u16,
    remote_port: Option<u16>,
) -> Result<()> {
    // 0. Fail fast if there is nothing to expose: the local service must be
    //    reachable before we register a tunnel for it.
    ensure_local_service(local_port).await?;

    // 1. Build the client-side TLS configuration from the system roots.
    let connector = build_connector();

    // The server hostname is the part of `server_addr` before the port.
    let server_ip = server_hostname(&cfg.server_addr);
    let server_name =
        ServerName::try_from(server_ip.clone()).context("invalid server hostname")?;

    // 2. Open the control connection and complete the TLS handshake.
    let mut stream = tls_connect(&connector, &server_name, &cfg.server_addr).await?;

    // 3. Send the registration request.
    let register = ClientMsg::Register(Register {
        token: cfg.token.clone(),
        protocol: protocol.to_string(),
        local_port,
        remote_port,
    });
    stream.write_all(encode(&register)?.as_bytes()).await?;
    stream.flush().await?;

    // 4. Read the server's response to the registration.
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .await
        .context("failed to read server response")?;

    let assigned_port = match decode(line.trim_end())? {
        ServerMsg::Error(ServerError { reason }) => return Err(anyhow!(reason)),
        ServerMsg::Registered(Registered { remote_port }) => remote_port,
        other => bail!("unexpected response from server: {other:?}"),
    };

    print_banner(protocol, &server_ip, local_port, assigned_port);

    // 5. Keep listening for server notifications and serve each new visitor.
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break, // server closed the control connection
            Ok(_) => {}
            Err(_) => break,
        }

        let msg = match decode(line.trim_end()) {
            Ok(msg) => msg,
            Err(_) => break,
        };

        match msg {
            ServerMsg::NewConn(NewConn { conn_id }) => {
                let connector = connector.clone();
                let server_name = server_name.clone();
                let server_addr = cfg.server_addr.clone();
                tokio::spawn(async move {
                    let result =
                        serve_conn(connector, server_name, server_addr, local_port, conn_id).await;
                    if let Err(e) = result {
                        eprintln!("data connection error: {e}");
                    }
                });
            }
            _ => break,
        }
    }

    Ok(())
}

/// Extracts the hostname from a `host:port` address, used as the TLS
/// `ServerName`. Falls back to the whole string when no port is present.
fn server_hostname(server_addr: &str) -> String {
    server_addr
        .split(':')
        .next()
        .unwrap_or(server_addr)
        .to_string()
}

/// Ensures a local service is listening on `127.0.0.1:local_port`. Connecting
/// is the most reliable cross-platform check: a free port refuses the
/// connection, so a successful connect proves something is listening.
async fn ensure_local_service(local_port: u16) -> Result<()> {
    TcpStream::connect(("127.0.0.1", local_port))
        .await
        .map(|_| ())
        .with_context(|| {
            format!(
                "no local service is listening on 127.0.0.1:{local_port}; \
                 start it before opening the tunnel"
            )
        })
}

/// Validates connectivity, TLS and the configured token by performing a
/// `Ping`/`Pong` round-trip against the server. Returns an error describing the
/// first failure (connection, TLS handshake, or rejected token). Acquires no
/// port on the server, so it is safe to call from `borehole config`.
pub async fn check_server(cfg: &BoreholeConfig) -> Result<()> {
    let connector = build_connector();
    let server_ip = server_hostname(&cfg.server_addr);
    let server_name =
        ServerName::try_from(server_ip).context("invalid server hostname")?;

    let mut stream = tls_connect(&connector, &server_name, &cfg.server_addr).await?;

    let ping = ClientMsg::Ping(Ping {
        token: cfg.token.clone(),
    });
    stream.write_all(encode(&ping)?.as_bytes()).await?;
    stream.flush().await?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .await
        .context("failed to read server response")?;

    match decode(line.trim_end())? {
        ServerMsg::Pong => Ok(()),
        ServerMsg::Error(ServerError { reason }) => Err(anyhow!(reason)),
        other => bail!("unexpected response from server: {other:?}"),
    }
}

/// Builds a `TlsConnector` trusting the system's root certificates.
fn build_connector() -> TlsConnector {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();

    TlsConnector::from(Arc::new(config))
}

/// Opens a TCP connection to `addr` and performs the TLS handshake against
/// `server_name`.
async fn tls_connect(
    connector: &TlsConnector,
    server_name: &ServerName<'static>,
    addr: &str,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>> {
    let tcp = TcpStream::connect(addr)
        .await
        .with_context(|| format!("cannot connect to {addr}"))?;
    connector
        .connect(server_name.clone(), tcp)
        .await
        .context("TLS handshake failed")
}

/// Serves a single visitor connection: opens a fresh TLS data connection,
/// identifies it with `conn_id`, then splices it to the local service.
async fn serve_conn(
    connector: TlsConnector,
    server_name: ServerName<'static>,
    server_addr: String,
    local_port: u16,
    conn_id: String,
) -> Result<()> {
    let mut server_stream = tls_connect(&connector, &server_name, &server_addr).await?;

    let data_conn = ClientMsg::DataConn(DataConn { conn_id });
    server_stream.write_all(encode(&data_conn)?.as_bytes()).await?;
    server_stream.flush().await?;

    let mut local_stream = TcpStream::connect(format!("127.0.0.1:{local_port}"))
        .await
        .with_context(|| format!("cannot reach local service on port {local_port}"))?;

    copy_bidirectional(&mut server_stream, &mut local_stream).await?;
    Ok(())
}

/// Prints the success banner once the tunnel is established.
fn print_banner(protocol: &str, server_ip: &str, local_port: u16, remote_port: u16) {
    let local_url = format!("localhost:{local_port}");
    let remote_url = format!("{protocol}://{server_ip}:{remote_port}");

    println!("{} Túnel activo", "✓".green());
    println!("  local   → {}", local_url.cyan());
    println!("  remoto  → {}", remote_url.cyan());
    println!();
    println!("  Ctrl+C para cerrar");
}
