//! A watchdog for "Not Responding": frames that take too long, and the
//! heavy actions known to run on the UI thread, are appended to
//! `<gs-studio home>/slow_frames.log` (and stderr) with a timestamp. A
//! freeze that never returns still leaves the "begin" line of whatever was
//! running.

use std::io::Write as _;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A frame slower than this is logged.
pub const SLOW_FRAME: Duration = Duration::from_millis(250);

pub const FILE_NAME: &str = "slow_frames.log";

/// Append one line, timestamped, to the log (and stderr). Never fails the
/// caller: a log that cannot be written is just skipped.
pub fn log(line: &str) {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    let text = format!("{secs:.3} {line}");
    eprintln!("gs-studio: {text}");
    let path = crate::data_home::default_home_or_local().join(FILE_NAME);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{text}");
    }
}

thread_local! {
    /// This frame's timed parts: (name, time). Cleared as a frame starts.
    static SPANS: std::cell::RefCell<Vec<(&'static str, Duration)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Parts faster than this are not worth naming.
const SPAN_MIN: Duration = Duration::from_millis(15);

/// Times a part of the frame until dropped: `let _t = perf_log::span("bake");`.
pub struct Span {
    name: &'static str,
    started: std::time::Instant,
}

pub fn span(name: &'static str) -> Span {
    Span { name, started: std::time::Instant::now() }
}

impl Drop for Span {
    fn drop(&mut self) {
        let took = self.started.elapsed();
        if took >= SPAN_MIN {
            SPANS.with(|s| s.borrow_mut().push((self.name, took)));
        }
    }
}

/// A frame starts: forget the last one's parts.
pub fn begin_frame() {
    SPANS.with(|s| s.borrow_mut().clear());
}

/// This frame's slowest parts, e.g. "rigs: skin 7900 ms, rigs: bake 40 ms".
pub fn spans_detail() -> String {
    SPANS.with(|s| {
        let mut v = s.borrow().clone();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        v.iter().take(5).map(|(n, d)| format!("{n} {} ms", d.as_millis())).collect::<Vec<_>>().join(", ")
    })
}

/// The line for a slow frame, or None when it was fast enough.
pub fn slow_frame_line(took: Duration, tab: &str, detail: &str) -> Option<String> {
    (took >= SLOW_FRAME).then(|| format!("slow frame: {} ms on {tab}{}", took.as_millis(), if detail.is_empty() { String::new() } else { format!(" ({detail})") }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_slow_frames_are_reported() {
        assert_eq!(slow_frame_line(Duration::from_millis(40), "Rigs", ""), None);
        let line = slow_frame_line(Duration::from_millis(1200), "Dolls", "sets").unwrap();
        assert_eq!(line, "slow frame: 1200 ms on Dolls (sets)");
    }

    #[test]
    fn slow_parts_are_named_and_fast_ones_are_not() {
        begin_frame();
        {
            let _t = span("slow part");
            std::thread::sleep(Duration::from_millis(30));
        }
        {
            let _t = span("fast part");
        }
        let detail = spans_detail();
        assert!(detail.starts_with("slow part ") && !detail.contains("fast"), "{detail}");
        begin_frame();
        assert_eq!(spans_detail(), "");
    }
}
