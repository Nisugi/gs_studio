//! gs-studio's OWN data home: the directory the pool falls back to when
//! nothing points it anywhere else.
//!
//! **Why this exists.** `creature_field::data_dirs` resolves its base directory to
//! `VELLUM_FE_DIR`, else `~/.vellum-fe`. That is VellumFE's default and it
//! MUST stay that way -- VellumFE is adopting the crate and depends on it.
//! But gs-studio is a standalone app, not part of VellumFE, and inheriting
//! that default meant a fresh gs-studio silently read (and could write
//! sidecars into) the user's real game-client artwork. The host sets its own
//! default instead, through the override `set_base_dir` already provides.
//!
//! The home is the platform config directory + `gs-studio`, e.g.
//! `%APPDATA%\gs-studio` on Windows, `~/.config/gs-studio` on Linux,
//! `~/Library/Application Support/gs-studio` on macOS. It holds the same
//! subtree shape the pool and the calibrators expect, so a calibrator opened
//! against it behaves exactly as one opened against `~/.vellum-fe`.
//!
//! Nothing here ever reads or writes `~/.vellum-fe`. The one path that
//! touches it at all is [`import`], which COPIES out of it on an explicit
//! request and never writes a byte back into it.

use std::path::{Path, PathBuf};

/// The directory name under the platform config dir.
pub const APP_DIR: &str = "gs-studio";

/// The pool subtree gs-studio creates on first run: every category the pool
/// listing and the calibrators read, plus the scene, rig and puppet stores.
/// `creatures` is NOT in `creature_field::pool_dirs::IMAGE_CATEGORIES` (that list
/// predates the creature field) but every creature tab needs it, so it is
/// named explicitly here rather than inferred.
pub const POOL_SUBDIRS: &[&str] = &[
    "global/images/creatures",
    "global/images/scenery",
    "global/images/scenes",
    "global/images/frames",
    "global/images/dolls",
    "global/images/icons",
    "global/images/compass",
    "global/images/backgrounds",
    "global/images/statusicons",
    "global/images/hands",
    "global/scenes",
    "global/skins",
    "global/puppets",
];

/// What `import` may copy over: the pool subtree, and only that. `global/`
/// in a real VellumFE dir also holds keybinds, colours, macros and other
/// client settings that mean nothing here.
pub const IMPORTABLE: &[&str] = &["global/images", "global/scenes", "global/skins", "global/puppets"];

/// gs-studio's own data home: `<platform config dir>/gs-studio`. `None`
/// when the platform has no config directory (then the caller falls back to
/// the working directory, never to `~/.vellum-fe`).
pub fn default_home() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_DIR))
}

/// [`default_home`], or `./gs-studio-data` when the platform will not name a
/// config directory. Never `~/.vellum-fe`.
pub fn default_home_or_local() -> PathBuf {
    default_home().unwrap_or_else(|| PathBuf::from("gs-studio-data"))
}

/// Create the pool subtree under `home` (idempotent). Errors are returned,
/// not fatal: a read-only home still runs, just empty.
pub fn ensure(home: &Path) -> std::io::Result<()> {
    for sub in POOL_SUBDIRS {
        let mut dir = home.to_path_buf();
        for seg in sub.split('/') {
            dir.push(seg);
        }
        std::fs::create_dir_all(&dir)?;
    }
    Ok(())
}

/// Does this home hold any creature art yet? Drives the first-run panel.
pub fn is_empty_pool(home: &Path) -> bool {
    let creatures = home.join("global").join("images").join("creatures");
    match std::fs::read_dir(&creatures) {
        Ok(entries) => entries.flatten().next().is_none(),
        Err(_) => true,
    }
}

// ------------------------------------------------------- VellumFE import

/// Where a VellumFE data directory would be, WITHOUT consulting
/// `creature_field::data_dirs` (whose override this app has already moved). Same
/// rule VellumFE itself uses: `VELLUM_FE_DIR`, else `~/.vellum-fe`.
pub fn vellum_fe_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("VELLUM_FE_DIR").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    dirs::home_dir().map(|h| h.join(".vellum-fe"))
}

