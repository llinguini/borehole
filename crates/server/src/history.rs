// In-memory, fixed-capacity history of closed tunnels and past connections.
//
// Both ring buffers drop their oldest entry once full. Everything is ephemeral
// (lost on restart): this is the v2 stand-in for a real database. Reads return
// the most recent entries first.

use std::collections::VecDeque;

use serde::Serialize;
use tokio::sync::Mutex;

/// Keep the last 500 closed tunnels.
const MAX_TUNNELS: usize = 500;
/// Keep the last 1000 connections.
const MAX_CONNECTIONS: usize = 1000;

/// A tunnel session that has ended.
#[derive(Debug, Clone, Serialize)]
pub struct TunnelHistory {
    pub tunnel_id: String,
    pub protocol: String,
    pub local_port: u16,
    pub remote_port: u16,
    pub node: Option<String>,
    /// RFC3339 open time.
    pub started_at: String,
    /// RFC3339 close time.
    pub ended_at: String,
    pub duration_secs: u64,
}

/// A single visitor connection that has been served (direct path only; node
/// paths terminate visitors on the node, out of the server's view).
#[derive(Debug, Clone, Serialize)]
pub struct ConnectionHistory {
    pub tunnel_id: String,
    pub conn_id: String,
    pub source_ip: String,
    pub source_port: u16,
    /// RFC3339 connect time.
    pub connected_at: String,
    pub duration_secs: u64,
}

/// Two capped ring buffers behind independent mutexes.
#[derive(Default)]
pub struct History {
    tunnels: Mutex<VecDeque<TunnelHistory>>,
    connections: Mutex<VecDeque<ConnectionHistory>>,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a closed tunnel, evicting the oldest if at capacity.
    pub async fn record_tunnel(&self, entry: TunnelHistory) {
        let mut queue = self.tunnels.lock().await;
        if queue.len() == MAX_TUNNELS {
            queue.pop_front();
        }
        queue.push_back(entry);
    }

    /// Records a finished connection, evicting the oldest if at capacity.
    pub async fn record_connection(&self, entry: ConnectionHistory) {
        let mut queue = self.connections.lock().await;
        if queue.len() == MAX_CONNECTIONS {
            queue.pop_front();
        }
        queue.push_back(entry);
    }

    /// Most-recent-first page of closed tunnels.
    pub async fn tunnels(&self, limit: usize, offset: usize) -> Vec<TunnelHistory> {
        let queue = self.tunnels.lock().await;
        queue.iter().rev().skip(offset).take(limit).cloned().collect()
    }

    /// Most-recent-first connections, optionally filtered by tunnel.
    pub async fn connections(
        &self,
        tunnel_id: Option<&str>,
        limit: usize,
    ) -> Vec<ConnectionHistory> {
        let queue = self.connections.lock().await;
        queue
            .iter()
            .rev()
            .filter(|conn| tunnel_id.is_none_or(|id| conn.tunnel_id == id))
            .take(limit)
            .cloned()
            .collect()
    }
}
