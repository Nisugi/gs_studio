//! Asset packs (docs/crate-split-plan.md, S8; docs/asset-repos-plan.md):
//! art and creature data come to a pool as packs, not compiled into any
//! binary. A pack is a folder -- or a zip of one -- in the pool's own
//! layout: `pack.toml` (what it is) and `global/...` (the files, where they
//! go in the pool). Installing one copies its `global/` tree into the pool
//! (the base dir: the folder holding `global/`).
//!
//! The pool keeps a ledger, `<pool>/packs/installed.toml`: each pack's
//! version and the SHA-256 of every file it installed. An update replaces
//! only files still as the pack left them, and keeps -- and says -- any the
//! user changed; a file the pool had before any pack is kept unless it is
//! the pack's own already. A file a new version drops goes, unless changed.
//!
//! No HTTP here: a host fetches a pack's zip and hands its bytes (and the
//! SHA-256 its catalogue gave) to [`install_zip`]. No egui.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// The pack format this build understands; a pack newer than it is refused.
pub const FORMAT: u32 = 1;

/// The most a pack may unpack to, and how many files it may hold: a zip
/// that claims more is refused before anything is written.
pub const MAX_UNPACKED_BYTES: u64 = 4 << 30;
pub const MAX_FILES: usize = 100_000;

/// `pack.toml`: what a pack is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Its name in a ledger and a catalogue: lower snake case.
    pub id: String,
    #[serde(default)]
    pub title: String,
    /// What it holds: `forms`, `figures`, `skins`, `area`...
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub version: String,
    /// The pack format it is written in ([`FORMAT`]).
    #[serde(default = "first_format")]
    pub format: u32,
}

fn first_format() -> u32 {
    1
}

impl Manifest {
    pub fn from_toml(text: &str) -> Result<Manifest> {
        let manifest: Manifest = toml::from_str(text).context("pack.toml")?;
        let id_ok = !manifest.id.is_empty() && manifest.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
        if !id_ok {
            bail!("pack.toml: id '{}' is not lower snake case", manifest.id);
        }
        if manifest.format > FORMAT {
            bail!("pack '{}' is format {}, newer than this build understands ({FORMAT})", manifest.id, manifest.format);
        }
        Ok(manifest)
    }
}

/// What an install, update or removal did, by pool path (`global/...`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub id: String,
    pub version: String,
    /// Written where the pool had nothing.
    pub added: Vec<String>,
    /// Replaced: the pool's was as the pack's last version left it.
    pub updated: Vec<String>,
    /// Left alone: the user changed it, or the pool had its own before.
    pub kept: Vec<String>,
    /// Gone with the pack's last version, as it left them.
    pub removed: Vec<String>,
}

impl Report {
    /// One line for a status bar or a log.
    pub fn summary(&self) -> String {
        let mut s = format!("{} {}: {} added, {} updated, {} removed", self.id, self.version, self.added.len(), self.updated.len(), self.removed.len());
        if !self.kept.is_empty() {
            s += &format!(", {} kept as the pool has them", self.kept.len());
        }
        s
    }
}

/// `<pool>/packs/installed.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ledger {
    #[serde(default)]
    pub pack: BTreeMap<String, Installed>,
}

/// One pack as the ledger has it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Installed {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub kind: String,
    /// Pool path -> the SHA-256 of the pack's file there.
    #[serde(default)]
    pub files: BTreeMap<String, String>,
}

/// Where a pool's ledger is.
pub fn ledger_path(pool: &Path) -> PathBuf {
    pool.join("packs").join("installed.toml")
}

/// The pool's ledger (empty when it has none).
pub fn ledger(pool: &Path) -> Result<Ledger> {
    match std::fs::read_to_string(ledger_path(pool)) {
        Ok(text) => toml::from_str(&text).with_context(|| ledger_path(pool).display().to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Ledger::default()),
        Err(e) => Err(e).with_context(|| ledger_path(pool).display().to_string()),
    }
}

fn write_ledger(pool: &Path, ledger: &Ledger) -> Result<()> {
    let text = toml::to_string_pretty(ledger).context("ledger")?;
    write_file(&ledger_path(pool), text.as_bytes())
}