/// A VellumFE directory worth importing from: it exists and has a `global/`.
pub fn importable_source() -> Option<PathBuf> {
    let dir = vellum_fe_dir()?;
    dir.join("global").is_dir().then_some(dir)
}

/// What an import would copy: files, bytes, and the top-level subtrees.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportPlan {
    pub files: u64,
    pub bytes: u64,
    /// The `IMPORTABLE` entries actually present, with per-subtree counts.
    pub parts: Vec<(String, u64, u64)>,
}

impl ImportPlan {
    pub fn is_empty(&self) -> bool {
        self.files == 0
    }

    /// "1,204 files, 318.2 MB".
    pub fn summary(&self) -> String {
        format!("{} files, {}", self.files, human_bytes(self.bytes))
    }
}

pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[0])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Count what `import` would copy out of `source` (a VellumFE data dir).
/// READ ONLY -- it opens directories and stats files, nothing else.
pub fn plan_import(source: &Path) -> ImportPlan {
    let mut plan = ImportPlan::default();
    for part in IMPORTABLE {
        let mut dir = source.to_path_buf();
        for seg in part.split('/') {
            dir.push(seg);
        }
        if !dir.is_dir() {
            continue;
        }
        let (files, bytes) = measure_tree(&dir);
        if files > 0 {
            plan.files += files;
            plan.bytes += bytes;
            plan.parts.push(((*part).to_string(), files, bytes));
        }
    }
    plan
}

fn measure_tree(dir: &Path) -> (u64, u64) {
    let (mut files, mut bytes) = (0u64, 0u64);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(path),
                Ok(t) if t.is_file() => {
                    files += 1;
                    bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
                }
                _ => {}
            }
        }
    }
    (files, bytes)
}

/// What an import did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportReport {
    pub copied: u64,
    pub bytes: u64,
    /// Files already present in the home and left alone.
    pub skipped: u64,
    pub errors: Vec<String>,
}

impl ImportReport {
    pub fn summary(&self) -> String {
        let mut s = format!("Imported {} files ({})", self.copied, human_bytes(self.bytes));
        if self.skipped > 0 {
            s.push_str(&format!(", {} already here", self.skipped));
        }
        if !self.errors.is_empty() {
            s.push_str(&format!(", {} failed", self.errors.len()));
        }
        s
    }
}

/// COPY `source`'s pool subtrees into `home`.
///
/// Never moves, never symlinks, and never opens `source` for writing: every
/// operation is a read there and a create here. A file that already exists
/// in `home` is left untouched (counted as skipped), so an import cannot
/// clobber work done in the studio.
pub fn import(source: &Path, home: &Path) -> ImportReport {
    let mut report = ImportReport::default();
    for part in IMPORTABLE {
        let (mut from, mut to) = (source.to_path_buf(), home.to_path_buf());
        for seg in part.split('/') {
            from.push(seg);
            to.push(seg);
        }
        if from.is_dir() {
            copy_tree(&from, &to, &mut report);
        }
    }
    report
}

