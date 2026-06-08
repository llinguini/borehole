// Bidirectional TCP proxy.
//
// Splices two streams together, forwarding bytes in both directions until either
// side closes. On a node both ends are plain `TcpStream`s (the external visitor
// and the CLI data connection), but the function stays generic.

use tokio::io::{copy_bidirectional, AsyncRead, AsyncWrite};
use tracing::{debug, warn};

/// Pipes all traffic between `a` and `b` until one side closes. Errors are
/// logged, not propagated: each tunnel runs as an independent task.
pub async fn pipe<A, B>(mut a: A, mut b: B)
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    match copy_bidirectional(&mut a, &mut b).await {
        Ok((up, down)) => debug!("proxy closed: {up}↑ {down}↓ bytes"),
        Err(e) => warn!("proxy error: {e}"),
    }
}
