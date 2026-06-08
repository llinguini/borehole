// Device endpoint. Lists CLI host machines that have registered tunnels.
//
// The registry is global (see `crate::devices`); `tunnel_count` is computed live
// from the active tunnels at request time, matching tunnels by hostname.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{extract::State, Json};

use crate::api::auth::AuthClaims;
use crate::control::ServerState;
use crate::devices::DeviceInfo;

/// GET /api/devices — list known devices with their current active-tunnel count.
pub async fn list(
    _auth: AuthClaims,
    State(state): State<Arc<ServerState>>,
) -> Json<Vec<DeviceInfo>> {
    // Active tunnels per hostname (a single snapshot, then we drop the lock).
    let counts: HashMap<String, usize> = {
        let tunnels = state.active_tunnels.lock().await;
        let mut counts = HashMap::new();
        for entry in tunnels.values() {
            if let Some(hostname) = &entry.hostname {
                *counts.entry(hostname.clone()).or_insert(0) += 1;
            }
        }
        counts
    };

    let devices = state.devices.lock().await;
    let rows = devices
        .iter()
        .map(|device| DeviceInfo {
            id: device.id.clone(),
            hostname: device.hostname.clone(),
            last_seen_at: device.last_seen_at.to_rfc3339(),
            tunnel_count: counts.get(&device.hostname).copied().unwrap_or(0),
        })
        .collect();
    Json(rows)
}
