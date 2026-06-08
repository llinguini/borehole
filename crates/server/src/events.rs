// Real-time event bus for the dashboard.
//
// A single in-process `broadcast` channel fans out `BhEvent`s to every connected
// SSE client. Handlers in the control plane `publish` lifecycle changes (tunnels
// and nodes coming and going); the `/api/events` endpoint `subscribe`s a fresh
// receiver per client. There is no persistence: events are ephemeral and only
// delivered to clients connected at the time.

use serde::Serialize;
use tokio::sync::broadcast;

/// Capacity of the broadcast channel. A slow SSE client that falls this many
/// events behind is signalled a lag (we drop those events for that client).
const CHANNEL_CAPACITY: usize = 256;

/// A dashboard-facing real-time event. Serialized as an internally-tagged JSON
/// object (`{"type":"tunnel_opened", ...}`) for the SSE `data:` payload.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BhEvent {
    /// A tunnel became active. `node` is the edge node name, or empty for the
    /// direct (single-node) path. `started_at` is RFC3339 (so the dashboard can
    /// render a complete row from the event alone, without a follow-up fetch).
    TunnelOpened {
        tunnel_id: String,
        protocol: String,
        local_port: u16,
        remote_port: u16,
        node: String,
        started_at: String,
    },
    /// A tunnel was torn down (CLI disconnected or dashboard closed it).
    TunnelClosed { tunnel_id: String },
    /// An edge node registered.
    NodeConnected { node_id: String, name: String },
    /// An edge node disconnected.
    NodeLeft { node_id: String },
}

/// Broadcasts `BhEvent`s to all subscribed SSE clients.
pub struct EventBus {
    tx: broadcast::Sender<BhEvent>,
}

impl EventBus {
    /// Creates an event bus with no subscribers yet.
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self { tx }
    }

    /// Publishes an event. Does nothing if there are no subscribers (the only
    /// possible `send` error), so handlers can fire-and-forget.
    pub fn publish(&self, event: BhEvent) {
        let _ = self.tx.send(event);
    }

    /// Returns a fresh receiver that observes events published from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<BhEvent> {
        self.tx.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}
