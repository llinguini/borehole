// Tunnel endpoints. Both require a valid JWT via the `AuthClaims` extractor.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use serde::Serialize;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};

use borehole_common::proto::Protocol;

use crate::api::auth::AuthClaims;
use crate::api::json_error;
use crate::control::ServerState;

/// One row of `GET /api/tunnels`.
#[derive(Debug, Serialize)]
pub struct TunnelDto {
    pub tunnel_id: String,
    pub protocol: String,
    pub local_port: u16,
    pub remote_port: u16,
    /// Edge node name, or `null` for the direct path.
    pub node: Option<String>,
    /// CLI host machine name, or `null` if the CLI didn't report one.
    pub device: Option<String>,
    /// RFC3339 time the tunnel opened. The dashboard computes uptime client-side.
    pub started_at: String,
}

/// GET /api/tunnels — snapshot of the active tunnels.
pub async fn list(
    _auth: AuthClaims,
    State(state): State<Arc<ServerState>>,
) -> Json<Vec<TunnelDto>> {
    let tunnels = state.active_tunnels.lock().await;
    let rows = tunnels
        .iter()
        .map(|(id, entry)| TunnelDto {
            tunnel_id: id.clone(),
            protocol: protocol_str(&entry.protocol).to_string(),
            local_port: entry.local_port,
            remote_port: entry.remote_port,
            node: entry.node.clone(),
            device: entry.hostname.clone(),
            started_at: entry.started_at.to_rfc3339(),
        })
        .collect();
    Json(rows)
}

/// DELETE /api/tunnels/:id — signal the owning task to tear the tunnel down.
pub async fn close(
    _auth: AuthClaims,
    State(state): State<Arc<ServerState>>,
    Path(tunnel_id): Path<String>,
) -> Response {
    let close = {
        let tunnels = state.active_tunnels.lock().await;
        tunnels.get(&tunnel_id).map(|entry| entry.close.clone())
    };
    match close {
        Some(notify) => {
            // Wake the serving loop; it removes the entry and frees the port.
            notify.notify_one();
            StatusCode::NO_CONTENT.into_response()
        }
        None => (StatusCode::NOT_FOUND, json_error("tunnel not found")).into_response(),
    }
}

/// GET /api/events — Server-Sent Events stream of real-time `BhEvent`s.
///
/// Each connected dashboard subscribes a fresh receiver; lifecycle events
/// (tunnels/nodes coming and going) arrive as `data:` lines of JSON. A keep-alive
/// comment is sent every 15s so idle connections (and proxies) stay open. A slow
/// client that lags behind the broadcast buffer silently drops the missed events
/// (`result.ok()` discards `Lagged`).
pub async fn stream_events(
    _auth: AuthClaims,
    State(state): State<Arc<ServerState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.event_bus.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| {
        result.ok().and_then(|event| {
            serde_json::to_string(&event)
                .ok()
                .map(|data| Ok(Event::default().data(data)))
        })
    });
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    )
}

/// Renders a `Protocol` as its lowercase wire name.
fn protocol_str(protocol: &Protocol) -> &'static str {
    match protocol {
        Protocol::Tcp => "tcp",
        Protocol::Http => "http",
    }
}
