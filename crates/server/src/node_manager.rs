// Node manager (v2).
//
// In-memory registry of the `borehole-node` edges currently connected to the
// server. No persistence yet (no SQLite): everything lives behind an async
// `Arc<Mutex<HashMap<..>>>` so it can be cloned and shared across tasks.
//
// Each entry keeps a writer handle (`tx`) to the node's control TLS stream so
// the server can push orders (e.g. `OpenTunnel`) to it. That writer is a live
// `WriteHalf<TlsStream<TcpStream>>`, which cannot be constructed without a real
// TLS connection; unit tests for the load-balancing logic are therefore
// deferred to integration tests.
//
// `register`/`remove`/`pick`/`increment_tunnels`/`decrement_tunnels` are wired
// into the control plane. `list` (dashboard) and the stored `tx` writer
// (server -> node orders, pending `OpenTunnel`) are not exercised yet.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;

use tokio::io::WriteHalf;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_rustls::server::TlsStream;
use uuid::Uuid;

/// Writer half of a node's control TLS stream.
type NodeWriter = WriteHalf<TlsStream<TcpStream>>;

/// Shared, lockable writer used to send orders to a node.
pub type NodeTx = Arc<Mutex<NodeWriter>>;

/// A registered edge node and the bookkeeping needed to route tunnels to it.
///
/// `Debug` is implemented by hand: the `tx` writer is not `Debug`, so it is
/// omitted from the output.
#[derive(Clone)]
pub struct NodeInfo {
    /// Server-generated unique id (UUID v4).
    pub node_id: String,
    /// Human-friendly node name, e.g. "frankfurt", "nyc".
    pub name: String,
    /// IP or hostname advertised to CLIs for this node.
    pub host: String,
    /// Plain-TCP port where the node accepts `DataConn` connections.
    pub data_port: u16,
    /// Active tunnels routed to this node (load-balancing metric).
    pub tunnel_count: usize,
    /// Writer to the node's control stream, for sending orders.
    pub tx: NodeTx,
}

impl std::fmt::Debug for NodeInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeInfo")
            .field("node_id", &self.node_id)
            .field("name", &self.name)
            .field("host", &self.host)
            .field("data_port", &self.data_port)
            .field("tunnel_count", &self.tunnel_count)
            .finish_non_exhaustive()
    }
}

/// Thread-safe registry of connected nodes, keyed by `node_id`.
#[derive(Clone, Default)]
pub struct NodeManager {
    nodes: Arc<Mutex<HashMap<String, NodeInfo>>>,
}

impl NodeManager {
    /// Builds an empty manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a new node, returning its server-generated `node_id`.
    pub async fn register(
        &self,
        name: String,
        host: String,
        data_port: u16,
        tx: NodeTx,
    ) -> String {
        let node_id = Uuid::new_v4().to_string();
        let info = NodeInfo {
            node_id: node_id.clone(),
            name,
            host,
            data_port,
            tunnel_count: 0,
            tx,
        };
        self.nodes.lock().await.insert(node_id.clone(), info);
        node_id
    }

    /// Removes a disconnected node. No-op if `node_id` is unknown.
    pub async fn remove(&self, node_id: &str) {
        self.nodes.lock().await.remove(node_id);
    }

    /// Picks a node for a new tunnel.
    ///
    /// With `name = Some(_)`, restricts the choice to nodes with that name;
    /// otherwise considers all nodes. Among the candidates, returns the one with
    /// the fewest active tunnels. `None` if no candidate is available.
    pub async fn pick(&self, name: Option<&str>) -> Option<NodeInfo> {
        let nodes = self.nodes.lock().await;
        nodes
            .values()
            .filter(|node| name.is_none_or(|want| node.name == want))
            .min_by_key(|node| node.tunnel_count)
            .cloned()
    }

    /// Increments the active-tunnel counter for `node_id`. No-op if unknown.
    pub async fn increment_tunnels(&self, node_id: &str) {
        if let Some(node) = self.nodes.lock().await.get_mut(node_id) {
            node.tunnel_count += 1;
        }
    }

    /// Decrements the active-tunnel counter for `node_id`, saturating at zero.
    /// No-op if unknown.
    pub async fn decrement_tunnels(&self, node_id: &str) {
        if let Some(node) = self.nodes.lock().await.get_mut(node_id) {
            node.tunnel_count = node.tunnel_count.saturating_sub(1);
        }
    }

    /// Returns a snapshot of all registered nodes (for the dashboard).
    pub async fn list(&self) -> Vec<NodeInfo> {
        self.nodes.lock().await.values().cloned().collect()
    }
}
