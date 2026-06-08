// Node endpoint. Requires a valid JWT via the `AuthClaims` extractor.

use std::sync::Arc;

use axum::{extract::State, Json};
use serde::Serialize;

use crate::api::auth::AuthClaims;
use crate::control::ServerState;

/// One row of `GET /api/nodes`.
#[derive(Debug, Serialize)]
pub struct NodeDto {
    pub node_id: String,
    pub name: String,
    pub host: String,
    pub data_port: u16,
    pub tunnel_count: usize,
    /// Connectivity status. A node is only in the registry while connected, so
    /// every listed node is reported as "up".
    pub status: String,
}

/// GET /api/nodes — snapshot of the connected edge nodes.
pub async fn list(_auth: AuthClaims, State(state): State<Arc<ServerState>>) -> Json<Vec<NodeDto>> {
    let nodes = state.node_manager.list().await;
    let rows = nodes
        .into_iter()
        .map(|node| NodeDto {
            node_id: node.node_id,
            name: node.name,
            host: node.host,
            data_port: node.data_port,
            tunnel_count: node.tunnel_count,
            status: "up".to_string(),
        })
        .collect();
    Json(rows)
}
