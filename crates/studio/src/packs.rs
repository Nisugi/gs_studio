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
    zip_pack(&dir, data_root)
}

/// The pack folder `dir` zipped into `<data>/../packs/<id>.zip`.
fn zip_pack(dir: &Path, data_root: &Path) -> Result<Built, String> {
    let text = std::fs::read_to_string(dir.join("pack.toml")).map_err(|e| format!("{}: {e}", dir.join("pack.toml").display()))?;
    let manifest = packs::Manifest::from_toml(&text).map_err(|e| format!("{e:#}"))?;
    let bytes = zip_dir(dir)?;
    let zip = packs_dir(data_root).join(format!("{}.zip", manifest.id));
    std::fs::create_dir_all(packs_dir(data_root)).map_err(|e| e.to_string())?;
    std::fs::write(&zip, &bytes).map_err(|e| format!("{}: {e}", zip.display()))?;
    Ok(Built { zip, bytes: bytes.len() as u64, sha256: packs::sha256_hex(&bytes) })
}

/// Where gs-studio's packs are released: one GitHub release per pack
/// version, tagged `<id>-<version>`, holding `<id>.zip`; `packs.toml` at
/// the repo's top lists them (`gs_base::packs::Catalogue`).
pub const ASSET_REPO: &str = "Nisugi/studio_assets";

/// The catalogue the exports keep, what goes to the asset repo's top:
/// `<data>/../packs/packs.toml`.
pub fn catalogue_path(data_root: &Path) -> PathBuf {
    packs_dir(data_root).join("packs.toml")
}

/// What *Export pack* made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exported {
    pub id: String,
    pub version: String,
    pub built: Built,
    /// The files packed, `pack.toml` aside.
    pub files: usize,
    /// False when the zip is the one the catalogue already lists.
    pub changed: bool,
}

impl Exported {
    /// One line for the status bar and the tab.
    pub fn summary(&self) -> String {
        let what = if self.changed { format!("version {}: release it as {}-{} on {ASSET_REPO}", self.version, self.id, self.version) } else { format!("unchanged since version {}", self.version) };
        format!("{} ({} files, {} KB), {what}", self.built.zip.display(), self.files, self.built.bytes.div_ceil(1024))
    }
}

/// The pool's frame recipe `name` as a pack (the Frames tab's *Export
/// pack*, `--pack frame:<name>`): `<data>/../packs/frame_<name>/` holding
/// the recipe with the parts it names (`frame_recipe::write_pack_files`),
/// then [`publish`]ed.
pub fn export_frame(name: &str, data_root: &Path) -> Result<Exported, String> {
    use gs_calibrators::frame_recipe as fr;
    let path = fr::recipe_path(name).ok_or_else(|| "no pool".to_owned())?;
    let recipe = fr::Recipe::read(&path)?;
    let problems = fr::validate(&recipe, |kind, part| fr::load_part(kind, part));
    if !fr::exportable(&problems) {
        let errors: Vec<&str> = problems.iter().filter(|p| !p.warning).map(|p| p.text.as_str()).collect();
        return Err(format!("{name}: {}", errors.join("; ")));
    }
    let id = fr::pack_id(name);
    let dir = packs_dir(data_root).join(&id);
    let files = fr::write_pack_files(name, &recipe, &dir)?;
    publish(&dir, &id, name, "frames", &format!("The frame recipe '{name}' and the parts it names"), files.len(), data_root)
}

