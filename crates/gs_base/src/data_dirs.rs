//! Data-directory resolution as free functions.
//!
//! Everything here needs only the `VELLUM_FE_DIR` env var and
//! `dirs::home_dir()` — no `Config` instance, no profile, no settings. The
//! asset-format modules (`skins`, `scenes`, `pool`, `creature_field`) resolve
//! their paths through this module so they do not depend on the application's
//! `Config` type. `Config::base_dir()` and friends (see `paths.rs`) are
//! one-line wrappers over these.
//!
//! The field's own folders (skins, scenes, images...) and the atomic file
//! writer are `pool_dirs` (split out: the base dir is what the doll and the
//! field share, docs/crate-split-plan.md).
//!
//! Ported from VellumFE `src/config/data_dirs.rs` (commit 60c84a79). Added
//! here: [`set_base_dir`] / [`clear_base_dir`], a process-wide override that
//! is checked BEFORE the env var, so hosts other than VellumFE (gs-studio, a
//! second client) can point the asset store elsewhere. With no override set
//! the resolution is exactly VellumFE's.

use std::path::PathBuf;

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


/// Process-wide base-dir override; see [`set_base_dir`].
static BASE_DIR_OVERRIDE: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);

/// Point every path in this module at `dir` for the rest of the process,
/// ahead of `VELLUM_FE_DIR` and the home-directory default. May be called
/// again to move it. A cache of what is under it keys on
/// [`base_generation`], which this bumps (the field's image pool also wants
/// its `pool::invalidate_cache()`).
pub fn set_base_dir(dir: PathBuf) {
    *BASE_DIR_OVERRIDE.write().unwrap_or_else(|e| e.into_inner()) = Some(dir);
    BASE_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Bumped each time the base dir is set or cleared: a cache of what is
/// under it (the doll's figures, the field's puppets) keys on it and reads
/// afresh when it moves (ADDITION).
static BASE_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many times the base dir has moved ([`set_base_dir`], [`clear_base_dir`]).
pub fn base_generation() -> u64 {
    BASE_GENERATION.load(std::sync::atomic::Ordering::Relaxed)
}

/// Drop the [`set_base_dir`] override; resolution falls back to
/// `VELLUM_FE_DIR`, then `~/.vellum-fe`.
pub fn clear_base_dir() {
    *BASE_DIR_OVERRIDE.write().unwrap_or_else(|e| e.into_inner()) = None;
    BASE_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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

/// Where the pool keeps modelled bodies (ADDITION): `global/bodies/<form>/`
/// holds `body.obj` (or `body_thin.obj`), `color.png`, `normal.png`,
/// `weights.txt`, `splat.spz` (or `.ply`) and `doll.toml`, as
/// `<data>/meshes/<form>/<size>/` has them (gs-studio installs the largest
/// size). The injury doll's figures, and any form's body. Not compiled in:
/// too big.
pub fn bodies_dir() -> Result<PathBuf> {
    Ok(global_dir()?.join("bodies"))
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
        assert_eq!(global_dir().unwrap(), PathBuf::from("/from/override").join("global"));

        clear_base_dir();
        assert_eq!(base_dir().unwrap(), PathBuf::from("/from/env"));
        std::env::remove_var("VELLUM_FE_DIR");
    }
}
