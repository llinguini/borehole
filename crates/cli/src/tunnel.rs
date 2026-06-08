// Tunnel client: the persistent TLS control connection plus the per-visitor
// data connections.
//
// Flow: the CLI opens one TLS connection to the server control port, sends a
// `Register`, and on success keeps listening for `NewConn` notifications. For
// each `NewConn` it opens a *plain* TCP connection to the server data port
// (control port + 1), announces the `conn_id` with a `DataConn` frame, dials
// the local service, and splices both sockets together.

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use owo_colors::OwoColorize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tracing::warn;

use borehole_common::proto::{self, DataConn, Message, Protocol, Register, Registered};

use crate::config::CliConfig;

/// Opens the tunnel and runs the control loop until the server disconnects.
///
/// `node` is an optional preferred edge node name. `ready_tx`, when provided,
/// receives the server-assigned remote port once registration succeeds (used by
/// the background worker to update its record); it is ignored on failure.
pub async fn run(
    local_port: u16,
    remote_port: Option<u16>,
    protocol: Protocol,
    node: Option<String>,
    ready_tx: Option<tokio::sync::oneshot::Sender<u16>>,
) -> Result<()> {
    // 1. Load configuration.
    let cfg = CliConfig::load()?;
    let server_addr = cfg.require_server_addr()?;
    let token = cfg.require_token()?;
    let host = host_of(&server_addr).to_string();

    // 2-3. TLS handshake over a fresh TCP connection to the control port.
    let connector = build_tls_connector()?;
    let server_name = rustls::pki_types::ServerName::try_from(host.clone())
        .context("invalid server hostname")?;
    let tcp = TcpStream::connect(&server_addr)
        .await
        .with_context(|| format!("cannot connect to {server_addr}"))?;
    let tls = connector
        .connect(server_name, tcp)
        .await
        .context("TLS handshake failed")?;

    // 4. Split so we can read control frames and write requests independently.
    let (read_half, mut write_half) = tokio::io::split(tls);
    let mut reader = BufReader::new(read_half);

    // 5. Register. Include the host machine name so the dashboard can track
    // this device (best-effort; `None` if it can't be resolved).
    let hostname = hostname::get()
        .ok()
        .and_then(|name| name.into_string().ok());
    let register = Message::Register(Register {
        token,
        protocol: protocol.clone(),
        local_port,
        remote_port,
        node,
        hostname,
    });
    write_half
        .write_all(proto::to_line(&register)?.as_bytes())
        .await?;
    write_half.flush().await?;

    // 6. Await the server's verdict.
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let assigned_port = match proto::from_line(&line)? {
        Message::Registered(Registered { remote_port, .. }) => remote_port,
        Message::Error(err) => return Err(anyhow!(err.reason)),
        _ => return Err(anyhow!("unexpected message from server")),
    };

    // Report the assigned port to a waiting background worker, if any.
    if let Some(tx) = ready_tx {
        let _ = tx.send(assigned_port);
    }

    // Banner. Labels dimmed, remote address/URL in brand blue (#5B8DEF,
    // approximated by the terminal's bright blue).
    println!("{}", "✓ Túnel activo".green());
    println!("  {} localhost:{local_port}", "local  →".dimmed());
    println!(
        "  {} {}",
        "remoto →".dimmed(),
        format!("{host}:{assigned_port}").bright_blue()
    );
    if matches!(protocol, Protocol::Http) {
        println!(
            "  {} {}",
            "url    →".dimmed(),
            format!("http://{host}:{assigned_port}").bright_blue()
        );
    }
    println!("\n  {}", "Ctrl+C para cerrar".dimmed());

    // 7. Control loop: spawn a data connection for every incoming visitor, and
    // shut down cleanly on Ctrl+C. A line-based reader lets us await frames.
    let mut lines = reader.lines();
    loop {
        tokio::select! {
            // Graceful shutdown: not an error, so no reconnection is attempted.
            _ = tokio::signal::ctrl_c() => {
                println!("\n{}", "✗ Túnel cerrado".red());
                return Ok(());
            }
            next = lines.next_line() => {
                match next {
                    Ok(Some(line)) => match proto::from_line(&line) {
                        Ok(Message::NewConn(new_conn)) => {
                            let server_addr = server_addr.clone();
                            let conn_id = new_conn.conn_id;
                            let node = new_conn.node_host.zip(new_conn.node_port);
                            tokio::spawn(async move {
                                if let Err(e) =
                                    handle_data_conn(server_addr, conn_id, local_port, node).await
                                {
                                    warn!("data connection error: {e}");
                                }
                            });
                        }
                        Ok(other) => warn!("unexpected control message: {other:?}"),
                        Err(e) => warn!("malformed control message: {e}"),
                    },
                    // Unexpected close: surface an error so `start` can retry.
                    Ok(None) => return Err(anyhow!("servidor desconectado")),
                    Err(e) => return Err(e).context("control channel read failed"),
                }
            }
        }
    }
}

