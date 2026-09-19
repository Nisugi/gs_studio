//! Where the art pool is: the directory holding `global/images/...` and
//! `global/scenes/` -- gs_field's "base dir", which is VellumFE's data
//! directory. Precedence: `--pool <dir>`, then the `GS_STUDIO_POOL`
//! environment variable, then gs_field's own default (`VELLUM_FE_DIR`,
//! else `~/.vellum-fe`).
//!
//! This is NOT the data root (`data_root.rs`): that one holds
//! `outlines.json` and `rigs/` and belongs to the rig editor.

use std::path::{Path, PathBuf};

pub const ENV_VAR: &str = "GS_STUDIO_POOL";

#[derive(Debug, Clone, PartialEq)]
pub struct PoolDir {
    pub path: PathBuf,
    /// Where the path came from, for the status line.
    pub source: &'static str,
    /// True when gs_field must be told (`set_base_dir`); false when the
    /// path IS gs_field's default and nothing needs overriding.
    pub overrides: bool,
}

pub fn resolve(cli: Option<PathBuf>, env: Option<PathBuf>, default: PathBuf) -> PoolDir {
    if let Some(path) = cli {
        return PoolDir { path, source: "--pool", overrides: true };
    }
    if let Some(path) = env {
        return PoolDir { path, source: ENV_VAR, overrides: true };
    }
    PoolDir { path: default, source: "default", overrides: false }
}

/// [`resolve`] against the real process environment, applied to gs_field.
pub fn from_environment(cli: Option<PathBuf>) -> PoolDir {
    let env = std::env::var_os(ENV_VAR).filter(|v| !v.is_empty()).map(PathBuf::from);
    let default = gs_field::data_dirs::base_dir().unwrap_or_else(|_| PathBuf::from(".vellum-fe"));
    let pool = resolve(cli, env, default);
    if pool.overrides {
        apply(&pool.path);
    }
    pool
}

/// Point gs_field at `dir` and drop everything it cached from the old one
/// (the pool scan, the art calibrations). The caller resets its art store.
pub fn apply(dir: &Path) {
    gs_field::data_dirs::set_base_dir(dir.to_path_buf());
    gs_field::pool::invalidate_cache();
    gs_field::geometry::invalidate_calibrations();
}

/// A pool dir typed into the app.
pub fn set_in_app(text: &str) -> Result<PoolDir, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("The pool directory is empty".to_string());
    }
    let path = PathBuf::from(text);
    apply(&path);
    Ok(PoolDir { path, source: "set in the app", overrides: true })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_beats_env_beats_default() {
        let r = resolve(Some("/cli".into()), Some("/env".into()), "/home/.vellum-fe".into());
        assert_eq!((r.path.as_path(), r.source, r.overrides), (Path::new("/cli"), "--pool", true));
        let r = resolve(None, Some("/env".into()), "/home/.vellum-fe".into());
        assert_eq!((r.path.as_path(), r.source, r.overrides), (Path::new("/env"), ENV_VAR, true));
        let r = resolve(None, None, "/home/.vellum-fe".into());
        assert_eq!((r.path.as_path(), r.source, r.overrides), (Path::new("/home/.vellum-fe"), "default", false));
    }

    #[test]
    fn setting_it_in_the_app_moves_gs_fields_directories() {
        let _guard = gs_field::data_dirs::VELLUM_FE_DIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        assert!(set_in_app("   ").is_err());
        let pool = set_in_app(&format!("  {}  ", dir.path().display())).unwrap();
        assert_eq!((pool.path.as_path(), pool.source), (dir.path(), "set in the app"));
        assert_eq!(gs_field::data_dirs::base_dir().unwrap(), dir.path());
        assert!(gs_field::data_dirs::global_images_dir().unwrap().starts_with(dir.path()));
        gs_field::data_dirs::clear_base_dir();
        gs_field::pool::invalidate_cache();
    }
}
