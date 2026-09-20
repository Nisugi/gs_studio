//! Where the art pool is: the directory holding `global/images/...` and
//! `global/scenes/`. Precedence: `--pool <dir>`, then the `GS_STUDIO_POOL`
//! environment variable, then **gs-studio's own data home**
//! (`crate::data_home`).
//!
//! It is deliberately NOT gs_field's own default. `gs_field::data_dirs`
//! resolves to `VELLUM_FE_DIR`, else `~/.vellum-fe`, because VellumFE is
//! adopting that crate and depends on that behaviour; but gs-studio is a
//! separate app and must not read the game client's artwork unless the user
//! explicitly points it there. So the HOST installs its own default at
//! startup through `data_dirs::set_base_dir`, which the crate already
//! provides for exactly this. gs_field's unset behaviour is untouched.
//!
//! This is NOT the data root (`data_root.rs`): that one holds
//! `outlines.json` and `rigs/` and belongs to the rig editor.

use std::path::{Path, PathBuf};

use crate::data_home;

pub const ENV_VAR: &str = "GS_STUDIO_POOL";

/// How the pool directory was chosen. The status bar says which, so
/// "where did it find my artwork" always has a visible answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolSource {
    /// `--pool <dir>`.
    Cli,
    /// `GS_STUDIO_POOL`.
    Env,
    /// gs-studio's own data home -- nothing pointed it anywhere.
    StudioHome,
    /// Typed into the Pool box and applied with "Set".
    SetInApp,
}

impl PoolSource {
    /// The short tag in the status bar's `pool: <path> (<source>)`.
    pub fn label(self) -> &'static str {
        match self {
            PoolSource::Cli => "--pool",
            PoolSource::Env => ENV_VAR,
            PoolSource::StudioHome => "gs-studio home, the default",
            PoolSource::SetInApp => "set in the app",
        }
    }

    /// Did the user say where to look?
    pub fn is_explicit(self) -> bool {
        self != PoolSource::StudioHome
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PoolDir {
    pub path: PathBuf,
    /// Where the path came from, for the status line.
    pub source: PoolSource,
}

impl PoolDir {
    /// The status-bar text: the path and how it was chosen.
    pub fn status(&self) -> String {
        format!("pool: {} ({})", self.path.display(), self.source.label())
    }
}

/// `--pool` > `GS_STUDIO_POOL` > gs-studio's own home. `studio_home` is
/// passed in so the precedence can be tested without touching the real
/// filesystem or the user's environment.
pub fn resolve(cli: Option<PathBuf>, env: Option<PathBuf>, studio_home: PathBuf) -> PoolDir {
    if let Some(path) = cli {
        return PoolDir { path, source: PoolSource::Cli };
    }
    if let Some(path) = env {
        return PoolDir { path, source: PoolSource::Env };
    }
    PoolDir { path: studio_home, source: PoolSource::StudioHome }
}

/// [`resolve`] against the real process environment, applied to gs_field.
///
/// ALWAYS calls [`apply`]: even the default case is an override, because
/// gs_field's own unset default is `~/.vellum-fe` and this app must never
/// land there by accident. The home directory is created when it is the
/// one being used, so a first run has somewhere to put art.
pub fn from_environment(cli: Option<PathBuf>) -> PoolDir {
    let env = std::env::var_os(ENV_VAR).filter(|v| !v.is_empty()).map(PathBuf::from);
    let pool = resolve(cli, env, data_home::default_home_or_local());
    if pool.source == PoolSource::StudioHome {
        if let Err(e) = data_home::ensure(&pool.path) {
            eprintln!("gs-studio: could not create {}: {e}", pool.path.display());
        }
    }
    apply(&pool.path);
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
    Ok(PoolDir { path, source: PoolSource::SetInApp })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_beats_env_beats_the_gs_studio_home() {
        let home = PathBuf::from("/config/gs-studio");
        let r = resolve(Some("/cli".into()), Some("/env".into()), home.clone());
        assert_eq!((r.path.as_path(), r.source), (Path::new("/cli"), PoolSource::Cli));
        let r = resolve(None, Some("/env".into()), home.clone());
        assert_eq!((r.path.as_path(), r.source), (Path::new("/env"), PoolSource::Env));
        let r = resolve(None, None, home.clone());
        assert_eq!((r.path.as_path(), r.source), (home.as_path(), PoolSource::StudioHome));
        assert!(!r.source.is_explicit());
        assert!(PoolSource::Cli.is_explicit() && PoolSource::Env.is_explicit() && PoolSource::SetInApp.is_explicit());
    }

    /// The bug this module exists to prevent: with nothing set, gs-studio
    /// must NOT land in the user's VellumFE directory.
    #[test]
    fn the_default_pool_is_not_the_vellum_fe_directory() {
        let pool = resolve(None, None, data_home::default_home_or_local());
        let text = pool.path.to_string_lossy().to_ascii_lowercase();
        assert!(!text.contains(".vellum-fe"), "{:?}", pool.path);
        assert!(text.ends_with(data_home::APP_DIR), "{:?}", pool.path);
        // ... not even when VELLUM_FE_DIR is set, which only steers gs_field's
        // own (unchanged) default -- a default this app never uses.
        let _guard = gs_field::data_dirs::VELLUM_FE_DIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("VELLUM_FE_DIR", "/somebodys/.vellum-fe");
        gs_field::data_dirs::clear_base_dir();
        assert_eq!(gs_field::data_dirs::base_dir().unwrap(), Path::new("/somebodys/.vellum-fe"), "gs_field unchanged");
        let pool = resolve(None, None, data_home::default_home_or_local());
        assert_ne!(pool.path, Path::new("/somebodys/.vellum-fe"));
        std::env::remove_var("VELLUM_FE_DIR");
    }

    #[test]
    fn the_status_line_names_the_path_and_where_it_came_from() {
        let pool = PoolDir { path: "/config/gs-studio".into(), source: PoolSource::StudioHome };
        assert_eq!(pool.status(), "pool: /config/gs-studio (gs-studio home, the default)");
        let pool = PoolDir { path: "/art".into(), source: PoolSource::Cli };
        assert_eq!(pool.status(), "pool: /art (--pool)");
    }

    #[test]
    fn setting_it_in_the_app_moves_gs_fields_directories() {
        let _guard = gs_field::data_dirs::VELLUM_FE_DIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        assert!(set_in_app("   ").is_err());
        let pool = set_in_app(&format!("  {}  ", dir.path().display())).unwrap();
        assert_eq!((pool.path.as_path(), pool.source), (dir.path(), PoolSource::SetInApp));
        assert_eq!(gs_field::data_dirs::base_dir().unwrap(), dir.path());
        assert!(gs_field::data_dirs::global_images_dir().unwrap().starts_with(dir.path()));
        gs_field::data_dirs::clear_base_dir();
        gs_field::pool::invalidate_cache();
    }
}
