//! The creature field's folders under the base dir (`data_dirs`): skins,
//! scenes, the image pool's categories, icons, data, dolls -- and the atomic
//! file writer its stores use. Split out of `data_dirs` (ADDITION): the base
//! dir is what the doll and the field share; these are the field's.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::data_dirs::global_dir;

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
