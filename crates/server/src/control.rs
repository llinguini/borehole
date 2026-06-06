// Control plane: accepts TLS connections, validates tokens

use std::collections::{HashMap, HashSet};
use std::ops::RangeInclusive;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

use tokio_rustls::server::TlsStream;

use common::proto::{decode, encode, ClientMsg, NewConn, Registered, ServerError, ServerMsg};
use uuid::Uuid;

use crate::port_mgr::PortManager;
use crate::proxy;

/// Shared, reference-counted server state passed to every connection handler.
///
/// Wrapped in an `Arc` so it can be cheaply cloned across async tasks. The
/// interior `Mutex`es guard the mutable pieces of state.
pub struct ServerState {
    /// Set of valid authentication tokens loaded from configuration.
    tokens: HashSet<String>,
    /// Pool of public ports available for tunnels.
    port_mgr: Mutex<PortManager>,
    /// Map of `conn_id` -> oneshot channel used to hand the freshly accepted
    /// data socket to the control handler that is waiting for it.
    pending_conns: Mutex<HashMap<String, oneshot::Sender<TlsStream<TcpStream>>>>,
}

impl ServerState {
    /// Builds the shared state from the configured tokens and port range,
    /// returning it wrapped in an `Arc` ready to share across tasks.
    pub fn new(tokens: Vec<String>, port_range: RangeInclusive<u16>) -> Arc<Self> {
        Arc::new(Self {
            tokens: tokens.into_iter().collect(),
            port_mgr: Mutex::new(PortManager::new(port_range)),
            pending_conns: Mutex::new(HashMap::new()),
        })
    }
}

/// Encodes a server message and writes the resulting newline-delimited frame
/// to `writer`. Serialization failures are surfaced as I/O errors so callers
/// can treat the whole send as a single fallible operation.
async fn write_msg<W>(writer: &mut W, msg: &ServerMsg) -> std::io::Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let frame = encode(msg).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    writer.write_all(frame.as_bytes()).await
}

/// Handles a single incoming control connection.
///
/// Reads exactly one framed `ClientMsg` and then either:
/// - `Register`: authenticates, reserves a public port, and serves visitor
///   connections by announcing each one and waiting for the client to open a
///   matching data connection; or
/// - `DataConn`: hands the raw socket to the control handler waiting for that
///   `conn_id`.
pub async fn handle_control(stream: TlsStream<TcpStream>, state: Arc<ServerState>) {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();

    match reader.read_line(&mut line).await {
        Ok(0) => return, // peer closed before sending anything
        Ok(_) => {}
        Err(e) => {
            eprintln!("control read error: {e}");
            return;
        }
    }

    let msg: ClientMsg = match decode(line.trim_end()) {
        Ok(msg) => msg,
        Err(e) => {
            eprintln!("control decode error: {e}");
            return;
        }
    };

    match msg {
        ClientMsg::Register(reg) => {
            // Log the client version (empty for pre-versioning clients) so
            // operators can spot outdated clients.
            let client_version = if reg.client_version.is_empty() {
                "unknown".to_string()
            } else {
                reg.client_version.clone()
            };

            // (a) Reject unknown tokens.
            if !state.tokens.contains(&reg.token) {
                let err = ServerMsg::Error(ServerError {
                    reason: "invalid token".into(),
                });
                let _ = write_msg(&mut reader, &err).await;
                return;
            }

            // (b) Reserve a public port (specific or server-chosen). The lock
            // is released immediately so it is never held across an await.
            let remote_port = state.port_mgr.lock().unwrap().acquire(reg.remote_port);
            let remote_port = match remote_port {
                Some(port) => port,
                None => {
                    let err = ServerMsg::Error(ServerError {
                        reason: "no ports available".into(),
                    });
                    let _ = write_msg(&mut reader, &err).await;
                    return;
                }
            };

            // (c) Confirm the registration to the client, advertising our
            // version so the CLI can warn on mismatches.
            let registered = ServerMsg::Registered(Registered {
                remote_port,
                server_version: crate::VERSION.to_string(),
            });
            if write_msg(&mut reader, &registered).await.is_err() {
                state.port_mgr.lock().unwrap().release(remote_port);
                return;
            }
            eprintln!(
                "tunnel registered on port {remote_port} (client v{client_version})"
            );

            // (d) Open the public listener for external visitors.
            let listener = match TcpListener::bind(("0.0.0.0", remote_port)).await {
                Ok(listener) => listener,
                Err(e) => {
                    eprintln!("failed to bind port {remote_port}: {e}");
                    state.port_mgr.lock().unwrap().release(remote_port);
                    return;
                }
            };

            // (e) For each visitor, announce a new connection and wait for the
            // client to open the matching data connection.
            loop {
                let (visitor, _addr) = match listener.accept().await {
                    Ok(pair) => pair,
                    Err(e) => {
                        eprintln!("accept error on port {remote_port}: {e}");
                        break;
                    }
                };

                let conn_id = Uuid::new_v4().to_string();
                let notice = ServerMsg::NewConn(NewConn {
                    conn_id: conn_id.clone(),
                });
                if write_msg(&mut reader, &notice).await.is_err() {
                    break;
                }

                let (tx, rx) = oneshot::channel::<TlsStream<TcpStream>>();
                state.pending_conns.lock().unwrap().insert(conn_id, tx);

                // Wait for the data socket off the control path and splice it
                // to the visitor connection.
                tokio::spawn(async move {
                    if let Ok(data) = rx.await {
                        proxy::pipe(visitor, data).await;
                    }
                });
            }

            // Control connection gone: return the port to the pool.
            state.port_mgr.lock().unwrap().release(remote_port);
        }
        ClientMsg::DataConn(dc) => {
            // Deliver the raw socket to the handler waiting on this id.
            let waiter = state.pending_conns.lock().unwrap().remove(&dc.conn_id);
            if let Some(tx) = waiter {
                let _ = tx.send(reader.into_inner());
            }
            // Unknown id: drop the connection silently.
        }
        ClientMsg::Ping(ping) => {
            // Connectivity/token probe: validate the token and reply, without
            // reserving a port or opening any public listener.
            let reply = if state.tokens.contains(&ping.token) {
                ServerMsg::Pong
            } else {
                ServerMsg::Error(ServerError {
                    reason: "invalid token".into(),
                })
            };
            let _ = write_msg(&mut reader, &reply).await;
        }
    }
}
