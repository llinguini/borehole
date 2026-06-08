// CLI configuration persisted at ~/.borehole.json.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// On-disk CLI configuration. v1 fields (`server_addr`, `token`) are used today;
/// the v2 fields are reserved and stay `None` for now.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct CliConfig {
    /// Server control address, e.g. "1.2.3.4:7000" (v1).
    pub server_addr: Option<String>,
    /// Authentication token (v1).
    pub token: Option<String>,
    /// v2 (reserved): base URL of the control API.
    pub server_url: Option<String>,
    /// v2 (reserved): issued JWT.
    pub jwt: Option<String>,
    /// v2 (reserved): JWT expiry timestamp.
    pub jwt_expires_at: Option<String>,
}

impl CliConfig {
    /// Returns the path to ~/.borehole.json.
    pub fn path() -> Result<PathBuf> {
        let home = home_dir().context("cannot determine home directory")?;
        Ok(home.join(".borehole.json"))
    }

    /// Loads the config from disk, returning the default when the file is absent.
    pub fn load() -> Result<Self> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read config at {}", path.display()))?;
        let config = serde_json::from_str(&contents)
            .with_context(|| format!("invalid config at {}", path.display()))?;
        Ok(config)
    }

    /// Saves the config to disk (creating the file if needed).
    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        let json = serde_json::to_string_pretty(self).context("failed to serialize config")?;
        std::fs::write(&path, json)
            .with_context(|| format!("cannot write config to {}", path.display()))?;
        Ok(())
    }

    /// Returns the configured server address, or a descriptive error.
    pub fn require_server_addr(&self) -> Result<String> {
        self.server_addr
            .clone()
            .filter(|s| !s.is_empty())
            .context("no server configured; run `borehole config` first")
    }

    /// Returns the configured token, or a descriptive error.
    pub fn require_token(&self) -> Result<String> {
        self.token
            .clone()
            .filter(|s| !s.is_empty())
            .context("no token configured; run `borehole config` first")
    }

    /// Returns the configured dashboard URL (v2), or a descriptive error.
    pub fn require_server_url(&self) -> Result<String> {
        self.server_url
            .clone()
            .filter(|s| !s.is_empty())
            .context("no server URL configured; run `borehole config` first")
    }

    /// Returns the stored JWT (v2), or a descriptive error.
    // Part of the v2 auth API; consumed once authenticated commands land.
    #[allow(dead_code)]
    pub fn require_jwt(&self) -> Result<String> {
        self.jwt
            .clone()
            .filter(|s| !s.is_empty())
            .context("not authenticated; run `borehole login` first")
    }

    /// Reports whether the stored JWT is missing or past its expiry. A missing
    /// or unparseable `jwt_expires_at` is treated as expired (fail closed).
    // Part of the v2 auth API; consumed once authenticated commands land.
    #[allow(dead_code)]
    pub fn is_jwt_expired(&self) -> bool {
        match self.jwt_expires_at.as_deref() {
            Some(s) => match chrono::DateTime::parse_from_rfc3339(s) {
                Ok(expiry) => expiry.with_timezone(&chrono::Utc) <= chrono::Utc::now(),
                Err(_) => true,
            },
            None => true,
        }
    }
}

/// Best-effort home directory lookup (`$HOME`, then `%USERPROFILE%`).
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}
