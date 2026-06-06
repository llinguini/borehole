// Port pool manager
//
// Tracks which TCP ports are free and which are handed out to active tunnels.
// This type is intentionally `std`-only and not thread-safe; the caller is
// expected to wrap it in a `Mutex` when shared across async tasks.

use std::collections::{BTreeSet, HashSet};
use std::ops::RangeInclusive;

/// Manages a finite pool of TCP ports, allowing callers to reserve a specific
/// or arbitrary free port and release it back when the tunnel closes.
pub struct PortManager {
    /// Ports currently free, ordered so the lowest can be picked cheaply.
    available: BTreeSet<u16>,
    /// Ports currently reserved by a tunnel.
    in_use: HashSet<u16>,
}

impl PortManager {
    /// Builds a manager whose available pool contains every port in `range`.
    pub fn new(range: RangeInclusive<u16>) -> Self {
        Self {
            available: range.collect(),
            in_use: HashSet::new(),
        }
    }

    /// Reserves a port and moves it from `available` to `in_use`.
    ///
    /// - `Some(p)`: tries to reserve exactly `p`; returns `Some(p)` if it was
    ///   free, `None` if it was taken (or outside the pool).
    /// - `None`: reserves and returns the lowest available port, or `None` if
    ///   the pool is exhausted.
    pub fn acquire(&mut self, requested: Option<u16>) -> Option<u16> {
        let port = match requested {
            Some(p) => {
                if self.available.remove(&p) {
                    p
                } else {
                    return None;
                }
            }
            None => {
                let p = *self.available.iter().next()?;
                self.available.remove(&p);
                p
            }
        };

        self.in_use.insert(port);
        Some(port)
    }

    /// Returns `port` to the available pool. No-op if it was not in use.
    pub fn release(&mut self, port: u16) {
        if self.in_use.remove(&port) {
            self.available.insert(port);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_random_returns_lowest_port() {
        let mut mgr = PortManager::new(30000..=30002);

        assert_eq!(mgr.acquire(None), Some(30000));
    }

    #[test]
    fn acquire_specific_reserves_exact_port() {
        let mut mgr = PortManager::new(30000..=30002);

        assert_eq!(mgr.acquire(Some(30001)), Some(30001));
    }

    #[test]
    fn acquire_busy_port_returns_none() {
        let mut mgr = PortManager::new(30000..=30002);

        assert_eq!(mgr.acquire(Some(30001)), Some(30001));
        assert_eq!(mgr.acquire(Some(30001)), None);
    }

    #[test]
    fn release_returns_port_to_pool() {
        let mut mgr = PortManager::new(30000..=30002);

        let port = mgr.acquire(Some(30000)).expect("port must be free");
        assert_eq!(mgr.acquire(Some(30000)), None);

        mgr.release(port);
        assert_eq!(mgr.acquire(Some(30000)), Some(30000));
    }
}
