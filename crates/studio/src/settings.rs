//! Remembered folders: `<gs-studio home>/settings.toml`.
//!
//! Each folder resolves `--flag` > environment variable > this file > the
//! built-in default, so a flag or variable still wins for one run without
//! rewriting what was remembered. The file is written whenever a folder is
//! changed IN THE APP (Data Open, Pool Set, Publish Set, an outline import);
//! a folder that came from a flag or variable is never saved.
//!
//! It always lives in the gs-studio home (`data_home::default_home_or_local`),
//! wherever the pool points, so it can be found before anything else is
//! resolved.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const FILE_NAME: &str = "settings.toml";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The data root: the silhouette chart, `outlines.json`, `rigs/`, `poses/`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<PathBuf>,
    /// The art pool: `global/images/`, `global/scenes/`, `global/puppets/`...
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pool: Option<PathBuf>,
    /// Where Publish ships baked rigs, puppets and poses for game clients:
    /// a directory holding `rigs/`, `puppets/`, `poses/`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publish: Option<PathBuf>,
    /// The silhouette source, `scale-chart.html`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chart: Option<PathBuf>,
}

impl Settings {
    /// The settings in `home`. A missing file is the defaults; an unreadable
    /// or corrupt one is the defaults plus a message saying why.
    pub fn load(home: &Path) -> (Settings, Option<String>) {
        let path = home.join(FILE_NAME);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Settings::default(), None),
            Err(e) => return (Settings::default(), Some(format!("{}: {e}", path.display()))),
        };
        match toml::from_str(&text) {
            Ok(settings) => (settings, None),
            Err(e) => (Settings::default(), Some(format!("{} ignored: {e}", path.display()))),
        }
    }

    pub fn save(&self, home: &Path) -> Result<(), String> {
        let path = home.join(FILE_NAME);
        std::fs::create_dir_all(home).map_err(|e| format!("{}: {e}", home.display()))?;
        let text = toml::to_string(self).map_err(|e| e.to_string())?;
        let text = format!("# gs-studio: remembered folders. Flags and environment variables override these.\n{text}");
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// Where the settings file lives.
pub fn home() -> PathBuf {
    crate::data_home::default_home_or_local()
}

/// The one mutation the app makes: change one folder and write the file.
/// Returns what to show if the write failed (the change still applies for
/// this run).
pub fn remember(edit: impl FnOnce(&mut Settings)) -> Option<String> {
    let home = home();
    let (mut settings, _) = Settings::load(&home);
    edit(&mut settings);
    settings.save(&home).err().map(|e| format!("could not remember the folder: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_leaves_unset_folders_out() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Settings::load(dir.path()), (Settings::default(), None), "no file = defaults, quietly");
        let s = Settings { pool: Some("D:/art".into()), chart: Some("C:/charts/scale-chart.html".into()), ..Default::default() };
        s.save(dir.path()).unwrap();
        let text = std::fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();
        assert!(!text.contains("data") && !text.contains("publish"), "{text}");
        assert_eq!(Settings::load(dir.path()).0, s);
    }

    #[test]
    fn a_corrupt_file_is_ignored_with_a_reason() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE_NAME), "pool = [not a path").unwrap();
        let (s, why) = Settings::load(dir.path());
        assert_eq!(s, Settings::default());
        assert!(why.unwrap().contains("ignored"));
    }
}
