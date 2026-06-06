// Bidirectional TCP proxy
//
// Splices two streams together, forwarding bytes in both directions until
// either side closes. Used to glue an external (plain TCP) visitor connection
// to the client's (TLS) data connection, so the two endpoints may differ in
// type; hence the generic parameters.

use tokio::io::{copy_bidirectional, AsyncRead, AsyncWrite};

/// Pipes all traffic between `a` and `b` until one side closes, then logs the
/// outcome. Errors are reported but not propagated, since each tunnel runs as
/// an independent task.
pub async fn pipe<A, B>(mut a: A, mut b: B)
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    match copy_bidirectional(&mut a, &mut b).await {
        Ok((a_to_b, b_to_a)) => {
            eprintln!("proxy closed: {a_to_b} bytes a->b, {b_to_a} bytes b->a");
        }
        Err(e) => {
            eprintln!("proxy error: {e}");
        }
    }
}