/// Serves a single visitor: plain TCP to the data endpoint, announce the
/// `conn_id`, dial the local service, then splice both ends.
///
/// `node` is `Some((host, port))` for a multi-node tunnel (dial the edge node
/// directly) or `None` for the single-node path (dial the server's data port).
async fn handle_data_conn(
    server_addr: String,
    conn_id: String,
    local_port: u16,
    node: Option<(String, u16)>,
) -> Result<()> {
    // 1. Plain TCP to the data endpoint (node when present, else the server's
    // data port = control port + 1). No TLS on the data plane.
    let data_addr = match node {
        Some((host, port)) => format!("{host}:{port}"),
        None => data_addr_of(&server_addr)?,
    };
    let mut server_conn = TcpStream::connect(&data_addr)
        .await
        .with_context(|| format!("cannot connect to data endpoint {data_addr}"))?;

    // 2. Identify which visitor this connection serves.
    let frame = proto::to_line(&Message::DataConn(DataConn { conn_id }))?;
    server_conn.write_all(frame.as_bytes()).await?;
    server_conn.flush().await?;

    // 3. Connect to the locally exposed service.
    let mut local = TcpStream::connect(("127.0.0.1", local_port))
        .await
        .with_context(|| format!("cannot reach local service on 127.0.0.1:{local_port}"))?;

    // 4. Pipe bytes both ways until either side closes.
    tokio::io::copy_bidirectional(&mut server_conn, &mut local).await?;
    Ok(())
}

/// Builds a TLS client connector.
///
/// TODO: validate the server certificate in production. For v1/development this
/// accepts any certificate so self-signed setups work out of the box.
fn build_tls_connector() -> Result<TlsConnector> {
    // rustls 0.23 needs a process-level crypto provider; standardise on
    // aws-lc-rs. Ignore the error if one is already installed.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(danger::NoCertVerifier))
        .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(config)))
}

/// Returns the host part of a `host:port` address.
fn host_of(addr: &str) -> &str {
    addr.rsplit_once(':').map(|(host, _)| host).unwrap_or(addr)
}

/// Derives the data address (`host:control_port+1`) from the control address.
fn data_addr_of(addr: &str) -> Result<String> {
    let (host, port) = addr
        .rsplit_once(':')
        .context("server address must be host:port")?;
    let port: u16 = port.parse().context("invalid server port")?;
    Ok(format!("{host}:{}", port.saturating_add(1)))
}

/// Dangerous TLS verifier that accepts any certificate.
///
/// TODO: replace with real certificate validation (webpki roots or a pinned CA)
/// before production use. Scoped to its own module to keep the unsafe-by-policy
/// surface obvious.
mod danger {
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::{DigitallySignedStruct, Error, SignatureScheme};

    #[derive(Debug)]
    pub struct NoCertVerifier;

    impl ServerCertVerifier for NoCertVerifier {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            rustls::crypto::aws_lc_rs::default_provider()
                .signature_verification_algorithms
                .supported_schemes()
        }
    }
}
