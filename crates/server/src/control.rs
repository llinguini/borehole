// Control plane: the heart of the server.
//
// Both CLIs and edge nodes open one persistent TLS connection to the same
// control port. The server tells them apart by the first frame:
//   - `Register`     -> a CLI (handled by `handle_cli`)
//   - `RegisterNode` -> an edge node (handled by `handle_node`)
//
// CLI, single-node (v1) path: after a successful `Register` the server binds the
// assigned public port and, for every visitor, notifies the CLI with a
// `NewConn`. The CLI opens a plain-TCP connection to the data port
// (control port + 1) carrying a `DataConn`; the data listener pairs it with the
// waiting visitor.
//
// CLI, multi-node (v2) path: when a node is available the server advertises the
// node's host/port in `Registered` and records the tunnel in `active_tunnels`.
// A node later reports visitors via `VisitorConn`, which the server forwards to
// the owning CLI as a `NewConn` carrying the node's address.
//
// NOTE (scaffold): the multi-node data path is not yet end to end. There is no
// `OpenTunnel` (server -> node) message telling a node which public port to
// bind, and the current CLI still dials the server for data connections. So the
// node branches below are dormant until those land; the single-node path is the
// only one exercised today.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
    WriteHalf,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, Mutex, Notify};
use tokio_rustls::server::TlsStream;
use tracing::{info, warn};
use uuid::Uuid;

use borehole_common::proto::{
    self, CloseTunnel, DataConn, ErrorMsg, Message, NewConn, NodeRegistered, OpenTunnel, Protocol,
    Register, RegisterNode, Registered, VisitorConn,
};

use crate::api::tokens::TokenInfo;
use crate::config::UserConfig;
use crate::devices::DeviceRecord;
use crate::events::{BhEvent, EventBus};
use crate::history::{ConnectionHistory, History, TunnelHistory};
use crate::node_manager::NodeManager;
use crate::port_mgr::PortManager;
use crate::proxy;

/// How long a visitor waits for the CLI's data connection before being dropped.
const DATA_CONN_TIMEOUT: Duration = Duration::from_secs(10);

/// Shared, lockable writer to a CLI's control stream. Cloned into
/// `active_tunnels` so a node task can push `NewConn` frames to the CLI.
type CliWriter = Arc<Mutex<WriteHalf<TlsStream<TcpStream>>>>;

/// A live tunnel routed through an edge node: the CLI control writer plus the
/// node's host and data port, so a node task can hand visitors back to the CLI.
type TunnelRoute = (CliWriter, String, u16);

/// A live tunnel: the metadata the dashboard displays plus the machinery to
/// route visitors (multi-node only) and to close it on demand.
pub struct TunnelEntry {
    /// Tunnelled protocol (tcp/http/...).
    pub protocol: Protocol,
    /// Local port on the CLI host being exposed.
    pub local_port: u16,
    /// Public port assigned on the server/node edge.
    pub remote_port: u16,
    /// Edge node name when routed through a node; `None` for the direct path.
    pub node: Option<String>,
    /// CLI host machine name, when the CLI reported one (used by the dashboard
    /// device view and the tunnels table).
    pub hostname: Option<String>,
    /// Wall-clock time the tunnel was registered (used for uptime and the
    /// dashboard's `started_at`, served as RFC3339).
    pub started_at: DateTime<Utc>,
    /// Routing handle for the multi-node path (CLI writer + node address);
    /// `None` for the direct path, which does not need it.
    pub route: Option<TunnelRoute>,
    /// Notified to ask the serving task to tear this tunnel down (dashboard
    /// DELETE). The serving loop selects on it and exits, triggering cleanup.
    pub close: Arc<Notify>,
}

/// `tunnel_id` -> entry. Source of truth for the dashboard and for node tasks
/// forwarding visitors back to the owning CLI.
type ActiveTunnels = Arc<Mutex<HashMap<String, TunnelEntry>>>;

