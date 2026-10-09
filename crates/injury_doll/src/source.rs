//! Where the doll's puppet cache gets its figures (docs/crate-split-plan.md,
//! S6): the installed figures alone, each its own folder in the pool,
//! `global/bodies/<form>/` -- its puppet (`puppet.toml`), its body and
//! `doll.toml` -- posed by the pose sets gs_puppet ships. The creature
//! field's set (every creature's puppet, their skins) is never loaded for
//! the doll.
//!
//! An install from before the split keeps a figure's puppet in the
//! creatures' folder, `global/puppets/<form>.puppet.toml`: read when the
//! figure's own folder has none. No egui here.

use std::any::Any;
use std::path::PathBuf;
use std::sync::Arc;

use gs_field_egui::puppet_cache::{PuppetSource, Sheets};

/// The doll's figures as a [`PuppetSource`]: what [`crate::view::DollCache`]
/// reads.
#[derive(Debug, Clone, Default)]
pub struct DollSource {
    /// The pool's `global/` folder (None: the base dir's, as it is now).
    global: Option<PathBuf>,
}

/// The one thing read off a baked puppet before it is parsed.
#[derive(serde::Deserialize)]
struct Head {
    #[serde(default)]
    body_type: String,
}

impl DollSource {
    /// The figures in the pool at `global` (a pool's `global/` folder),
    /// wherever the base dir is.
    pub fn at(global: PathBuf) -> Self {
        DollSource { global: Some(global) }
    }

    fn global(&self) -> Option<PathBuf> {
        self.global.clone().or_else(|| gs_field::data_dirs::global_dir().ok())
    }

    /// `form`'s folder of its own (`global/bodies/<form>/`).
    fn folder(&self, form: &str) -> Option<PathBuf> {
        Some(self.global()?.join("bodies").join(form.trim()))
    }

    /// Where the pre-split install kept `form`'s files:
    /// `global/puppets/<form><suffix>`.
    fn legacy(&self, form: &str, suffix: &str) -> Option<PathBuf> {
        Some(self.global()?.join("puppets").join(format!("{}{suffix}", form.trim())))
    }

    /// `form`'s baked puppet, unparsed: its own folder's, else the
    /// pre-split install's.
    pub fn baked_toml(&self, form: &str) -> Option<String> {
        [self.folder(form).map(|d| d.join("puppet.toml")), self.legacy(form, ".puppet.toml")].into_iter().flatten().find_map(|path| std::fs::read_to_string(path).ok())
    }
}

impl PuppetSource for DollSource {
    fn refresh(&mut self) -> u64 {
        crate::roster::generation()
    }

    fn generation_now(&self) -> u64 {
        crate::roster::generation()
    }

    fn baked(&self, form: &str) -> Option<(Arc<str>, String)> {
        let text = self.baked_toml(form)?;
        let body_type = toml::from_str::<Head>(&text).map(|h| h.body_type).unwrap_or_default();
        Some((Arc::from(text), body_type))
    }

    fn pose_file(&self, body_type: &str) -> Option<Arc<str>> {
        gs_puppet::PoseSet::shipped_toml(body_type).map(Arc::from)
    }

    /// A figure is drawn as its body; a pattern sheet for its ring puppet
    /// (drawn without the GPU) is where the pre-split install would have
    /// put one, `global/puppets/<form>.texture.png` and `.normal.png`.
    fn sheets(&mut self, form: &str, _creature: Option<&str>) -> Sheets {
        let read = |suffix: &str| self.legacy(form, suffix).and_then(|p| std::fs::read(p).ok()).map(Arc::<[u8]>::from);
        Sheets { texture: read(".texture.png"), normal: read(".normal.png") }
    }

    fn body_dir(&self, form: &str) -> Option<PathBuf> {
        let dir = self.folder(form)?;
        dir.join("body.obj").is_file().then_some(dir)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A figure's puppet is read from its own folder, else from where an
    /// install before the split put it; its body is its folder's; its pose
    /// set the one shipped for its body type; a form with neither is none.
    #[test]
    fn a_figure_is_read_from_its_own_folder_first() {
        let global = std::env::temp_dir().join(format!("injury_doll_source_{}", std::process::id())).join("global");
        let _ = std::fs::remove_dir_all(&global);
        std::fs::create_dir_all(global.join("bodies/half_elf_male")).unwrap();
        std::fs::create_dir_all(global.join("puppets")).unwrap();
        std::fs::write(global.join("bodies/half_elf_male/puppet.toml"), "family = \"half_elf_male\"\nbody_type = \"biped\"\n").unwrap();
        std::fs::write(global.join("bodies/half_elf_male/body.obj"), "v 0 0 0\n").unwrap();
        std::fs::write(global.join("puppets/half_elf_male.puppet.toml"), "family = \"stale\"\nbody_type = \"quadruped\"\n").unwrap();
        std::fs::write(global.join("puppets/elf_female.puppet.toml"), "family = \"elf_female\"\nbody_type = \"biped\"\n").unwrap();
        let source = DollSource::at(global.clone());
        let (text, body_type) = source.baked("half_elf_male").expect("its own folder's");
        assert!(text.contains("\"half_elf_male\"") && body_type == "biped", "its own folder first: {text}");
        assert_eq!(source.baked("elf_female").map(|(_, b)| b).as_deref(), Some("biped"), "the pre-split install's");
        assert!(source.baked("humanoid").is_none(), "a creature's form is not a figure");
        assert_eq!(source.body_dir("half_elf_male"), Some(global.join("bodies/half_elf_male")));
        assert_eq!(source.body_dir("elf_female"), None, "no body there");
        assert!(source.pose_file("biped").is_some_and(|t| gs_puppet::PoseSet::from_toml(&t).is_ok()));
        std::fs::remove_dir_all(global.parent().unwrap()).ok();
    }
}
