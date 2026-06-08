// borehole-node: the edge agent of a multi-node deployment ("the muscle").
//
// A node keeps one TLS control connection to the server and authenticates only
// itself (not end-user CLIs). On each `OpenTunnel` order it binds the requested
// public port and accepts external visitors; for every visitor it tells the
// server (`VisitorConn`), waits for the CLI's plain-TCP `DataConn` on its data
// port, and splices the two sockets. `CloseTunnel` drops the public listener.
//
// The node never terminates TLS for user traffic: it speaks TLS only to the
// server; visitor and CLI data connections are plain TCP.
//
// If the server connection drops, the node retries every 5 seconds forever.

mod config;
mod proxy;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader, WriteHalf,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, Mutex as AsyncMutex};
use tokio::task::JoinHandle;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use borehole_common::proto::{
    self, CloseTunnel, DataConn, Message, NodeRegistered, OpenTunnel, RegisterNode, VisitorConn,
};

use config::NodeConfig;

/// Default configuration path used when `--config` is not supplied.
const DEFAULT_CONFIG_PATH: &str = "/etc/borehole/node.json";

/// How long a tunnel waits for the CLI's `DataConn` after reporting a visitor.
const DATA_CONN_TIMEOUT: Duration = Duration::from_secs(10);

/// Delay between reconnection attempts when the server link drops.
const RECONNECT_DELAY: Duration = Duration::from_secs(5);

/// `conn_id` -> sender that hands the CLI data socket to the waiting visitor
/// task. Guarded by a std mutex (only sync inserts/removes, never held across
/// `.await`).
type PendingConns = Arc<Mutex<HashMap<String, oneshot::Sender<TcpStream>>>>;

/// Shared writer to the server control stream. Async mutex: writes happen from
/// concurrent tunnel tasks and span `.await`s.
type ServerWriter = Arc<AsyncMutex<WriteHalf<TlsStream<TcpStream>>>>;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    // rustls 0.23 needs a process-level crypto provider; standardise on
    // aws-lc-rs. Ignore the error if one is already installed.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let config = NodeConfig::load(&parse_config_path())?;
    let connector = build_connector()?;

    // The data listener is the node's stable port; bind it once and route every
    // `DataConn` to the visitor task waiting on its `conn_id`.
    let pending: PendingConns = Arc::new(Mutex::new(HashMap::new()));
    let data_listener = TcpListener::bind(&config.data_bind)
        .await
        .with_context(|| format!("cannot bind data listener on {}", config.data_bind))?;
    info!("listening for data connections on {}", config.data_bind);
    {
        let pending = pending.clone();
        tokio::spawn(async move { data_loop(data_listener, pending).await });
    }

    // Reconnect forever: each session runs until the server link drops.
    loop {
        if let Err(e) = run_session(&config, &connector, &pending).await {
            warn!("session ended: {e}");
        }
        info!("reconnecting in {}s", RECONNECT_DELAY.as_secs());
        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}

/// Runs one server session: connect, register, then act on tunnel orders until
/// the control channel closes. Public listeners opened during the session are
/// torn down before returning.
async fn run_session(
    config: &NodeConfig,
    connector: &TlsConnector,
    pending: &PendingConns,
) -> Result<()> {
    // Connect and TLS-handshake to the server control plane.
    let host = host_of(&config.server_addr).to_string();
    let server_name =
        rustls::pki_types::ServerName::try_from(host).context("invalid server hostname")?;
    let tcp = TcpStream::connect(&config.server_addr)
        .await
        .with_context(|| format!("cannot connect to {}", config.server_addr))?;
    let tls = connector
        .connect(server_name, tcp)
        .await
        .context("TLS handshake failed")?;

    let (read_half, mut write_half) = tokio::io::split(tls);
    let mut reader = BufReader::new(read_half);

    // Register this node.
    let register = Message::RegisterNode(RegisterNode {
        token: config.token.clone(),
        name: config.name.clone(),
        data_port: config.data_port,
    });
    write_half
        .write_all(proto::to_line(&register)?.as_bytes())
        .await?;
    write_half.flush().await?;

    // Await the registration verdict.
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    match proto::from_line(&line)? {
        Message::NodeRegistered(NodeRegistered { node_id }) => {
            info!("registered as {node_id}");
        }
        Message::Error(err) => anyhow::bail!("server rejected registration: {}", err.reason),
        other => anyhow::bail!("unexpected response from server: {other:?}"),
    }

    let server_writer: ServerWriter = Arc::new(AsyncMutex::new(write_half));
    let mut tunnels: HashMap<String, JoinHandle<()>> = HashMap::new();

    // Act on server orders until the control channel closes.
    let result = serve_orders(&mut reader, &server_writer, pending, &mut tunnels).await;

    // Tear down all public listeners so their ports are freed before we reconnect.
    for (_, handle) in tunnels.drain() {
        handle.abort();
    }
    result
}

/// Reads and dispatches server orders for one session.
async fn serve_orders<R>(
    reader: &mut R,
    server_writer: &ServerWriter,
    pending: &PendingConns,
    tunnels: &mut HashMap<String, JoinHandle<()>>,
) -> Result<()>
where
    R: AsyncBufRead + Unpin,
{
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            anyhow::bail!("server disconnected");
        }
        match proto::from_line(&line) {
            Ok(Message::OpenTunnel(OpenTunnel {
                tunnel_id,
                remote_port,
            })) => {
                open_tunnel(tunnel_id, remote_port, server_writer, pending, tunnels).await;
            }
            Ok(Message::CloseTunnel(CloseTunnel { tunnel_id })) => {
                if let Some(handle) = tunnels.remove(&tunnel_id) {
                    handle.abort();
                    info!("tunnel {tunnel_id} closed");
                }
            }
            Ok(other) => warn!("unexpected message from server: {other:?}"),
            Err(e) => warn!("malformed message from server: {e}"),
        }
    }
}

