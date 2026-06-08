// Node configuration, deserialized from a JSON file.

use anyhow::{Context, Result};
use serde::Deserialize;

/// On-disk configuration for a `borehole-node` agent.
#[derive(Debug, Deserialize)]
pub struct NodeConfig {
    /// Server control-plane address (TLS), e.g. "1.2.3.4:7000".
    pub server_addr: String,
    /// Token presented to the server in `RegisterNode`.
    pub token: String,
    /// Node name, e.g. "frankfurt".
    pub name: String,
    /// Plain-TCP port advertised to the server (and, through it, to CLIs) as the
    /// node's data endpoint. Should match the port of `data_bind`.
    pub data_port: u16,
    /// Plain-TCP address the node listens on for CLI `DataConn`s,
    /// e.g. "0.0.0.0:7002".
    pub data_bind: String,
}

impl NodeConfig {
    /// Reads and deserializes the configuration from the JSON file at `path`.
    pub fn load(path: &str) -> Result<Self> {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config file: {path}"))?;
        let config = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse config file: {path}"))?;
        Ok(config)
    }
}