/// The pool's doll picture `name` (`global/images/dolls/<name>.png`) as a
/// pack (the Dolls tab's *Export pack*, `--pack doll:<name>`): id
/// `doll_<name>`, the picture with its calibration in it and its own
/// overlays (`<name>_<part>_<level>.png`), where they sit in the pool, so a
/// client installs them at `global/images/dolls/` (agreed with Hydra's side
/// 2026-10-11). Refused while the picture carries no calibration.
pub fn export_doll(name: &str, data_root: &Path) -> Result<Exported, String> {
    use gs_calibrators::doll_calibration::is_overlay;
    let name = name.trim().trim_end_matches(".png");
    let dolls = creature_field::pool_dirs::global_images_dir().map_err(|e| format!("{e:#}"))?.join("dolls");
    let picture = dolls.join(format!("{name}.png"));
    if !picture.is_file() {
        return Err(format!("no doll picture {}", picture.display()));
    }
    if gs_base::png_meta::read_embedded(&picture).is_none() {
        return Err(format!("{name} is not calibrated: place its parts in the Dolls tab and Save first"));
    }
    let slug: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c.to_ascii_lowercase() } else { '_' }).collect();
    let id = format!("doll_{slug}");
    let dir = packs_dir(data_root).join(&id);
    let global = dir.join("global");
    if global.is_dir() {
        std::fs::remove_dir_all(&global).map_err(|e| format!("{}: {e}", global.display()))?;
    }
    let into = global.join("images").join("dolls");
    std::fs::create_dir_all(&into).map_err(|e| format!("{}: {e}", into.display()))?;
    // its own overlays: `<name>_<part>_<level>`, not another picture's
    let own = |stem: &str| stem == name || (is_overlay(stem) && stem.rsplitn(3, '_').nth(2) == Some(name));
    let mut files = 0;
    for entry in std::fs::read_dir(&dolls).map_err(|e| format!("{}: {e}", dolls.display()))?.flatten() {
        let path = entry.path();
        let png = path.extension().is_some_and(|x| x.eq_ignore_ascii_case("png"));
        if png && path.file_stem().and_then(|s| s.to_str()).is_some_and(own) {
            let to = into.join(entry.file_name());
            std::fs::copy(&path, &to).map_err(|e| format!("{}: {e}", to.display()))?;
            files += 1;
        }
    }
    publish(&dir, &id, name, "dolls", &format!("The injury doll '{name}', its calibration in the picture"), files, data_root)
}

/// A pack folder whose `global/` is written: its `pack.toml`, the zip
/// beside it, and its line in `packs.toml`. The version is a whole number,
/// raised when the zip is not the one the catalogue lists.
fn publish(dir: &Path, id: &str, title: &str, kind: &str, about: &str, files: usize, data_root: &Path) -> Result<Exported, String> {
    let catalogue_file = catalogue_path(data_root);
    let mut catalogue = match std::fs::read_to_string(&catalogue_file) {
        Ok(text) => packs::Catalogue::from_toml(&text).map_err(|e| format!("{}: {e:#}", catalogue_file.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => packs::Catalogue::default(),
        Err(e) => return Err(format!("{}: {e}", catalogue_file.display())),
    };
    let listed = catalogue.get(id).cloned();
    let manifest = |version: &str| {
        let text = format!(
            "# {about}, for Hydra: gs-studio's Export pack.\n# Written afresh by every export.\nid = \"{id}\"\ntitle = \"{title}\"\nkind = \"{kind}\"\nversion = \"{version}\"\nformat = {}\n",
            packs::FORMAT
        );
        std::fs::write(dir.join("pack.toml"), text).map_err(|e| format!("{}: {e}", dir.join("pack.toml").display()))
    };
    let mut version = listed.as_ref().map_or_else(|| "1".to_owned(), |l| l.version.clone());
    manifest(&version)?;
    let mut built = zip_pack(dir, data_root)?;
    let changed = listed.as_ref().is_none_or(|l| l.sha256 != built.sha256);
    if changed && listed.is_some() {
        let n: u64 = version.trim().parse().map_err(|_| format!("{}: {id}'s version '{version}' is not a whole number", catalogue_file.display()))?;
        version = (n + 1).to_string();
        manifest(&version)?;
        built = zip_pack(dir, data_root)?;
    }
    catalogue.put(packs::Listed {
        id: id.to_owned(),
        title: title.to_owned(),
        kind: kind.to_owned(),
        version: version.clone(),
        size: built.bytes,
        sha256: built.sha256.clone(),
        url: format!("https://github.com/{ASSET_REPO}/releases/download/{id}-{version}/{id}.zip"),
    });
    let text = catalogue.to_toml().map_err(|e| format!("{e:#}"))?;
    let header = format!("# The packs on github.com/{ASSET_REPO}, read raw by Hydra from the repo's top.\n# gs-studio's Export pack keeps it (packs::publish); a pack's zip is its release's asset.\n\n");
    std::fs::write(&catalogue_file, header + &text).map_err(|e| format!("{}: {e}", catalogue_file.display()))?;
    Ok(Exported { id: id.to_owned(), version, built, files, changed })
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
