//! What every kind of note shares, without the drawing: a reply under a
//! note, who wrote it, and the date. Rig notes (`editor::rig_notes`, pinned
//! to a puppet) and scene notes (`stage::scene_notes`, pinned to the stage)
//! are the same conversation -- an observation, then replies, then done --
//! between whoever is working on the thing, people and Claude alike.

use serde::{Deserialize, Serialize};

/// One reply under a note: who, what, when.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub by: String,
    pub text: String,
    /// `YYYY-MM-DD`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub made: String,
}

impl Reply {
    pub fn new(by: &str, text: &str, made: &str) -> Reply {
        Reply { by: by.trim().to_owned(), text: text.trim().to_owned(), made: made.to_owned() }
    }
}

/// The name a new note or reply is signed with: Settings' *Your name*
/// (`author` in `settings.toml`), else nothing.
pub fn author() -> String {
    let (settings, _) = crate::settings::Settings::load(&crate::settings::home());
    settings.author.unwrap_or_default().trim().to_owned()
}

/// Today's date, `YYYY-MM-DD` (UTC), for a new note or reply.
pub fn today() -> String {
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() / 86_400) as i64;
    // days since 1970-01-01 to a civil date (H. Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Write `text` to `path` through a temporary beside it, renamed over it, so
/// a failed write never leaves half a file.
pub fn write_atomic(path: &std::path::Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn today_is_a_date() {
        let d = today();
        assert_eq!(d.len(), 10);
        assert!(d.starts_with("20") && &d[4..5] == "-" && &d[7..8] == "-", "{d}");
    }

    #[test]
    fn a_reply_trims_and_leaves_empty_fields_out() {
        let r = Reply::new("  ", " looks right now ", "2026-09-28");
        assert_eq!((r.by.as_str(), r.text.as_str()), ("", "looks right now"));
        let text = toml::to_string(&r).unwrap();
        assert!(!text.contains("by"), "{text}");
        assert_eq!(toml::from_str::<Reply>(&text).unwrap(), r);
    }
}