/// Per-tunnel context shared by the two serving paths. Bundled into one struct
/// so the serve helpers keep a small, readable signature.
struct TunnelCtx<'a> {
    state: &'a Arc<ServerState>,
    tunnel_id: &'a str,
    /// Public port assigned to this tunnel.
    port: u16,
    register: &'a Register,
    /// Close signal triggered by the dashboard DELETE.
    close: Arc<Notify>,
}

/// State shared across every server task (and the Axum API).
pub struct ServerState {
    pub port_mgr: PortManager,
    pub tokens: Arc<Vec<String>>,
    /// `conn_id` -> sender that delivers the data socket to the waiting proxy.
    pub pending: Arc<Mutex<HashMap<String, oneshot::Sender<TcpStream>>>>,
    /// Registry of connected edge nodes (v2).
    pub node_manager: NodeManager,
    /// `tunnel_id` -> entry. Lets a node task forward a visitor back to the
    /// owning CLI and lets the dashboard list/close tunnels.
    pub active_tunnels: ActiveTunnels,
    /// Secret used to sign and verify dashboard JWTs (HS256).
    pub jwt_secret: String,
    /// Dashboard users (hardcoded in config; no database yet).
    pub users: Arc<Vec<UserConfig>>,
    /// Real-time event bus fanned out to dashboard SSE clients.
    pub event_bus: Arc<EventBus>,
    /// Dashboard-managed API tokens, in memory, keyed by user email.
    pub api_tokens: Arc<Mutex<HashMap<String, Vec<TokenInfo>>>>,
    /// Known CLI devices (global; no per-user binding yet).
    pub devices: Arc<Mutex<Vec<DeviceRecord>>>,
    /// Ring buffers of closed tunnels and past connections.
    pub history: Arc<History>,
}

/// Entry point for each connection to the control port. Reads the first frame
/// and dispatches to the CLI or node handler.
pub async fn handle_client(mut stream: TlsStream<TcpStream>, state: Arc<ServerState>) -> Result<()> {
    // Read the first frame directly off the TLS stream (byte-by-byte, no
    // buffering) so the dispatched handler can keep reading without losing data.
    let first = match read_frame(&mut stream).await? {
        Some(line) => line,
        None => return Ok(()), // client closed immediately
    };

    match proto::from_line(&first) {
        Ok(Message::Register(reg)) => handle_cli(stream, state, reg).await,
        Ok(Message::RegisterNode(reg)) => handle_node(stream, state, reg).await,
        Ok(_) => {
            let (_, mut writer) = tokio::io::split(stream);
            reject(&mut writer, "expected register or register_node").await;
            Ok(())
        }
        Err(_) => {
            let (_, mut writer) = tokio::io::split(stream);
            reject(&mut writer, "malformed first message").await;
            Ok(())
        }
    }
}

