//! Data-directory resolution as free functions.
//!
//! Everything here needs only the `VELLUM_FE_DIR` env var and
//! `dirs::home_dir()` — no `Config` instance, no profile, no settings. The
//! asset-format modules (`skins`, `scenes`, `pool`, `creature_field`) resolve
//! their paths through this module so they do not depend on the application's
//! `Config` type. `Config::base_dir()` and friends (see `paths.rs`) are
//! one-line wrappers over these.
//!
//! Ported from VellumFE `src/config/data_dirs.rs` (commit 60c84a79). Added
//! here: [`set_base_dir`] / [`clear_base_dir`], a process-wide override that
//! is checked BEFORE the env var, so hosts other than VellumFE (gs-studio, a
//! second client) can point the asset store elsewhere. With no override set
//! the resolution is exactly VellumFE's.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// One process-wide lock for tests that set the global `VELLUM_FE_DIR` env
/// var. Every such test across every module must serialize on THIS lock, not
/// a per-module one — separate mutexes guarding the same global don't mutually
/// exclude, so tests would race (and one panic would poison only its own lock,
/// cascading failures). Guard the whole set/use/remove with it.
///
/// Not `#[cfg(test)]` here (it was in VellumFE): a dependent crate's tests
/// build this crate WITHOUT `cfg(test)`, and they need the same lock. Tests
/// that call [`set_base_dir`] serialize on it too.
pub static VELLUM_FE_DIR_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Write a user config file safely: write to a sibling `.tmp` file, back up
/// the existing file to `<name>.bak`, then rename over the target. A crash
/// mid-write can no longer truncate user data, and the previous version is
/// always one rename away.
///
/// Use this for every file a user (or an editor UI) authors. First-run
/// default extraction and cache files don't need it.
pub fn write_atomic(path: &Path, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, contents)?;
    if path.exists() {
        let mut bak = path.as_os_str().to_owned();
        bak.push(".bak");
        // Best-effort backup: a failed copy must not block the save itself.
        let _ = std::fs::copy(path, PathBuf::from(bak));
    }
    std::fs::rename(&tmp, path)
}

/// Process-wide base-dir override; see [`set_base_dir`].
static BASE_DIR_OVERRIDE: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);

/// Point every path in this module at `dir` for the rest of the process,
/// ahead of `VELLUM_FE_DIR` and the home-directory default. May be called
/// again to move it. Callers that cache scans ([`crate::pool`]) should
/// `pool::invalidate_cache()` after moving it.
pub fn set_base_dir(dir: PathBuf) {
    *BASE_DIR_OVERRIDE.write().unwrap_or_else(|e| e.into_inner()) = Some(dir);
}

