mod config;
mod tunnel;
mod update;

use std::io::Write;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use owo_colors::OwoColorize;

/// CLI version, resolved at build time (release tag in CI, crate version
/// otherwise). See `build.rs`.
pub const VERSION: &str = env!("BOREHOLE_VERSION");

/// Top-level CLI: each variant is a subcommand of `borehole`.
#[derive(Parser)]
#[command(name = "borehole", version = VERSION, about = "Self-hosted reverse tunnel")]
enum Cli {
    /// Configure server address and token
    Config {
        #[arg(long)]
        server: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    /// Start a tunnel
    Start {
        #[command(subcommand)]
        protocol: StartCmd,
    },
    /// Update the borehole CLI to the latest release
    Update,
}

/// Tunnel protocols supported by `borehole start`.
#[derive(Subcommand)]
enum StartCmd {
    /// TCP tunnel
    Tcp {
        local_port: u16,
        #[arg(long)]
        remote_port: Option<u16>,
    },
    /// HTTP tunnel
    Http {
        local_port: u16,
        #[arg(long)]
        remote_port: Option<u16>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // rustls 0.23 needs an explicit default when multiple crypto backends could
    // be linked; we standardise on aws-lc-rs (same as tokio-rustls / ureq).
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    match Cli::parse() {
        Cli::Config { server, token } => run_config(server, token).await,
        Cli::Start { protocol } => run_start(protocol).await,
        Cli::Update => {
            // The updater is blocking (ureq + file replacement), so run it off
            // the async runtime.
            tokio::task::spawn_blocking(update::run)
                .await
                .map_err(|e| anyhow!("update task failed: {e}"))?
        }
    }
}

/// Handles `borehole config`: updates the saved configuration from flags, or
/// runs an interactive wizard when no flag is provided. After saving, it probes
/// the server (TLS + token) so misconfigurations surface immediately.
async fn run_config(server: Option<String>, token: Option<String>) -> Result<()> {
    // With no flags, fall back to an interactive wizard.
    let (server, token) = if server.is_none() && token.is_none() {
        let server = prompt("Server address (host:port): ")?;
        let token = prompt("Token: ")?;
        (Some(server), Some(token))
    } else {
        (server, token)
    };

    // Start from the existing config (or defaults) and overwrite the fields
    // that were provided.
    let mut cfg = config::load().unwrap_or_default();
    if let Some(server) = server {
        // Default to the standard control port when the user omits it.
        cfg.server_addr = config::normalize_server_addr(&server);
    }
    if let Some(token) = token {
        cfg.token = token;
    }

    config::save(&cfg)?;
    println!("{} Config saved", "✓".green());

    // Verify the connection (TLS + token). A failure is reported but not fatal:
    // the user may legitimately configure the CLI while the server is down.
    print!("Validando conexión con el servidor... ");
    std::io::stdout().flush()?;
    match tunnel::check_server(&cfg).await {
        Ok(()) => println!("{}", "✓".green()),
        Err(e) => println!("{} {e}", "⚠".yellow()),
    }
    Ok(())
}

/// Handles `borehole start`: loads the config and runs the requested tunnel.
async fn run_start(protocol: StartCmd) -> Result<()> {
    let cfg = config::load()?;
    let (proto, local_port, remote_port) = match protocol {
        StartCmd::Tcp {
            local_port,
            remote_port,
        } => ("tcp", local_port, remote_port),
        StartCmd::Http {
            local_port,
            remote_port,
        } => ("http", local_port, remote_port),
    };

    tunnel::run(&cfg, proto, local_port, remote_port).await
}

/// Prints `label` and reads a trimmed line from stdin.
fn prompt(label: &str) -> Result<String> {
    print!("{label}");
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}