/// `bytes`' SHA-256, lower-case hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// A pack's files, read: its manifest and `(pool path, bytes)`.
struct Unpacked {
    manifest: Manifest,
    files: Vec<(String, Vec<u8>)>,
}

/// `path` (a pack's, `/`-separated) as a pool path: under `global/`, with
/// no `..`, no root, no drive.
fn pool_path(path: &str) -> Result<String> {
    let path = path.replace('\\', "/");
    let ok = Path::new(&path).components().all(|c| matches!(c, Component::Normal(_)));
    if !ok || !path.starts_with("global/") || path.ends_with('/') {
        bail!("pack file '{path}' is not under global/");
    }
    Ok(path)
}

/// A pack folder: `pack.toml` and every file under `global/`.
fn read_dir_pack(dir: &Path) -> Result<Unpacked> {
    let manifest = Manifest::from_toml(&std::fs::read_to_string(dir.join("pack.toml")).with_context(|| dir.join("pack.toml").display().to_string())?)?;
    let mut files = Vec::new();
    let mut total = 0u64;
    let mut stack = vec![dir.join("global")];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).with_context(|| d.display().to_string())? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                bail!("{}: a pack holds no links", entry.path().display());
            }
            if kind.is_dir() {
                stack.push(entry.path());
                continue;
            }
            let rel = entry.path().strip_prefix(dir).expect("under the pack").to_string_lossy().replace('\\', "/");
            let bytes = std::fs::read(entry.path()).with_context(|| entry.path().display().to_string())?;
            total += bytes.len() as u64;
            if total > MAX_UNPACKED_BYTES || files.len() >= MAX_FILES {
                bail!("pack '{}' is larger than a pack may be", manifest.id);
            }
            files.push((pool_path(&rel)?, bytes));
        }
    }
    files.sort();
    Ok(Unpacked { manifest, files })
}

/// A pack's zip: `pack.toml` (at its root, or under one folder) and the
/// files under `global/`; checked before anything is unpacked.
fn read_zip_pack(bytes: &[u8]) -> Result<Unpacked> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).context("not a zip")?;
    if zip.len() > MAX_FILES {
        bail!("the zip holds more files than a pack may");
    }
    let mut claimed = 0u64;
    for i in 0..zip.len() {
        claimed += zip.by_index_raw(i)?.size();
    }
    if claimed > MAX_UNPACKED_BYTES {
        bail!("the zip unpacks to more than a pack may");
    }
    let names: Vec<String> = zip.file_names().map(|n| n.map(|n| n.into_owned())).collect::<Result<_, _>>().context("a name in the zip")?;
    // a zip of the pack's folder has it all under that folder
    let root = names.iter().find(|n| n.ends_with("pack.toml") && n.matches('/').count() <= 1).map(|n| n.trim_end_matches("pack.toml").to_owned()).context("the zip has no pack.toml")?;
    let mut manifest = None;
    let mut files = Vec::new();
    let mut total = 0u64;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i)?;
        if file.is_dir() {
            continue;
        }
        let name = file.name().context("a name in the zip")?.into_owned();
        if file.is_symlink() {
            bail!("{name}: a pack holds no links");
        }
        let Some(rel) = name.strip_prefix(root.as_str()).map(str::to_owned) else { bail!("{name}: outside the pack's folder") };
        let mut data = Vec::with_capacity(file.size().min(MAX_UNPACKED_BYTES) as usize);
        file.by_ref().take(MAX_UNPACKED_BYTES - total + 1).read_to_end(&mut data)?;
        total += data.len() as u64;
        if total > MAX_UNPACKED_BYTES {
            bail!("the zip unpacks to more than a pack may");
        }
        if rel == "pack.toml" {
            manifest = Some(Manifest::from_toml(std::str::from_utf8(&data).context("pack.toml is not text")?)?);
        } else {
            files.push((pool_path(&rel)?, data));
        }
    }
    files.sort();
    Ok(Unpacked { manifest: manifest.context("the zip has no pack.toml")?, files })
}

/// Install (or update) the pack folder `dir` into the pool at `pool`.
pub fn install_dir(dir: &Path, pool: &Path) -> Result<Report> {
    install(read_dir_pack(dir)?, pool)
}

