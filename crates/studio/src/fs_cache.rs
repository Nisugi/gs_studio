//! Answers from the filesystem, kept for a moment instead of re-asked every
//! frame. A `stat` on Windows can stall for a second or more (an antivirus
//! scan, a drive waking), and the UI asked several of them 60 times a
//! second -- that was the "Not Responding". A value is recomputed when its
//! key changes, when it is older than its time-to-live, or when invalidated
//! after something that changes the answer (a save, a pool change).
//!
//! [`Ttl`] recomputes on the calling thread, so even every two seconds a
//! stalled `stat` freezes the frame that asks (it did: 3.5 and 9 second
//! frames in the Rigs tab's document controls). [`Background`] never
//! blocks: it asks on a worker thread and hands back the last answer it has.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A filesystem answer computed OFF the UI thread. [`Background::get`]
/// returns the last answer for the key (None until the first arrives, or
/// when the key changed) and starts a worker when that answer is stale.
pub struct Background<K, V> {
    ttl: Duration,
    done: Option<(K, Instant, V)>,
    job: Option<(K, Arc<Mutex<Option<V>>>)>,
}

impl<K: PartialEq + Clone + Send + 'static, V: Send + 'static> Background<K, V> {
    pub fn new(ttl: Duration) -> Self {
        Background { ttl, done: None, job: None }
    }

    /// The last answer for `key`, never waiting for the filesystem.
    pub fn get(&mut self, key: K, compute: impl FnOnce() -> V + Send + 'static) -> Option<&V> {
        let finished = self.job.as_ref().and_then(|(k, slot)| slot.lock().ok()?.take().map(|v| (k.clone(), v)));
        if let Some((k, v)) = finished {
            self.done = Some((k, Instant::now(), v));
            self.job = None;
        }
        let fresh = self.done.as_ref().is_some_and(|(k, at, _)| *k == key && at.elapsed() < self.ttl);
        let asking = self.job.as_ref().is_some_and(|(k, _)| *k == key);
        if !fresh && !asking {
            let slot = Arc::new(Mutex::new(None));
            let out = slot.clone();
            std::thread::spawn(move || {
                let v = compute();
                if let Ok(mut s) = out.lock() {
                    *s = Some(v);
                }
            });
            self.job = Some((key.clone(), slot));
        }
        self.done.as_ref().filter(|(k, _, _)| *k == key).map(|(_, _, v)| v)
    }

    /// The answer is out of date (a save changed it): ask again, still
    /// showing the old one until the new one arrives.
    pub fn invalidate(&mut self) {
        if let Some((_, at, _)) = &mut self.done {
            *at = Instant::now().checked_sub(self.ttl * 2).unwrap_or(*at);
        }
    }

    /// A worker is out: the caller should draw again soon to pick it up.
    pub fn pending(&self) -> bool {
        self.job.is_some()
    }
}

impl<K: PartialEq + Clone + Send + 'static, V: Send + 'static> Default for Background<K, V> {
    /// Two seconds, as [`Ttl`].
    fn default() -> Self {
        Background::new(Duration::from_secs(2))
    }
}

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
    #[cfg_attr(not(test), allow(dead_code))]
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
    fn background_answers_arrive_without_blocking_the_caller() {
        let mut c: Background<&str, u32> = Background::new(Duration::from_secs(60));
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let t = Instant::now();
        // The worker waits to be let go: `get` must not wait for it.
        assert_eq!(c.get("a", move || { rx.recv().ok(); 7 }), None);
        assert!(t.elapsed() < Duration::from_millis(200) && c.pending());
        tx.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while c.get("a", || 99).is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(c.get("a", || 99), Some(&7), "the first answer, kept");
        assert!(!c.pending());
        c.invalidate();
        assert_eq!(c.get("a", || 8), Some(&7), "stale is still shown while it is asked again");
        assert_eq!(c.get("b", || 1), None, "another key has no answer yet");
    }

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