/// Handles a CLI connection: token check, port assignment, and either the
/// single-node (v1) proxy path or the multi-node (v2) advertisement path.
async fn handle_cli(
    stream: TlsStream<TcpStream>,
    state: Arc<ServerState>,
    register: Register,
) -> Result<()> {
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);

    // Validate the token.
    if !state.tokens.iter().any(|t| t == &register.token) {
        reject(&mut write_half, "invalid token").await;
        return Ok(());
    }

    // Record/refresh the device for the dashboard, before anything that might
    // bail out (e.g. no port available) so it shows up regardless.
    if let Some(hostname) = register.hostname.as_deref() {
        upsert_device(&state, hostname).await;
    }

    // Assign a public port (specific if requested, else random from pool).
    let port = match register.remote_port {
        Some(p) => state.port_mgr.acquire_specific(p).await,
        None => state.port_mgr.acquire().await,
    };
    let port = match port {
        Some(p) => p,
        None => {
            reject(&mut write_half, "no port available").await;
            return Ok(());
        }
    };

    // Identify the tunnel and create its close signal up front, so both serving
    // paths register a dashboard entry and the dashboard can tear it down.
    let tunnel_id = Uuid::new_v4().to_string();
    let ctx = TunnelCtx {
        state: &state,
        tunnel_id: &tunnel_id,
        port,
        register: &register,
        close: Arc::new(Notify::new()),
    };

    // Pick an edge node for this tunnel, honoring the CLI's `--node` preference
    // when set; no match (or no nodes) falls back to the v1 direct path.
    match state.node_manager.pick(register.node.as_deref()).await {
        Some(node) => {
            serve_cli_via_node(reader, write_half, ctx, node).await;
        }
        None => {
            serve_cli_direct(&mut reader, &mut write_half, ctx).await;
        }
    }

    // Central cleanup: drop the dashboard entry, archive it to history, and emit
    // `TunnelClosed`. Only do so if the tunnel was actually registered (i.e. it
    // had emitted `TunnelOpened`), keeping the open/close events paired.
    let removed = state.active_tunnels.lock().await.remove(&tunnel_id);
    if let Some(entry) = removed {
        let ended_at = Utc::now();
        let duration_secs = (ended_at - entry.started_at).num_seconds().max(0) as u64;
        state
            .history
            .record_tunnel(TunnelHistory {
                tunnel_id: tunnel_id.clone(),
                protocol: protocol_str(&entry.protocol),
                local_port: entry.local_port,
                remote_port: entry.remote_port,
                node: entry.node.clone(),
                started_at: entry.started_at.to_rfc3339(),
                ended_at: ended_at.to_rfc3339(),
                duration_secs,
            })
            .await;
        state.event_bus.publish(BhEvent::TunnelClosed {
            tunnel_id: tunnel_id.clone(),
        });
    }
    state.port_mgr.release(port).await;
    info!("tunnel closed: port {port}");
    Ok(())
}

/// Single-node (v1) path: the server binds the public port and proxies visitor
/// traffic to the CLI directly.
async fn serve_cli_direct<R, W>(reader: &mut R, writer: &mut W, ctx: TunnelCtx<'_>)
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let port = ctx.port;
    let tunnel_listener = match TcpListener::bind(("0.0.0.0", port)).await {
        Ok(listener) => listener,
        Err(e) => {
            warn!("failed to bind tunnel port {port}: {e}");
            reject(writer, "failed to open tunnel port").await;
            return;
        }
    };

    let registered = Message::Registered(Registered {
        remote_port: port,
        node_host: None,
        node_port: None,
    });
    if send(writer, &registered).await.is_err() {
        return;
    }

    // Record the tunnel for the dashboard. The direct path has no node and no
    // route handle (the server proxies visitors itself in `serve_tunnel`).
    let started_at = Utc::now();
    ctx.state.active_tunnels.lock().await.insert(
        ctx.tunnel_id.to_string(),
        TunnelEntry {
            protocol: ctx.register.protocol.clone(),
            local_port: ctx.register.local_port,
            remote_port: port,
            node: None,
            hostname: ctx.register.hostname.clone(),
            started_at,
            route: None,
            close: ctx.close.clone(),
        },
    );
    // The direct path has no edge node; report an empty node name.
    ctx.state.event_bus.publish(BhEvent::TunnelOpened {
        tunnel_id: ctx.tunnel_id.to_string(),
        protocol: protocol_str(&ctx.register.protocol),
        local_port: ctx.register.local_port,
        remote_port: port,
        node: String::new(),
        started_at: started_at.to_rfc3339(),
    });

    info!("tunnel open: {port} -> local:{}", ctx.register.local_port);
    serve_tunnel(
        reader,
        writer,
        &tunnel_listener,
        ctx.state,
        ctx.tunnel_id,
        port,
        &ctx.close,
    )
    .await;
}

