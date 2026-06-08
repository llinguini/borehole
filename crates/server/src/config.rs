// Server configuration: loaded once at startup from a JSON file.

use anyhow::{Context, Result};
use serde::Deserialize;

/// Top-level server configuration.
// Scaffold: only `bind_control` is read so far; the rest are consumed once the
// listeners/TLS are wired up in later steps.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    /// Address for the control plane, e.g. "0.0.0.0:7000".
    pub bind_control: String,
    /// Address for the public HTTP plane, e.g. "0.0.0.0:80".
    pub bind_http: String,
    /// Inclusive range of public ports the server may assign, e.g.
    /// [30000, 40000].
    pub port_range: [u16; 2],
    /// Accepted client authentication tokens.
    pub tokens: Vec<String>,
    /// Secret used to sign dashboard JWTs (HS256).
    pub jwt_secret: String,
    /// Dashboard users (no database yet; hardcoded in config for v2).
    #[serde(default)]
    pub users: Vec<UserConfig>,
    /// TLS material for the control plane.
    pub tls: TlsConfig,
}

/// A dashboard user. Passwords are stored as their lowercase SHA-256 hex digest,
/// never in plaintext.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct UserConfig {
    pub email: String,
    /// SHA-256 hex digest of the password.
    pub password_hash: String,
}

/// Paths to the TLS certificate and private key (PEM).
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct TlsConfig {
    /// Path to the certificate chain PEM.
    pub cert: String,
    /// Path to the private key PEM.
    pub key: String,
}

impl ServerConfig {
    /// Reads and deserializes the configuration from the JSON file at `path`.
    pub fn load(path: &str) -> Result<Self> {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config file: {path}"))?;
        let config = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse config file: {path}"))?;
        Ok(config)
    }

    /// Plain-TCP port for CLI data connections: the control port plus one
    /// (e.g. control :7000 -> data :7001). Falls back to 0 if `bind_control`
    /// has no parseable port (the data listener bind then surfaces the error).
    pub fn data_port(&self) -> u16 {
        self.bind_control
            .rsplit_once(':')
            .and_then(|(_, port)| port.parse::<u16>().ok())
            .map(|port| port.saturating_add(1))
            .unwrap_or(0)
    }
}
