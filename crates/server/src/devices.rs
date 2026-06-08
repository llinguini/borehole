// In-memory device registry.
//
// A "device" is a CLI host machine, identified by its hostname. There is no
// database and no per-user binding yet (CLI auth uses a shared token, not a
// dashboard user), so the registry is global: every dashboard user sees every
// device. `tunnel_count` is computed live from the active tunnels at read time,
// so it is not stored here.

use chrono::{DateTime, Utc};
use serde::Serialize;

/// Stored device record (server-internal).
#[derive(Debug, Clone)]
pub struct DeviceRecord {
    pub id: String,
    pub hostname: String,
    pub last_seen_at: DateTime<Utc>,
}

/// Public device view returned by `GET /api/devices`.
#[derive(Debug, Clone, Serialize)]
pub struct DeviceInfo {
    pub id: String,
    pub hostname: String,
    /// RFC3339 last-seen time.
    pub last_seen_at: String,
    /// Active tunnels for this device right now.
    pub tunnel_count: usize,
}