/// Multi-node (v2) path: tell the chosen node to open the public port, advertise
/// the node to the CLI, and record the tunnel so the node can route visitors
/// back. The public port is owned by the node, so the server does not bind it.
async fn serve_cli_via_node<R>(
    mut reader: R,
    write_half: WriteHalf<TlsStream<TcpStream>>,
    ctx: TunnelCtx<'_>,
    node: crate::node_manager::NodeInfo,
) where
    R: AsyncBufRead + Unpin,
{
    let port = ctx.port;
    let writer: CliWriter = Arc::new(Mutex::new(write_half));

    // Order the node to bind the public port for this tunnel. If the node's
    // control channel is gone, fall back to the v1 direct path so the CLI still
    // gets a working tunnel.
    let open = Message::OpenTunnel(OpenTunnel {
        tunnel_id: ctx.tunnel_id.to_string(),
        remote_port: port,
    });
    {
        let mut guard = node.tx.lock().await;
        if send(&mut *guard, &open).await.is_err() {
            warn!("node {} unreachable; falling back to direct tunnel", node.name);
            drop(guard);
            let mut cli_guard = writer.lock().await;
            serve_cli_direct(&mut reader, &mut *cli_guard, ctx).await;
            return;
        }
    }

    // Record the tunnel for the dashboard, keeping the route handle so node
    // tasks can forward visitors back to this CLI.
    let started_at = Utc::now();
    ctx.state.active_tunnels.lock().await.insert(
        ctx.tunnel_id.to_string(),
        TunnelEntry {
            protocol: ctx.register.protocol.clone(),
            local_port: ctx.register.local_port,
            remote_port: port,
            node: Some(node.name.clone()),
            hostname: ctx.register.hostname.clone(),
            started_at,
            route: Some((writer.clone(), node.host.clone(), node.data_port)),
            close: ctx.close.clone(),
        },
    );
    ctx.state.node_manager.increment_tunnels(&node.node_id).await;
    ctx.state.event_bus.publish(BhEvent::TunnelOpened {
        tunnel_id: ctx.tunnel_id.to_string(),
        protocol: protocol_str(&ctx.register.protocol),
        local_port: ctx.register.local_port,
        remote_port: port,
        node: node.name.clone(),
        started_at: started_at.to_rfc3339(),
    });

    let registered = Message::Registered(Registered {
        remote_port: port,
        node_host: Some(node.host.clone()),
        node_port: Some(node.data_port),
    });
    {
        let mut guard = writer.lock().await;
        if send(&mut *guard, &registered).await.is_ok() {
            info!(
                "tunnel open via node {}: {port} -> local:{}",
                node.name, ctx.register.local_port
            );
        }
    }

    // Keep the control channel alive so forwarded `NewConn` frames can be
    // written; exit when the CLI disconnects or the dashboard closes the tunnel.
    let mut line = String::new();
    loop {
        tokio::select! {
            _ = ctx.close.notified() => break,
            res = reader.read_line(&mut line) => match res {
                Ok(0) | Err(_) => break,
                Ok(_) => line.clear(),
            },
        }
    }

    // Tunnel gone: ask the node to release the public port and update its load.
    // The dashboard entry is removed centrally by `handle_cli`.
    let close_msg = Message::CloseTunnel(CloseTunnel {
        tunnel_id: ctx.tunnel_id.to_string(),
    });
    {
        let mut guard = node.tx.lock().await;
        let _ = send(&mut *guard, &close_msg).await;
    }
    ctx.state.node_manager.decrement_tunnels(&node.node_id).await;
}