/// Drop the [`set_base_dir`] override; resolution falls back to
/// `VELLUM_FE_DIR`, then `~/.vellum-fe`.
pub fn clear_base_dir() {
    *BASE_DIR_OVERRIDE.write().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Get the base vellum-fe directory (~/.vellum-fe/)
/// Can be overridden with VELLUM_FE_DIR environment variable, and before
/// that by [`set_base_dir`].
pub fn base_dir() -> Result<PathBuf> {
    if let Some(dir) = BASE_DIR_OVERRIDE
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
    {
        return Ok(dir);
    }

    // Check for custom directory from environment variable
    if let Ok(custom_dir) = std::env::var("VELLUM_FE_DIR") {
        return Ok(PathBuf::from(custom_dir));
    }

    // Default to ~/.vellum-fe
    let home = dirs::home_dir().context("Could not find home directory")?;
    Ok(home.join(".vellum-fe"))
}

/// Get the global directory (for all shared resources)
/// Returns: ~/.vellum-fe/global/
pub fn global_dir() -> Result<PathBuf> {
    Ok(base_dir()?.join("global"))
}

/// Get the shared skins directory (one subdirectory per skin, each with a
/// skin.toml manifest plus any skin-local image assets)
/// Returns: ~/.vellum-fe/global/skins/
pub fn skins_dir() -> Result<PathBuf> {
    Ok(global_dir()?.join("skins"))
}

/// Category subfolders of the shared image pool. Created at startup so
/// the structure is visible; nothing enforces which category a file
/// lives in (resolution is by relative path).
pub const IMAGE_CATEGORIES: &[&str] = &[
    "icons",
    "frames",
    "dolls",
    "compass",
    "backgrounds",
    "statusicons",
    "hands",
    "scenes",
    "scenery",
];

/// Get the shared stage-scene store: one `<name>.toml` per authored
/// scene (background + scenery props; see `config::scenes`).
/// Returns: ~/.vellum-fe/global/scenes/
pub fn scenes_dir() -> Result<PathBuf> {
    Ok(global_dir()?.join("scenes"))
}

/// Get the shared image pool: one subfolder per category
/// (see IMAGE_CATEGORIES). Skin manifests resolve relative image paths
/// against the skin folder first, then this pool — so skins can share
/// art without copying it.
/// Returns: ~/.vellum-fe/global/images/
pub fn global_images_dir() -> Result<PathBuf> {
    Ok(global_dir()?.join("images"))
}

/// Get the shared hotbar icon-sheet store: sheet images plus an
/// icons.toml manifest, available to every skin (and with no skin active)
/// Returns: ~/.vellum-fe/global/images/icons/
pub fn global_icons_dir() -> Result<PathBuf> {
    Ok(global_images_dir()?.join("icons"))
}

/// Get the data-pack local store (gameobj-data.xml and friends; see
/// `core::data_pack`)
/// Returns: ~/.vellum-fe/global/data/
pub fn global_data_dir() -> Result<PathBuf> {
    Ok(global_dir()?.join("data"))
}

/// Get the shared injury-doll base-image pool. Standalone doll base
/// images (installed by `.jinx`) drop in here; a skin's
/// `[injury_doll] base` can reference one as `dolls/<file>` (resolved
/// through the shared image pool) or by absolute path.
/// Returns: ~/.vellum-fe/global/images/dolls/
pub fn global_dolls_dir() -> Result<PathBuf> {
    Ok(global_images_dir()?.join("dolls"))
}

/// One shared-image-pool category folder (see IMAGE_CATEGORIES). Skins
/// reference pool art by relative path ("frames/iron.png"); `.jinx`
/// installs the per-file asset kinds here.
/// Returns: ~/.vellum-fe/global/images/<category>/
pub fn global_image_category_dir(category: &str) -> Result<PathBuf> {
    Ok(global_images_dir()?.join(category))
}

#[cfg(test)]
mod atomic_tests {
    use super::write_atomic;

    #[test]
    fn write_atomic_creates_file_and_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        write_atomic(&path, "a = 1\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a = 1\n");
    }

    #[test]
    fn write_atomic_backs_up_previous_contents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_atomic(&path, "old\n").unwrap();
        write_atomic(&path, "new\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\n");
        let bak = dir.path().join("config.toml.bak");
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), "old\n");
        // No stray temp file left behind.
        assert!(!dir.path().join("config.toml.tmp").exists());
    }
}

#[cfg(test)]
mod base_dir_tests {
    use super::*;

    #[test]
    fn override_wins_over_env_and_clears_back_to_it() {
        let _guard = VELLUM_FE_DIR_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::env::set_var("VELLUM_FE_DIR", "/from/env");
        assert_eq!(base_dir().unwrap(), PathBuf::from("/from/env"));

        set_base_dir(PathBuf::from("/from/override"));
        assert_eq!(base_dir().unwrap(), PathBuf::from("/from/override"));
        assert_eq!(
            global_images_dir().unwrap(),
            PathBuf::from("/from/override").join("global").join("images")
        );

        clear_base_dir();
        assert_eq!(base_dir().unwrap(), PathBuf::from("/from/env"));
        std::env::remove_var("VELLUM_FE_DIR");
    }
}
