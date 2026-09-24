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
}
