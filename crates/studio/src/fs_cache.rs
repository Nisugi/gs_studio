//! Answers from the filesystem, kept for a moment instead of re-asked every
//! frame. A `stat` on Windows can stall for a second or more (an antivirus
//! scan, a drive waking), and the UI asked several of them 60 times a
//! second -- that was the "Not Responding". A value is recomputed when its
//! key changes, when it is older than its time-to-live, or when invalidated
//! after something that changes the answer (a save, a pool change).

use std::time::{Duration, Instant};

pub struct Ttl<K, V> {
    ttl: Duration,
    entry: Option<(K, Instant, V)>,
}

impl<K: PartialEq, V> Ttl<K, V> {
    pub fn new(ttl: Duration) -> Self {
        Ttl { ttl, entry: None }
    }

    /// The value for `key`, computing it only when needed.
    pub fn get(&mut self, key: K, compute: impl FnOnce() -> V) -> &V {
        let fresh = self.entry.as_ref().is_some_and(|(k, at, _)| *k == key && at.elapsed() < self.ttl);
        if !fresh {
            self.entry = Some((key, Instant::now(), compute()));
        }
        &self.entry.as_ref().expect("just filled").2
    }

    /// Forget the value: the next `get` recomputes.
    pub fn invalidate(&mut self) {
        self.entry = None;
    }
}

impl<K: PartialEq, V> Default for Ttl<K, V> {
    /// Two seconds: stale answers are only ever about files, and a UI that
    /// changes them invalidates at once.
    fn default() -> Self {
        Ttl::new(Duration::from_secs(2))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_once_per_key_until_stale_or_invalidated() {
        let mut calls = 0;
        let mut c: Ttl<&str, u32> = Ttl::new(Duration::from_secs(60));
        assert_eq!(*c.get("a", || { calls += 1; 1 }), 1);
        assert_eq!(*c.get("a", || { calls += 1; 2 }), 1, "cached");
        assert_eq!(*c.get("b", || { calls += 1; 3 }), 3, "new key recomputes");
        c.invalidate();
        assert_eq!(*c.get("b", || { calls += 1; 4 }), 4, "invalidated");
        assert_eq!(calls, 3);
        let mut quick: Ttl<(), u32> = Ttl::new(Duration::ZERO);
        quick.get((), || 1);
        assert_eq!(*quick.get((), || 2), 2, "stale recomputes");
    }
}