/// Handles an edge node connection: token check, registration, and a loop that
/// forwards `VisitorConn` reports to the owning CLI.
async fn handle_node(
    stream: TlsStream<TcpStream>,
    state: Arc<ServerState>,
    reg: RegisterNode,
) -> Result<()> {
    // Resolve the node's host from the TCP peer before splitting the stream.
    let host = stream
        .get_ref()
        .0
        .peer_addr()
        .map(|addr| addr.ip().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);

    // Validate the token.
    if !state.tokens.iter().any(|t| t == &reg.token) {
        reject(&mut write_half, "invalid token").await;
        return Ok(());
    }

    // Register and acknowledge.
    let tx: CliWriter = Arc::new(Mutex::new(write_half));
    let node_id = state
        .node_manager
        .register(reg.name.clone(), host.clone(), reg.data_port, tx.clone())
        .await;

    {
        let ack = Message::NodeRegistered(NodeRegistered {
            node_id: node_id.clone(),
        });
        let mut guard = tx.lock().await;
        if send(&mut *guard, &ack).await.is_err() {
            state.node_manager.remove(&node_id).await;
            return Ok(());
        }
    }
    info!("node registered: {} ({})", reg.name, host);
    state.event_bus.publish(BhEvent::NodeConnected {
        node_id: node_id.clone(),
        name: reg.name.clone(),
    });

    // Forward visitor reports to the owning CLI until the node disconnects.
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) | Err(_) => break,
            Ok(_) => match proto::from_line(&line) {
                Ok(Message::VisitorConn(VisitorConn { tunnel_id, conn_id })) => {
                    forward_visitor(&state, &tunnel_id, &conn_id).await;
                }
                Ok(other) => warn!("unexpected message from node {}: {other:?}", reg.name),
                Err(e) => warn!("malformed message from node {}: {e}", reg.name),
            },
        }
    }

    state.node_manager.remove(&node_id).await;
    state.event_bus.publish(BhEvent::NodeLeft {
        node_id: node_id.clone(),
    });
    info!("node disconnected: {}", reg.name);
    Ok(())
}

/// Inserts or refreshes a device by hostname, updating its last-seen time.
async fn upsert_device(state: &Arc<ServerState>, hostname: &str) {
    let now = Utc::now();
    let mut devices = state.devices.lock().await;
    match devices.iter_mut().find(|d| d.hostname == hostname) {
        Some(device) => device.last_seen_at = now,
        None => devices.push(DeviceRecord {
            id: Uuid::new_v4().to_string(),
            hostname: hostname.to_string(),
            last_seen_at: now,
        }),
    }
}

/// Renders a `Protocol` as its lowercase wire name (for event payloads).
fn protocol_str(protocol: &Protocol) -> String {
    match protocol {
        Protocol::Tcp => "tcp".to_string(),
        Protocol::Http => "http".to_string(),
    }
}

/// Forwards a node's `VisitorConn` to the CLI that owns `tunnel_id` as a
/// `NewConn` carrying the node's address.
async fn forward_visitor(state: &Arc<ServerState>, tunnel_id: &str, conn_id: &str) {
    let route = {
        let tunnels = state.active_tunnels.lock().await;
        tunnels.get(tunnel_id).and_then(|entry| entry.route.clone())
    };
    let (cli_writer, node_host, node_port) = match route {
        Some(r) => r,
        None => {
            warn!("visitor for unknown tunnel {tunnel_id}");
            return;
        }
    };

    let notice = Message::NewConn(NewConn {
        conn_id: conn_id.to_string(),
        node_host: Some(node_host),
        node_port: Some(node_port),
    });
    let mut guard = cli_writer.lock().await;
    if send(&mut *guard, &notice).await.is_err() {
        warn!("failed to notify CLI for tunnel {tunnel_id}");
    }
}

