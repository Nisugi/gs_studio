//! Asset packs, gs-studio's side (docs/crate-split-plan.md, S8): the forms
//! pack its Publish writes (`packs/forms/`, committed), the figures pack
//! built from the data root's modelled bodies, a pack zipped for release
//! (`--pack`), and a pack installed into a pool (`--install-pack`, and the
//! forms pack into gs-studio's own pool as it starts). The installing is
//! `gs_base::packs`'; no egui here.

use std::path::{Path, PathBuf};

use gs_base::packs::{self, Report};

/// The packs folder beside the data root: `<data>/../packs`.
pub fn packs_dir(data_root: &Path) -> PathBuf {
    data_root.join("..").join("packs")
}

/// The forms pack Publish writes (`<data>/../packs/forms`).
pub fn forms_dir(data_root: &Path) -> PathBuf {
    packs_dir(data_root).join("forms")
}

/// The forms pack into `pool` as gs-studio starts: what the field draws
/// from now that nothing is compiled in. The pool keeps any file it had of
/// its own, and one the user changed (`gs_base::packs`). None when there is
/// no forms pack beside the data root (an installed gs-studio: its pool's
/// packs come from a catalogue).
pub fn seed_pool(data_root: &Path, pool: &Path) -> Option<Result<Report, String>> {
    let forms = forms_dir(data_root);
    forms.join("pack.toml").is_file().then(|| packs::install_dir(&forms, pool).map_err(|e| format!("{e:#}")))
}

/// A pack -- a folder, or a zip of one -- into the pool at `pool`.
pub fn install(pack: &Path, pool: &Path) -> Result<Report, String> {
    let result = if pack.is_dir() {
        packs::install_dir(pack, pool)
    } else {
        let bytes = std::fs::read(pack).map_err(|e| format!("{}: {e}", pack.display()))?;
        packs::install_zip(&bytes, None, pool)
    };
    result.map_err(|e| format!("{}: {e:#}", pack.display()))
}

/// What `--pack` made: the zip, its size and its SHA-256 (what a catalogue
/// lists).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Built {
    pub zip: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

/// `--pack <what>`: `forms` (the committed forms pack), `figures` (the
/// playable races' figures, built from the data root first:
/// [`crate::bodies::figures_pack`]), or a pack folder. Zipped into
/// `<data>/../packs/<id>.zip`.
pub fn build(what: &str, data_root: &Path) -> Result<Built, String> {
    let dir = match what {
        "forms" => forms_dir(data_root),
        "figures" => {
            let dir = packs_dir(data_root).join("figures");
            crate::bodies::figures_pack(data_root, &dir).map_err(|e| format!("figures: {e}"))?;
            dir
        }
        folder => PathBuf::from(folder),
    };
    let text = std::fs::read_to_string(dir.join("pack.toml")).map_err(|e| format!("{}: {e}", dir.join("pack.toml").display()))?;
    let manifest = packs::Manifest::from_toml(&text).map_err(|e| format!("{e:#}"))?;
    let bytes = zip_dir(&dir)?;
    let zip = packs_dir(data_root).join(format!("{}.zip", manifest.id));
    std::fs::create_dir_all(packs_dir(data_root)).map_err(|e| e.to_string())?;
    std::fs::write(&zip, &bytes).map_err(|e| format!("{}: {e}", zip.display()))?;
    Ok(Built { zip, bytes: bytes.len() as u64, sha256: packs::sha256_hex(&bytes) })
}

/// A pack folder zipped the same way every time: `pack.toml` and every file
/// under `global/`, in path order, each stamped 1980-01-01, so the same
/// pack makes the same bytes (and SHA-256).
pub fn zip_dir(dir: &Path) -> Result<Vec<u8>, String> {
    use std::io::Write;
    let mut files = vec![PathBuf::from("pack.toml")];
    let mut stack = vec![dir.join("global")];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).map_err(|e| format!("{}: {e}", d.display()))?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push(path.strip_prefix(dir).expect("under the pack").to_path_buf());
            }
        }
    }
    let name = |p: &Path| p.to_string_lossy().replace('\\', "/");
    files.sort_by_key(|p| name(p));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut out);
        for rel in &files {
            let bytes = std::fs::read(dir.join(rel)).map_err(|e| format!("{}: {e}", dir.join(rel).display()))?;
            zip.start_file(name(rel), options).map_err(|e| e.to_string())?;
            zip.write_all(&bytes).map_err(|e| e.to_string())?;
        }
        zip.finish().map_err(|e| e.to_string())?;
    }
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pack zips to the same bytes every time, and the zip installs what
    /// the folder holds.
    #[test]
    fn a_pack_zips_the_same_way_every_time_and_installs() {
        let root = std::env::temp_dir().join(format!("gs_studio_packs_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (data, pack) = (root.join("data"), root.join("packs/forms"));
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(pack.join("global/puppets/skins/humanoid")).unwrap();
        std::fs::write(pack.join("pack.toml"), "id = \"forms\"\nversion = \"1\"\n").unwrap();
        std::fs::write(pack.join("global/bestiary.json"), "{}").unwrap();
        std::fs::write(pack.join("global/puppets/skins/humanoid/lay_figure.png"), "png").unwrap();
        let first = build("forms", &data).unwrap();
        let again = build("forms", &data).unwrap();
        assert_eq!(first.sha256, again.sha256, "reproducible");
        assert_eq!(first.zip, root.join("data/../packs/forms.zip"));
        let pool = root.join("pool");
        let report = install(&first.zip, &pool).unwrap();
        assert_eq!(report.added, ["global/bestiary.json", "global/puppets/skins/humanoid/lay_figure.png"]);
        // and gs-studio's own pool gets the forms pack as it starts
        let mine = root.join("mine");
        assert_eq!(seed_pool(&data, &mine).unwrap().unwrap().added.len(), 2);
        assert!(seed_pool(&root.join("elsewhere/data"), &mine).is_none(), "no pack beside that data root");
        std::fs::remove_dir_all(&root).ok();
    }
}
