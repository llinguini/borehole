// Read/write ~/.borehole.json

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Persistent CLI configuration stored at `~/.borehole.json`.
///
/// `Default` yields empty strings, which is the starting point for the
/// `config` wizard before any field has been set.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BoreholeConfig {
    /// Address of the borehole server, e.g. "1.2.3.4:7000".
    pub server_addr: String,
    /// Authentication token presented to the server.
    pub token: String,
}

/// Returns the path to the configuration file: `~/.borehole.json`.
///
/// # Panics
///
/// Panics if the home directory cannot be determined from either
/// `dirs::home_dir()` or the `HOME` environment variable.
pub fn config_path() -> PathBuf {
    let home = dirs::home_dir()
        .or_else(|| std::env::var("HOME").ok().map(PathBuf::from))
        .expect("cannot determine home directory: dirs::home_dir() and $HOME are both unavailable");
    home.join(".borehole.json")
}

/// Loads and deserializes the configuration from `config_path()`.
///
/// Returns a friendly error if the file does not exist yet.
pub fn load() -> Result<BoreholeConfig> {
    let path = config_path();
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            bail!("No config found. Run `borehole config` first.");
        }
        Err(e) => {
            return Err(e).with_context(|| format!("failed to read {}", path.display()));
        }
    };

    serde_json::from_str(&contents)
        .with_context(|| format!("failed to parse {}", path.display()))
}

/// Serializes `cfg` as pretty-printed JSON and writes it to `config_path()`,
/// creating the file if it does not exist.
pub fn save(cfg: &BoreholeConfig) -> Result<()> {
    let path = config_path();
    let json = serde_json::to_string_pretty(cfg).context("failed to serialize config")?;
    std::fs::write(&path, json).with_context(|| format!("failed to write {}", path.display()))
}
