mod control;
mod port_mgr;
mod proxy;

use std::fs;
use std::sync::Arc;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::ServerConfig;
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use control::{handle_control, ServerState};

/// Default location of the server configuration file when none is given.
const DEFAULT_CONFIG_PATH: &str = "/etc/borehole/config.json";

/// Server configuration, deserialized from a JSON file.
#[derive(Debug, Deserialize)]
struct Config {
    /// Address the control plane listens on, e.g. "0.0.0.0:7000".
    bind_control: String,
    /// Inclusive public port range `[start, end]` available for tunnels.
    port_range: [u16; 2],
    /// Valid authentication tokens.
    tokens: Vec<String>,
    /// TLS certificate and key material.
    tls: TlsConfig,
}

/// Paths to the TLS certificate chain and private key (PEM-encoded).
#[derive(Debug, Deserialize)]
struct TlsConfig {
    /// Path to the PEM-encoded certificate chain.
    cert: String,
    /// Path to the PEM-encoded private key.
    key: String,
}

#[tokio::main]
async fn main() {
    // 1. Resolve the config path from argv[1], falling back to the default.
    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_CONFIG_PATH.to_string());

    // 2. Read and parse the configuration file.
    let config = load_config(&config_path);

    // 3. Build the TLS acceptor from the configured certificate and key.
    let tls_acceptor = build_tls_acceptor(&config.tls);

    // 4. Build the shared server state.
    let state = ServerState::new(
        config.tokens,
        config.port_range[0]..=config.port_range[1],
    );

    // 5. Bind the control listener.
    let listener = match TcpListener::bind(&config.bind_control).await {
        Ok(listener) => listener,
        Err(e) => fatal(&format!("cannot bind {}: {e}", config.bind_control)),
    };
    eprintln!("borehole-server listening on {}", config.bind_control);

    // 6. Accept control connections, complete the TLS handshake on a dedicated
    //    task and dispatch each one to the control plane.
    loop {
        let (stream, _addr) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                eprintln!("accept error: {e}");
                continue;
            }
        };

        let acceptor = tls_acceptor.clone();
        let state = state.clone();
        tokio::spawn(async move {
            match acceptor.accept(stream).await {
                Ok(tls_stream) => handle_control(tls_stream, state).await,
                Err(e) => eprintln!("TLS handshake error: {e}"),
            }
        });
    }
}

/// Reads and deserializes the configuration file, terminating the process on
/// any failure.
fn load_config(path: &str) -> Config {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(e) => fatal(&format!("cannot read config {path}: {e}")),
    };
    match serde_json::from_str(&contents) {
        Ok(config) => config,
        Err(e) => fatal(&format!("invalid config {path}: {e}")),
    }
}

/// Loads the certificate and key and assembles a `TlsAcceptor`, terminating the
/// process on any failure.
fn build_tls_acceptor(tls: &TlsConfig) -> TlsAcceptor {
    let cert = match CertificateDer::from_pem_file(&tls.cert) {
        Ok(cert) => cert,
        Err(e) => fatal(&format!("cannot load cert {}: {e}", tls.cert)),
    };
    let key = match PrivateKeyDer::from_pem_file(&tls.key) {
        Ok(key) => key,
        Err(e) => fatal(&format!("cannot load key {}: {e}", tls.key)),
    };

    let server_config = match ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
    {
        Ok(server_config) => server_config,
        Err(e) => fatal(&format!("invalid TLS material: {e}")),
    };

    TlsAcceptor::from(Arc::new(server_config))
}

/// Logs a fatal error to stderr and terminates the process with status 1.
fn fatal(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}