/// Install (or update) the pack zipped in `bytes` into the pool at `pool`,
/// once its SHA-256 is `sha256` when one is given (a catalogue's).
pub fn install_zip(bytes: &[u8], sha256: Option<&str>, pool: &Path) -> Result<Report> {
    if let Some(want) = sha256 {
        let got = sha256_hex(bytes);
        if !got.eq_ignore_ascii_case(want.trim()) {
            bail!("the pack's SHA-256 is {got}, not {want}: not installed");
        }
    }
    install(read_zip_pack(bytes)?, pool)
}

fn install(pack: Unpacked, pool: &Path) -> Result<Report> {
    let mut ledger = ledger(pool)?;
    let before = ledger.pack.remove(&pack.manifest.id).unwrap_or_default();
    let mut report = Report { id: pack.manifest.id.clone(), version: pack.manifest.version.clone(), ..Report::default() };
    let mut now = Installed { version: pack.manifest.version.clone(), kind: pack.manifest.kind.clone(), files: BTreeMap::new() };
    for (path, bytes) in &pack.files {
        let target = pool.join(path);
        let hash = sha256_hex(bytes);
        let have = std::fs::read(&target).ok().map(|b| sha256_hex(&b));
        match have {
            None => {
                write_file(&target, bytes)?;
                report.added.push(path.clone());
            }
            Some(h) if h == hash => {}
            // as the pack's last version left it: the pack's to replace
            Some(h) if before.files.get(path) == Some(&h) => {
                write_file(&target, bytes)?;
                report.updated.push(path.clone());
            }
            // the user's, or the pool's own from before: kept
            Some(_) => report.kept.push(path.clone()),
        }
        now.files.insert(path.clone(), hash);
    }
    for (path, hash) in &before.files {
        if now.files.contains_key(path) {
            continue;
        }
        let target = pool.join(path);
        match std::fs::read(&target).ok().map(|b| sha256_hex(&b)) {
            Some(h) if &h == hash => {
                std::fs::remove_file(&target).with_context(|| target.display().to_string())?;
                report.removed.push(path.clone());
            }
            Some(_) => report.kept.push(path.clone()),
            None => {}
        }
    }
    ledger.pack.insert(pack.manifest.id, now);
    write_ledger(pool, &ledger)?;
    Ok(report)
}

/// Take pack `id` out of the pool: every file of it still as it left it.
pub fn remove(id: &str, pool: &Path) -> Result<Report> {
    let mut ledger = ledger(pool)?;
    let Some(gone) = ledger.pack.remove(id) else { bail!("no pack '{id}' is installed") };
    let mut report = Report { id: id.to_owned(), version: gone.version.clone(), ..Report::default() };
    for (path, hash) in &gone.files {
        let target = pool.join(path);
        match std::fs::read(&target).ok().map(|b| sha256_hex(&b)) {
            Some(h) if &h == hash => {
                std::fs::remove_file(&target).with_context(|| target.display().to_string())?;
                report.removed.push(path.clone());
            }
            Some(_) => report.kept.push(path.clone()),
            None => {}
        }
    }
    write_ledger(pool, &ledger)?;
    Ok(report)
}

