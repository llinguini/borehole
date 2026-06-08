// History endpoints: closed tunnels and past connections. Both read from the
// in-memory ring buffers in `crate::history` and return most-recent-first.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    Json,
};
use serde::Deserialize;

use crate::api::auth::AuthClaims;
use crate::control::ServerState;
use crate::history::{ConnectionHistory, TunnelHistory};

/// Default page size when the client omits `limit`.
fn default_limit() -> usize {
    50
}

/// Query string for `GET /api/history/tunnels`.
#[derive(Debug, Deserialize)]
pub struct TunnelsQuery {
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

/// GET /api/history/tunnels — paginated list of closed tunnels.
pub async fn tunnels(
    _auth: AuthClaims,
    State(state): State<Arc<ServerState>>,
    Query(query): Query<TunnelsQuery>,
) -> Json<Vec<TunnelHistory>> {
    Json(state.history.tunnels(query.limit, query.offset).await)
}

/// Query string for `GET /api/history/connections`.
#[derive(Debug, Deserialize)]
pub struct ConnectionsQuery {
    pub tunnel_id: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

/// GET /api/history/connections — recent connections, optionally per tunnel.
pub async fn connections(
    _auth: AuthClaims,
    State(state): State<Arc<ServerState>>,
    Query(query): Query<ConnectionsQuery>,
) -> Json<Vec<ConnectionHistory>> {
    Json(
        state
            .history
            .connections(query.tunnel_id.as_deref(), query.limit)
            .await,
    )
}
