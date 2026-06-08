// borehole-server entry point.
//
// Loads the configuration and TLS material, then accepts CLI control
// connections on the control port and dispatches each to `control::handle_client`.

mod api;
mod config;
mod control;
mod devices;
mod events;
mod history;
mod node_manager;
mod port_mgr;
mod proxy;

use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader as StdBufReader;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::routing::{delete, get, post};
use axum::Router;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_rustls::TlsAcceptor;
use tower_http::cors::CorsLayer;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use config::{ServerConfig, TlsConfig};
use control::ServerState;
use events::EventBus;
use history::History;
use node_manager::NodeManager;
use port_mgr::PortManager;

/// Default configuration path used when `--config` is not supplied.
const DEFAULT_CONFIG_PATH: &str = "/etc/borehole/config.json";

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    // rustls 0.23 needs a process-level crypto provider; standardise on
    // aws-lc-rs. Ignore the error if one is already installed.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let config_path = parse_config_path();
    let config = ServerConfig::load(&config_path)?;

    let acceptor = build_tls_acceptor(&config.tls)?;

    let state = Arc::new(ServerState {
        port_mgr: PortManager::new(config.port_range[0], config.port_range[1]),
        tokens: Arc::new(config.tokens.clone()),
        pending: Arc::new(Mutex::new(HashMap::new())),
        node_manager: NodeManager::new(),
        active_tunnels: Arc::new(Mutex::new(HashMap::new())),
        jwt_secret: config.jwt_secret.clone(),
        users: Arc::new(config.users.clone()),
        event_bus: Arc::new(EventBus::new()),
        api_tokens: Arc::new(Mutex::new(HashMap::new())),
        devices: Arc::new(Mutex::new(Vec::new())),
        history: Arc::new(History::new()),
    });

    // HTTP server: the REST API under `/api` and the embedded dashboard for
    // everything else. CORS stays permissive so a separate dev dashboard
    // (`npm run dev` on another origin) can still call the API.
    let api_router = Router::new()
        .route("/auth/login", post(api::auth::login))
        .route("/auth/cli", get(api::auth::cli_login_page))
        .route("/tunnels", get(api::tunnels::list))
        .route("/tunnels/:id", delete(api::tunnels::close))
        .route("/events", get(api::tunnels::stream_events))
        .route("/nodes", get(api::nodes::list))
        .route("/tokens", get(api::tokens::list).post(api::tokens::create))
        .route("/tokens/:id", delete(api::tokens::revoke))
        .route("/devices", get(api::devices::list))
        .route("/history/tunnels", get(api::history::tunnels))
        .route("/history/connections", get(api::history::connections))
        .layer(CorsLayer::permissive())
        .with_state(state.clone());

    // API takes priority; any other path is served by the SPA (with a 404 when
    // the dashboard was not built into this binary).
    let app = Router::new()
        .nest("/api", api_router)
        .fallback_service(api::static_router());

    let http_listener = TcpListener::bind(&config.bind_http)
        .await
        .with_context(|| format!("cannot bind HTTP listener on {}", config.bind_http))?;
    info!("HTTP API listening on {}", config.bind_http);
    tokio::spawn(async move {
        if let Err(e) = axum::serve(http_listener, app).await {
            warn!("HTTP API server error: {e}");
        }
    });

    info!("borehole-server starting on {}", config.bind_control);
    let listener = TcpListener::bind(&config.bind_control)
        .await
        .with_context(|| format!("cannot bind control listener on {}", config.bind_control))?;
    info!("listening for control connections");

    // Plain-TCP data plane on the control port + 1. The CLI dials this port for
    // each visitor, identifying the connection with a `DataConn` frame.
    let data_addr = format!("{}:{}", control_host(&config.bind_control), config.data_port());
    let data_listener = TcpListener::bind(&data_addr)
        .await
        .with_context(|| format!("cannot bind data listener on {data_addr}"))?;
    info!("listening for data connections on {data_addr}");
    {
        let state = state.clone();
        tokio::spawn(async move { control::handle_data_listener(data_listener, state).await });
    }

    loop {
        let (tcp, peer) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                warn!("accept error: {e}");
                continue;
            }
        };

        let acceptor = acceptor.clone();
        let state = state.clone();
        tokio::spawn(async move {
            let tls = match acceptor.accept(tcp).await {
                Ok(stream) => stream,
                Err(e) => {
                    warn!("TLS handshake failed from {peer}: {e}");
                    return;
                }
            };
            if let Err(e) = control::handle_client(tls, state).await {
                warn!("client {peer} error: {e}");
            }
        });
    }
}

/// Initializes the global tracing subscriber. Honors `RUST_LOG`, defaulting to
/// `info` when it is unset or invalid.
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

/// Extracts the host part of a `host:port` bind address (used to bind the data
/// listener on the same interface as the control listener).
fn control_host(bind: &str) -> &str {
    bind.rsplit_once(':').map(|(host, _)| host).unwrap_or(bind)
}

/// Returns the path passed via `--config <path>`, or the default when absent.
fn parse_config_path() -> String {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--config" {
            if let Some(path) = args.next() {
                return path;
            }
        }
    }
    DEFAULT_CONFIG_PATH.to_string()
}

/// Builds a `TlsAcceptor` from the configured certificate chain and key.
fn build_tls_acceptor(tls: &TlsConfig) -> Result<TlsAcceptor> {
    let certs = load_certs(&tls.cert)?;
    let key = load_key(&tls.key)?;
    let server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("invalid certificate or key")?;
    Ok(TlsAcceptor::from(Arc::new(server_config)))
}

/// Loads the full certificate chain (leaf + intermediates) from a PEM file.
fn load_certs(path: &str) -> Result<Vec<CertificateDer<'static>>> {
    let file = File::open(path).with_context(|| format!("cannot open cert file: {path}"))?;
    let mut reader = StdBufReader::new(file);
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut reader)
        .collect::<std::result::Result<_, _>>()
        .with_context(|| format!("failed to parse certs in {path}"))?;
    if certs.is_empty() {
        anyhow::bail!("no certificates found in {path}");
    }
    Ok(certs)
}

/// Loads the first private key found in a PEM file.
fn load_key(path: &str) -> Result<PrivateKeyDer<'static>> {
    let file = File::open(path).with_context(|| format!("cannot open key file: {path}"))?;
    let mut reader = StdBufReader::new(file);
    rustls_pemfile::private_key(&mut reader)
        .with_context(|| format!("failed to parse key in {path}"))?
        .ok_or_else(|| anyhow::anyhow!("no private key found in {path}"))
}
