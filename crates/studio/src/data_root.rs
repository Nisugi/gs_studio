//! Where the `data/` directory is. Precedence: `--data <dir>`, then the
//! `GS_STUDIO_DATA` environment variable, then a `data/outlines.json` found
//! by walking up from the working directory, then from the executable.

use std::path::{Path, PathBuf};

pub const ENV_VAR: &str = "GS_STUDIO_DATA";

#[derive(Debug, Clone, PartialEq)]
pub struct DataRoot {
    pub path: PathBuf,
    /// Where the path came from, for the status line.
    pub source: &'static str,
}

/// `--data <dir>` or `--data=<dir>` from the command line (program name
/// already stripped). Err carries a usage message.
pub fn parse_args(args: &[String]) -> Result<Option<PathBuf>, String> {
    let mut data = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == "--data" {
            data = Some(PathBuf::from(it.next().ok_or("--data needs a directory")?));
        } else if let Some(dir) = arg.strip_prefix("--data=") {
            data = Some(PathBuf::from(dir));
        } else {
            return Err(format!("unknown argument '{arg}'\nusage: gs-studio [--data <dir>]"));
        }
    }
    Ok(data)
}

/// `is_data_dir` answers "does this directory hold outlines.json?" -- a
/// parameter so the search can be tested without a filesystem.
pub fn resolve(
    cli: Option<PathBuf>,
    env: Option<PathBuf>,
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
    let starts = [(cwd, "found from the working directory"), (exe.and_then(Path::parent), "found beside the executable")];
    for (start, source) in starts {
        let found = start.into_iter().flat_map(Path::ancestors).map(|d| d.join("data")).find(|d| is_data_dir(d));
        if let Some(path) = found {
            return DataRoot { path, source };
        }
    }
    DataRoot { path: cwd.unwrap_or(Path::new(".")).join("data"), source: "default (not found)" }
}

/// [`resolve`] against the real process environment.
pub fn from_environment(cli: Option<PathBuf>) -> DataRoot {
    let env = std::env::var_os(ENV_VAR).filter(|v| !v.is_empty()).map(PathBuf::from);
    resolve(
        cli,
        env,
        std::env::current_dir().ok().as_deref(),
        std::env::current_exe().ok().as_deref(),
        |d| d.join("outlines.json").is_file(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn parses_both_spellings_and_rejects_junk() {
        assert_eq!(parse_args(&args(&[])), Ok(None));
        assert_eq!(parse_args(&args(&["--data", "x/y"])), Ok(Some("x/y".into())));
        assert_eq!(parse_args(&args(&["--data=z"])), Ok(Some("z".into())));
        assert!(parse_args(&args(&["--data"])).is_err());
        assert!(parse_args(&args(&["--wat"])).is_err());
    }

    #[test]
    fn cli_beats_env_beats_search() {
        let repo = Path::new("/repo/data");
        let is_data = |d: &Path| d == repo;
        let cwd = Path::new("/repo/crates/studio");
        let exe = Path::new("/elsewhere/bin/gs-studio");

        let r = resolve(Some("/cli".into()), Some("/env".into()), Some(cwd), Some(exe), is_data);
        assert_eq!((r.path.as_path(), r.source), (Path::new("/cli"), "--data"));
        let r = resolve(None, Some("/env".into()), Some(cwd), Some(exe), is_data);
        assert_eq!((r.path.as_path(), r.source), (Path::new("/env"), ENV_VAR));
        let r = resolve(None, None, Some(cwd), Some(exe), is_data);
        assert_eq!(r.path, repo);
    }

    #[test]
    fn falls_back_to_the_exe_then_to_a_default() {
        let is_data = |d: &Path| d == Path::new("/repo/data");
        let exe = Path::new("/repo/target/debug/gs-studio");
        let r = resolve(None, None, Some(Path::new("/tmp")), Some(exe), is_data);
        assert_eq!((r.path.as_path(), r.source), (Path::new("/repo/data"), "found beside the executable"));
        let r = resolve(None, None, Some(Path::new("/tmp")), None, |_| false);
        assert_eq!(r.path, Path::new("/tmp/data"));
        assert_eq!(r.source, "default (not found)");
    }
}