/// Concurrently handles incoming visitors and detects control-channel close for
/// one open tunnel. Returns when the CLI control channel closes or errors.
async fn serve_tunnel<R, W>(
    reader: &mut R,
    writer: &mut W,
    tunnel_listener: &TcpListener,
    state: &Arc<ServerState>,
    tunnel_id: &str,
    port: u16,
    close: &Notify,
) where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    loop {
        tokio::select! {
            // a. The dashboard asked to close this tunnel.
            _ = close.notified() => break,

            // b. A new external visitor connected to the public port.
            accept_res = tunnel_listener.accept() => {
                let (visitor, addr) = match accept_res {
                    Ok(pair) => pair,
                    Err(e) => {
                        warn!("tunnel accept error on port {port}: {e}");
                        continue;
                    }
                };

                let conn_id = Uuid::new_v4().to_string();
                let (tx, rx) = oneshot::channel::<TcpStream>();
                state.pending.lock().await.insert(conn_id.clone(), tx);

                let notice = Message::NewConn(NewConn {
                    conn_id: conn_id.clone(),
                    node_host: None,
                    node_port: None,
                });
                if send(writer, &notice).await.is_err() {
                    state.pending.lock().await.remove(&conn_id);
                    break;
                }

                // Wait (bounded) for the paired data socket, then proxy. Once the
                // connection ends, archive it to history (direct path only: the
                // server sees the visitor's address here).
                let pending = state.pending.clone();
                let history = state.history.clone();
                let tunnel_id = tunnel_id.to_string();
                let connected_at = Utc::now();
                tokio::spawn(async move {
                    match tokio::time::timeout(DATA_CONN_TIMEOUT, rx).await {
                        Ok(Ok(data)) => {
                            if let Err(e) = proxy::run(visitor, data).await {
                                warn!("proxy error for conn {conn_id}: {e}");
                            }
                            let duration_secs =
                                (Utc::now() - connected_at).num_seconds().max(0) as u64;
                            history
                                .record_connection(ConnectionHistory {
                                    tunnel_id,
                                    conn_id,
                                    source_ip: addr.ip().to_string(),
                                    source_port: addr.port(),
                                    connected_at: connected_at.to_rfc3339(),
                                    duration_secs,
                                })
                                .await;
                        }
                        _ => {
                            warn!("timed out waiting for data connection {conn_id}");
                            pending.lock().await.remove(&conn_id);
                        }
                    }
                });
            }

            // c. The control channel: only used to detect the CLI going away.
            read_res = read_line_opt(reader) => {
                match read_res {
                    Ok(None) | Err(_) => break, // CLI disconnected
                    Ok(Some(_)) => { /* unexpected on the control channel; ignore */ }
                }
            }
        }
    }
}

/// Accept loop for the plain-TCP data port. Each connection opens with a
/// `DataConn` frame and is handed to the visitor waiting on that `conn_id`.
pub async fn handle_data_listener(listener: TcpListener, state: Arc<ServerState>) {
    loop {
        let (stream, _addr) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                warn!("data accept error: {e}");
                continue;
            }
        };
        let state = state.clone();
        tokio::spawn(async move { handle_data_conn(stream, state).await });
    }
}

/// Reads the opening `DataConn` frame and delivers the socket to the waiting
/// tunnel task via the oneshot registered under its `conn_id`.
async fn handle_data_conn(mut stream: TcpStream, state: Arc<ServerState>) {
    let line = match read_frame(&mut stream).await {
        Ok(Some(line)) => line,
        _ => return,
    };
    match proto::from_line(&line) {
        Ok(Message::DataConn(DataConn { conn_id })) => {
            let sender = state.pending.lock().await.remove(&conn_id);
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

/// Reads one newline-delimited frame. `Ok(None)` signals a clean EOF.
async fn read_line_opt<R: AsyncBufRead + Unpin>(reader: &mut R) -> std::io::Result<Option<String>> {
    let mut line = String::new();
    let n = reader.read_line(&mut line).await?;
    Ok(if n == 0 { None } else { Some(line) })
}

/// Reads a single '\n'-terminated frame one byte at a time, so no application
/// bytes following the frame are consumed (avoids `BufReader` over-read). Used
/// both for the control-stream dispatch and the plain-TCP data connection.
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

/// Serializes and writes a message frame to the CLI, flushing afterwards.
async fn send<W: AsyncWrite + Unpin>(writer: &mut W, msg: &Message) -> Result<()> {
    let frame = proto::to_line(msg)?;
    writer.write_all(frame.as_bytes()).await?;
    writer.flush().await?;
    Ok(())
}

/// Best-effort send of an error message (failures are ignored: we close anyway).
async fn reject<W: AsyncWrite + Unpin>(writer: &mut W, reason: &str) {
    let msg = Message::Error(ErrorMsg {
        reason: reason.to_string(),
    });
    let _ = send(writer, &msg).await;
}