fn copy_tree(from: &Path, to: &Path, report: &mut ImportReport) {
    if let Err(e) = std::fs::create_dir_all(to) {
        report.errors.push(format!("{}: {e}", to.display()));
        return;
    }
    let Ok(entries) = std::fs::read_dir(from) else {
        report.errors.push(format!("{}: unreadable", from.display()));
        return;
    };
    for entry in entries.flatten() {
        let src = entry.path();
        let dst = to.join(entry.file_name());
        match entry.file_type() {
            Ok(t) if t.is_dir() => copy_tree(&src, &dst, report),
            Ok(t) if t.is_file() => {
                if dst.exists() {
                    report.skipped += 1;
                    continue;
                }
                match std::fs::copy(&src, &dst) {
                    Ok(n) => {
                        report.copied += 1;
                        report.bytes += n;
                    }
                    Err(e) => report.errors.push(format!("{}: {e}", src.display())),
                }
            }
            // Symlinks and anything else: skipped, deliberately. A link
            // into ~/.vellum-fe would defeat the point of copying.
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_home_is_never_the_vellum_fe_directory() {
        let home = default_home_or_local();
        let text = home.to_string_lossy().to_ascii_lowercase();
        assert!(!text.contains(".vellum-fe"), "{home:?}");
        assert!(text.ends_with(APP_DIR), "{home:?}");
    }

    #[test]
    fn ensure_creates_the_pool_shape_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("gs-studio");
        ensure(&home).unwrap();
        ensure(&home).unwrap();
        for sub in POOL_SUBDIRS {
            let mut p = home.clone();
            for seg in sub.split('/') {
                p.push(seg);
            }
            assert!(p.is_dir(), "{sub}");
        }
        assert!(is_empty_pool(&home));
        std::fs::write(home.join("global/images/creatures/kobold.png"), b"x").unwrap();
        assert!(!is_empty_pool(&home));
    }

    #[test]
    fn a_missing_home_reads_as_an_empty_pool() {
        assert!(is_empty_pool(Path::new("/no/such/gs-studio-home")));
    }

    fn fake_vellum(root: &Path) {
        std::fs::create_dir_all(root.join("global/images/creatures/kobold")).unwrap();
        std::fs::create_dir_all(root.join("global/scenes")).unwrap();
        std::fs::create_dir_all(root.join("gui")).unwrap();
        std::fs::write(root.join("global/images/creatures/coyote.png"), vec![7u8; 100]).unwrap();
        std::fs::write(root.join("global/images/creatures/kobold/kobold.png"), vec![7u8; 50]).unwrap();
        std::fs::write(root.join("global/scenes/default.toml"), b"x = 1").unwrap();
        // Client settings and logs: not pool content, never imported.
        std::fs::write(root.join("global/keybinds.toml"), b"junk").unwrap();
        std::fs::write(root.join("vellum-fe.log"), b"junk").unwrap();
        std::fs::write(root.join("gui/layout.toml"), b"junk").unwrap();
    }

    #[test]
    fn the_plan_counts_only_the_pool_subtrees() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join(".vellum-fe");
        fake_vellum(&source);
        let plan = plan_import(&source);
        assert_eq!((plan.files, plan.bytes), (3, 155));
        assert_eq!(
            plan.parts.iter().map(|(p, _, _)| p.as_str()).collect::<Vec<_>>(),
            ["global/images", "global/scenes"]
        );
        assert!(plan.summary().starts_with("3 files, 155 B"), "{}", plan.summary());
        assert!(plan_import(Path::new("/no/such/dir")).is_empty());
    }

    #[test]
    fn import_copies_the_pool_and_leaves_the_source_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join(".vellum-fe");
        let home = dir.path().join("gs-studio");
        fake_vellum(&source);
        let before = super::tests::snapshot(&source);

        ensure(&home).unwrap();
        let report = import(&source, &home);
        assert_eq!((report.copied, report.bytes, report.skipped), (3, 155, 0));
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert!(home.join("global/images/creatures/coyote.png").is_file());
        assert!(home.join("global/images/creatures/kobold/kobold.png").is_file());
        assert!(home.join("global/scenes/default.toml").is_file());
        // Client settings did not come along.
        assert!(!home.join("global/keybinds.toml").exists());
        assert!(!home.join("gui").exists());
        assert!(!home.join("vellum-fe.log").exists());
        assert!(!is_empty_pool(&home));

        // Re-running skips what is already here, and writes nothing new.
        let again = import(&source, &home);
        assert_eq!((again.copied, again.skipped), (0, 3));

        // The source is byte-for-byte what it was: nothing moved, nothing
        // rewritten, nothing added.
        assert_eq!(super::tests::snapshot(&source), before);
    }

    /// Every file under `root`, relative, with its length -- the "nothing
    /// was touched" proof for the import tests.
    pub fn snapshot(root: &Path) -> Vec<(String, u64)> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if let Ok(meta) = entry.metadata() {
                    let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
                    out.push((rel, meta.len()));
                }
            }
        }
        out.sort();
        out
    }
}
