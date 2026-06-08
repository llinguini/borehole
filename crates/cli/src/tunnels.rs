// Background tunnel registry persisted under ~/.borehole/tunnels/<id>.json.
//
// Each detached tunnel is represented by one JSON record holding enough state
// for `list` (display) and `stop` (find the worker PID). The detaching parent
// writes the initial record; the worker process updates it once the server
// assigns a port.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::home_dir;

/// Lifecycle state of a background tunnel.
pub const STATE_STARTING: &str = "starting";
pub const STATE_ACTIVE: &str = "active";

/// Persisted description of a background tunnel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelRecord {
    /// Short unique id (also the JSON file stem).
    pub id: String,
    /// OS process id of the worker serving this tunnel (0 until it starts).
    pub pid: u32,
    /// "tcp" or "http".
    pub protocol: String,
    /// Local port being exposed.
    pub local_port: u16,
    /// Remote port assigned by the server (`None` until registered).
    pub remote_port: Option<u16>,
    /// Preferred edge node, if any.
    pub node: Option<String>,
    /// Lifecycle state: `STATE_STARTING` or `STATE_ACTIVE`.
    pub state: String,
}

impl TunnelRecord {
    /// Persists this record to its JSON file.
    pub fn save(&self) -> Result<()> {
        let path = record_path(&self.id)?;
        let json = serde_json::to_string_pretty(self).context("failed to serialize tunnel")?;
        std::fs::write(&path, json)
            .with_context(|| format!("cannot write tunnel record {}", path.display()))?;
        Ok(())
    }

    /// Loads a record by id.
    pub fn load(id: &str) -> Result<Self> {
        let path = record_path(id)?;
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("no background tunnel '{id}'"))?;
        let record = serde_json::from_str(&contents)
            .with_context(|| format!("corrupt tunnel record {}", path.display()))?;
        Ok(record)
    }

    /// Deletes the record file for `id` (no error if already gone).
    pub fn remove(id: &str) -> Result<()> {
        let path = record_path(id)?;
        if path.exists() {
            std::fs::remove_file(&path)
                .with_context(|| format!("cannot remove tunnel record {}", path.display()))?;
        }
        Ok(())
    }

    /// Lists every stored tunnel record (skipping unreadable/corrupt files).
    pub fn list() -> Result<Vec<Self>> {
        let dir = tunnels_dir()?;
        let mut records = Vec::new();
        if !dir.exists() {
            return Ok(records);
        }
        for entry in std::fs::read_dir(&dir)
            .with_context(|| format!("cannot read {}", dir.display()))?
        {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(contents) = std::fs::read_to_string(&path) {
                if let Ok(record) = serde_json::from_str::<Self>(&contents) {
                    records.push(record);
                }
            }
        }
        records.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(records)
    }
}

/// Returns ~/.borehole/tunnels, creating it if necessary.
pub fn tunnels_dir() -> Result<PathBuf> {
    let home = home_dir().context("cannot determine home directory")?;
    let dir = home.join(".borehole").join("tunnels");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("cannot create {}", dir.display()))?;
    Ok(dir)
}

/// Returns the JSON path for a tunnel id.
fn record_path(id: &str) -> Result<PathBuf> {
    Ok(tunnels_dir()?.join(format!("{id}.json")))
}