/// Write `bytes` to `path` whole or not at all (a temporary file renamed).
fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| dir.display().to_string())?;
    }
    let tmp = path.with_extension("pack-tmp");
    std::fs::write(&tmp, bytes).with_context(|| tmp.display().to_string())?;
    std::fs::rename(&tmp, path).with_context(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gs_base_packs_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn pack(dir: &Path, version: &str, files: &[(&str, &str)]) {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("pack.toml"), format!("id = \"forms\"\nkind = \"forms\"\nversion = \"{version}\"\n")).unwrap();
        for (path, text) in files {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }

    fn read(pool: &Path, path: &str) -> Option<String> {
        std::fs::read_to_string(pool.join(path)).ok()
    }

    /// A pack goes into the pool; an update replaces what it left as it
    /// left it, keeps what the user changed, and takes away what it no
    /// longer has; a file the pool had of its own before stays.
    #[test]
    fn an_update_replaces_only_what_the_pack_left() {
        let root = temp("update");
        let (src, pool) = (root.join("forms"), root.join("pool"));
        std::fs::create_dir_all(pool.join("global/puppets")).unwrap();
        std::fs::write(pool.join("global/puppets/ape.puppet.toml"), "the pool's own").unwrap();
        pack(&src, "1", &[("global/puppets/humanoid.puppet.toml", "h1"), ("global/puppets/canine.puppet.toml", "c1"), ("global/puppets/ape.puppet.toml", "a1"), ("global/bestiary.json", "b1"), ("global/rigs/old.toml", "o1")]);
        let first = install_dir(&src, &pool).unwrap();
        assert_eq!(first.added.len(), 4);
        assert_eq!(first.kept, ["global/puppets/ape.puppet.toml"], "the pool's own file stays");
        assert_eq!(read(&pool, "global/puppets/ape.puppet.toml").as_deref(), Some("the pool's own"));
        // the user changes one; the pack moves on
        std::fs::write(pool.join("global/puppets/canine.puppet.toml"), "mine").unwrap();
        pack(&src, "2", &[("global/puppets/humanoid.puppet.toml", "h2"), ("global/puppets/canine.puppet.toml", "c2"), ("global/bestiary.json", "b1")]);
        let second = install_dir(&src, &pool).unwrap();
        assert_eq!(second.updated, ["global/puppets/humanoid.puppet.toml"]);
        // the user's change, and the pool's own file the pack no longer ships
        assert_eq!(second.kept, ["global/puppets/canine.puppet.toml", "global/puppets/ape.puppet.toml"]);
        assert_eq!(second.removed, ["global/rigs/old.toml"]);
        assert_eq!(read(&pool, "global/puppets/humanoid.puppet.toml").as_deref(), Some("h2"));
        assert_eq!(read(&pool, "global/puppets/canine.puppet.toml").as_deref(), Some("mine"));
        assert_eq!(read(&pool, "global/puppets/ape.puppet.toml").as_deref(), Some("the pool's own"), "never the pack's: not taken away");
        assert_eq!(ledger(&pool).unwrap().pack["forms"].version, "2");
        let gone = remove("forms", &pool).unwrap();
        assert_eq!(gone.removed.len(), 2, "humanoid and the bestiary: {gone:?}");
        assert_eq!(read(&pool, "global/puppets/canine.puppet.toml").as_deref(), Some("mine"), "the user's change outlives the pack");
        std::fs::remove_dir_all(&root).ok();
    }

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::Write;
        let mut out = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut out);
            for (name, bytes) in files {
                zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
                zip.write_all(bytes).unwrap();
            }
            zip.finish().unwrap();
        }
        out.into_inner()
    }

    /// A zip installs as its folder would, under one folder or none; a
    /// wrong SHA-256, a path out of `global/`, or a newer format is refused
    /// with nothing written.
    #[test]
    fn a_zip_installs_once_it_checks_out() {
        let root = temp("zip");
        let pool = root.join("pool");
        let good = zip_of(&[("forms/pack.toml", b"id = \"forms\"\nversion = \"1\"\n"), ("forms/global/bestiary.json", b"{}")]);
        assert!(install_zip(&good, Some("00"), &pool).unwrap_err().to_string().contains("SHA-256"));
        assert!(!pool.join("global").exists(), "nothing written");
        let report = install_zip(&good, Some(&sha256_hex(&good)), &pool).unwrap();
        assert_eq!(report.added, ["global/bestiary.json"]);
        let flat = zip_of(&[("pack.toml", b"id = \"flat\"\n"), ("global/rigs/a.toml", b"a")]);
        assert_eq!(install_zip(&flat, None, &pool).unwrap().added, ["global/rigs/a.toml"]);
        let escape = zip_of(&[("pack.toml", b"id = \"bad\"\n"), ("global/../../evil.txt", b"x")]);
        assert!(install_zip(&escape, None, &pool).is_err());
        let outside = zip_of(&[("pack.toml", b"id = \"bad\"\n"), ("settings.toml", b"x")]);
        assert!(install_zip(&outside, None, &pool).is_err());
        let newer = zip_of(&[("pack.toml", b"id = \"new\"\nformat = 99\n"), ("global/a", b"a")]);
        assert!(install_zip(&newer, None, &pool).unwrap_err().to_string().contains("newer"));
        assert!(!root.join("evil.txt").exists() && !pool.join("settings.toml").exists());
        std::fs::remove_dir_all(&root).ok();
    }
}