/// Binds the public port for a tunnel and spawns its accept loop.
async fn open_tunnel(
    tunnel_id: String,
    remote_port: u16,
    server_writer: &ServerWriter,
    pending: &PendingConns,
    tunnels: &mut HashMap<String, JoinHandle<()>>,
) {
    let listener = match TcpListener::bind(("0.0.0.0", remote_port)).await {
        Ok(l) => l,
        Err(e) => {
            warn!("failed to bind public port {remote_port}: {e}");
            return;
        }
    };
    info!("tunnel {tunnel_id} open on public port {remote_port}");

    let writer = server_writer.clone();
    let pending = pending.clone();
    let id = tunnel_id.clone();
    let handle = tokio::spawn(async move {
        tunnel_accept_loop(listener, id, writer, pending).await;
    });
    tunnels.insert(tunnel_id, handle);
}

/// Accepts visitors on a tunnel's public port until the task is aborted.
/// For each visitor it reports a `VisitorConn` and spawns a task that waits for
/// the CLI's `DataConn` and proxies the two sockets.
async fn tunnel_accept_loop(
    listener: TcpListener,
    tunnel_id: String,
    server_writer: ServerWriter,
    pending: PendingConns,
) {
    loop {
        let (visitor, _addr) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                warn!("accept error on tunnel {tunnel_id}: {e}");
                continue;
            }
        };

        let conn_id = Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel::<TcpStream>();
        pending.lock().unwrap().insert(conn_id.clone(), tx);

        // Tell the server, which relays a `NewConn` to the owning CLI.
        let notice = Message::VisitorConn(VisitorConn {
            tunnel_id: tunnel_id.clone(),
            conn_id: conn_id.clone(),
        });
        let frame = match proto::to_line(&notice) {
            Ok(f) => f,
            Err(_) => {
                pending.lock().unwrap().remove(&conn_id);
                continue;
            }
        };
        {
            let mut guard = server_writer.lock().await;
            if guard.write_all(frame.as_bytes()).await.is_err() || guard.flush().await.is_err() {
                pending.lock().unwrap().remove(&conn_id);
                warn!("server unreachable; dropping visitor on tunnel {tunnel_id}");
                continue;
            }
        }

        // Wait (bounded) for the CLI's data socket, then splice.
        let pending = pending.clone();
        tokio::spawn(async move {
            match tokio::time::timeout(DATA_CONN_TIMEOUT, rx).await {
                Ok(Ok(data)) => proxy::pipe(visitor, data).await,
                _ => {
                    pending.lock().unwrap().remove(&conn_id);
                    warn!("timed out waiting for data connection {conn_id}");
                }
            }
        });
    }
}

/// Accept loop for the plain-TCP data listener: each connection opens with a
/// `DataConn` frame identifying the tunnel it serves.
async fn data_loop(listener: TcpListener, pending: PendingConns) {
    loop {
        let (stream, _addr) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                warn!("data accept error: {e}");
                continue;
            }
        };
        let pending = pending.clone();
        tokio::spawn(async move { handle_data_conn(stream, pending).await });
    }
}

/// Reads the opening `DataConn` frame and hands the socket to the visitor task
/// waiting on that `conn_id`.
async fn handle_data_conn(mut stream: TcpStream, pending: PendingConns) {
    let line = match read_frame(&mut stream).await {
        Ok(Some(line)) => line,
        _ => return,
    };
    match proto::from_line(&line) {
        Ok(Message::DataConn(DataConn { conn_id })) => {
            let sender = pending.lock().unwrap().remove(&conn_id);
            match sender {
                Some(tx) => {
                    if tx.send(stream).is_err() {
                        warn!("visitor for {conn_id} no longer waiting");
                    }
                }
                None => warn!("data conn for unknown conn_id {conn_id}"),
            }
        }
        _ => warn!("unexpected message on data port"),
    }
}

/// Reads a single '\n'-terminated frame one byte at a time, so no application
/// bytes following the frame are consumed (avoids `BufReader` over-read).
async fn read_frame<R: AsyncRead + Unpin>(stream: &mut R) -> std::io::Result<Option<String>> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        if stream.read(&mut byte).await? == 0 {
            return Ok((!buf.is_empty()).then(|| String::from_utf8_lossy(&buf).into_owned()));
        }
        if byte[0] == b'\n' {
            return Ok(Some(String::from_utf8_lossy(&buf).into_owned()));
        }
        buf.push(byte[0]);
    }
}

/// Initializes tracing. Honors `RUST_LOG`, defaulting to `info`.
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

/// Returns the path passed via `--config <path>`, or the default when absent.
fn parse_config_path() -> String {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--config" {
            if let Some(path) = args.next() {
                return path;
            }
        }
    }
    DEFAULT_CONFIG_PATH.to_string()
}

/// Returns the host part of a `host:port` address.
fn host_of(addr: &str) -> &str {
    addr.rsplit_once(':').map(|(host, _)| host).unwrap_or(addr)
}

/// Builds a TLS client connector.
///
/// TODO: validate the server certificate in production. For v1/development this
/// accepts any certificate so self-signed setups work out of the box.
fn build_connector() -> Result<TlsConnector> {
    let config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(danger::NoCertVerifier))
        .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(config)))
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
