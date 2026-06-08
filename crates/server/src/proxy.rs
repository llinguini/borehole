// Bidirectional TCP proxy between the external visitor and the CLI data socket.
//
// `copy_bidirectional` does all the work: it forwards bytes both ways until one
// side closes, then reports how much was transferred in each direction.

use tokio::io::copy_bidirectional;
use tokio::net::TcpStream;
use tracing::{debug, info};

/// Pipes traffic between `visitor` (the external user that reached the public
/// port) and `data_conn` (the data channel opened by the CLI) until either side
/// closes the connection.
pub async fn run(mut visitor: TcpStream, mut data_conn: TcpStream) -> anyhow::Result<()> {
    // Peer addresses are best-effort: a socket may already be closed, in which
    // case we log "unknown" rather than panicking.
    let visitor_addr = visitor
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let data_addr = data_conn
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    info!("proxy start: visitor={visitor_addr} data={data_addr}");

    let (up, down) = copy_bidirectional(&mut visitor, &mut data_conn).await?;
    debug!("proxy closed: {up}↑ {down}↓ bytes");
    Ok(())
}
