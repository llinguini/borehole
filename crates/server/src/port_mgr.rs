// Port pool for tunnel assignment.
//
// `PortManager` hands out public TCP ports from a configured range to incoming
// tunnels and takes them back when tunnels close. It is thread-safe and cheap
// to `Clone` (the pool lives behind an `Arc<Mutex<..>>`), so it can be shared
// across the many concurrent connection-handling tasks.

use std::collections::HashSet;
use std::sync::Arc;

use rand::seq::IteratorRandom;
use tokio::sync::Mutex;

/// Manages the set of currently-free ports within an inclusive range.
#[derive(Clone)]
pub struct PortManager {
    /// Ports still available for assignment.
    available: Arc<Mutex<HashSet<u16>>>,
    /// Inclusive `(start, end)` bounds of the managed range.
    range: (u16, u16),
}

impl PortManager {
    /// Creates a pool with every port in `[start, end]` (inclusive) free.
    pub fn new(start: u16, end: u16) -> Self {
        let available = (start..=end).collect::<HashSet<u16>>();
        Self {
            available: Arc::new(Mutex::new(available)),
            range: (start, end),
        }
    }

    /// Assigns a random free port from the pool, or `None` if it is exhausted.
    /// Choosing randomly (rather than sequentially) avoids immediately reusing a
    /// just-released port, which can race with lingering connections.
    pub async fn acquire(&self) -> Option<u16> {
        let mut available = self.available.lock().await;
        // `ThreadRng` is dropped before any `.await`, keeping the future `Send`.
        let chosen = available.iter().copied().choose(&mut rand::thread_rng());
        if let Some(port) = chosen {
            available.remove(&port);
        }
        chosen
    }

    /// Assigns a specific port if it is within range and currently free.
    /// Returns `None` if the port is out of range or already taken.
    pub async fn acquire_specific(&self, port: u16) -> Option<u16> {
        if !self.in_range(port) {
            return None;
        }
        let mut available = self.available.lock().await;
        if available.remove(&port) {
            Some(port)
        } else {
            None
        }
    }

    /// Returns a port to the pool. Ports outside the configured range are
    /// ignored to avoid corrupting the pool.
    pub async fn release(&self, port: u16) {
        if !self.in_range(port) {
            return;
        }
        self.available.lock().await.insert(port);
    }

    /// Number of ports still available (useful for logs and health checks).
    // Not wired into logging/health endpoints yet.
    #[allow(dead_code)]
    pub async fn available_count(&self) -> usize {
        self.available.lock().await.len()
    }

    /// Whether `port` falls within the configured inclusive range.
    fn in_range(&self, port: u16) -> bool {
        port >= self.range.0 && port <= self.range.1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn acquire_returns_port_in_range() {
        let pm = PortManager::new(30000, 30010);
        let port = pm.acquire().await.expect("pool should not be empty");
        assert!((30000..=30010).contains(&port), "got out-of-range port: {port}");
    }

    #[tokio::test]
    async fn acquire_specific_out_of_range_fails() {
        let pm = PortManager::new(30000, 30010);
        assert_eq!(pm.acquire_specific(29999).await, None);
        assert_eq!(pm.acquire_specific(30011).await, None);
    }

    #[tokio::test]
    async fn acquire_specific_already_taken_fails() {
        let pm = PortManager::new(30000, 30010);
        assert_eq!(pm.acquire_specific(30005).await, Some(30005));
        assert_eq!(pm.acquire_specific(30005).await, None);
    }

    #[tokio::test]
    async fn release_returns_port_to_pool() {
        // Single-port pool makes exhaustion/reuse easy to assert.
        let pm = PortManager::new(30000, 30000);
        assert_eq!(pm.acquire().await, Some(30000));
        assert_eq!(pm.acquire().await, None, "pool must be exhausted");

        pm.release(30000).await;
        assert_eq!(pm.acquire().await, Some(30000), "released port must be reusable");
    }

    #[tokio::test]
    async fn acquire_exhausts_pool() {
        let pm = PortManager::new(30000, 30002); // three ports
        let mut seen = HashSet::new();
        for _ in 0..3 {
            seen.insert(pm.acquire().await.expect("port should be available"));
        }
        assert_eq!(seen.len(), 3, "all three distinct ports must be handed out");
        assert_eq!(pm.acquire().await, None, "pool must be exhausted");
        assert_eq!(pm.available_count().await, 0);
    }
}
