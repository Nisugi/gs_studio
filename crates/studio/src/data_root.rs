//! Where the `data/` directory is. Precedence: `--data <dir>`, then the
//! `GS_STUDIO_DATA` environment variable, then the folder remembered in
//! `settings.toml`, then a `data/outlines.json` found by walking up from the
//! working directory, then from the executable, and finally
//! `<gs-studio home>/data` (where an outline import will create it).
//! (`--data` itself is parsed in `cli.rs`.)

use std::path::{Path, PathBuf};

pub const ENV_VAR: &str = "GS_STUDIO_DATA";

#[derive(Debug, Clone, PartialEq)]
pub struct DataRoot {
    pub path: PathBuf,
    /// Where the path came from, for the status line.
    pub source: &'static str,
}

/// `is_data_dir` answers "does this directory hold outlines.json?" -- a
/// parameter so the search can be tested without a filesystem.
pub fn resolve(
    cli: Option<PathBuf>,
    env: Option<PathBuf>,
    saved: Option<PathBuf>,
    home: &Path,
    cwd: Option<&Path>,
    exe: Option<&Path>,
    is_data_dir: impl Fn(&Path) -> bool,
) -> DataRoot {
    if let Some(path) = cli {
        return DataRoot { path, source: "--data" };
    }
    if let Some(path) = env {
        return DataRoot { path, source: ENV_VAR };
    }
    if let Some(path) = saved {
        return DataRoot { path, source: "remembered" };
    }
    let starts = [(cwd, "found from the working directory"), (exe.and_then(Path::parent), "found beside the executable")];
    for (start, source) in starts {
        let found = start.into_iter().flat_map(Path::ancestors).map(|d| d.join("data")).find(|d| is_data_dir(d));
        if let Some(path) = found {
            return DataRoot { path, source };
        }
    }
    DataRoot { path: home.join("data"), source: "gs-studio home (no outlines yet)" }
}

/// [`resolve`] against the real process environment.
pub fn from_environment(cli: Option<PathBuf>, saved: Option<PathBuf>) -> DataRoot {
    let env = std::env::var_os(ENV_VAR).filter(|v| !v.is_empty()).map(PathBuf::from);
    resolve(
        cli,
        env,
        saved,
        &crate::data_home::default_home_or_local(),
        std::env::current_dir().ok().as_deref(),
        std::env::current_exe().ok().as_deref(),
        |d| d.join("outlines.json").is_file(),
    )
}

/// Read the silhouette chart (`scale-chart.html`) and write
/// `<data>/outlines.json` from it, creating the data directory if needed.
/// The old file is replaced only once the new one is fully written. Ok is
/// the summary for the status line.
pub fn import_outlines(chart: &Path, data: &Path) -> Result<String, String> {
    let html = std::fs::read_to_string(chart).map_err(|e| format!("{}: {e}", chart.display()))?;
    let import = rig::chart::outlines_from_chart(&html).map_err(|e| format!("{}: {e}", chart.display()))?;
    std::fs::create_dir_all(data).map_err(|e| format!("{}: {e}", data.display()))?;
    let (tmp, out) = (data.join("outlines.json.tmp"), data.join("outlines.json"));
    std::fs::write(&tmp, &import.json).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &out).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(format!("Imported {} from {} into {}", import.summary(), chart.display(), out.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn importing_a_chart_writes_outlines_into_a_new_data_dir() {
        let dir = tempfile::tempdir().unwrap();
        let chart = dir.path().join("scale-chart.html");
        std::fs::write(
            &chart,
            "<script>\nconst SIL = {\"blob\": {\"d\": [\"M0,64 L64,64 L64,0 L0,0 Z\"], \"baseY\": 64, \"ref\": 2}};\n</script>",
        )
        .unwrap();
        let data = dir.path().join("new/data");
        let summary = import_outlines(&chart, &data).unwrap();
        assert!(summary.starts_with("Imported 1 outlines"), "{summary}");
        let outlines = rig::Outlines::load(&data.join("outlines.json")).unwrap();
        assert_eq!(outlines.families(), ["blob"]);
        assert!(!data.join("outlines.json.tmp").exists());
        // A bad chart leaves the good outlines alone.
        std::fs::write(&chart, "<html></html>").unwrap();
        assert!(import_outlines(&chart, &data).is_err());
        assert!(rig::Outlines::load(&data.join("outlines.json")).is_ok());
        assert!(import_outlines(&dir.path().join("missing.html"), &data).is_err());
    }

    #[test]
    fn cli_beats_env_beats_remembered_beats_search() {
        let repo = Path::new("/repo/data");
        let is_data = |d: &Path| d == repo;
        let cwd = Path::new("/repo/crates/studio");
        let exe = Path::new("/elsewhere/bin/gs-studio");

        let home = Path::new("/home");
        let saved = || Some(PathBuf::from("/saved"));
        let r = resolve(Some("/cli".into()), Some("/env".into()), saved(), home, Some(cwd), Some(exe), is_data);
        assert_eq!((r.path.as_path(), r.source), (Path::new("/cli"), "--data"));
        let r = resolve(None, Some("/env".into()), saved(), home, Some(cwd), Some(exe), is_data);
        assert_eq!((r.path.as_path(), r.source), (Path::new("/env"), ENV_VAR));
        let r = resolve(None, None, saved(), home, Some(cwd), Some(exe), is_data);
        assert_eq!((r.path.as_path(), r.source), (Path::new("/saved"), "remembered"));
        let r = resolve(None, None, None, home, Some(cwd), Some(exe), is_data);
        assert_eq!(r.path, repo);
    }

    #[test]
    fn falls_back_to_the_exe_then_to_the_gs_studio_home() {
        let is_data = |d: &Path| d == Path::new("/repo/data");
        let exe = Path::new("/repo/target/debug/gs-studio");
        let home = Path::new("/home/gs-studio");
        let r = resolve(None, None, None, home, Some(Path::new("/tmp")), Some(exe), is_data);
        assert_eq!((r.path.as_path(), r.source), (Path::new("/repo/data"), "found beside the executable"));
        let r = resolve(None, None, None, home, Some(Path::new("/tmp")), None, |_| false);
        assert_eq!(r.path, Path::new("/home/gs-studio/data"));
        assert!(r.source.starts_with("gs-studio home"));
    }
}
