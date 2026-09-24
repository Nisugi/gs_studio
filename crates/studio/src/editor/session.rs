//! The open document: which family, its silhouette, the editor, the cached
//! bake, and the save / open / discard rules.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use rig::{
    bake_mask, family_default, template, templates, write_baked, Baked, OutlineEntry, Outlines,
    Rig, Template, View,
};
use gs_puppet::Puppet;
use rig_bake::{rasterize, Mask};

use super::depth::DepthModel;
use super::state::Editor;
use super::warnings::warnings;

/// Map long side while something is being dragged / once it is released.
pub const DRAG_PX: usize = 256;
pub const FULL_PX: usize = 512;

/// Environment override for where baked artefacts are published.
pub const PUBLISH_ENV_VAR: &str = "GS_STUDIO_PUBLISH";

/// Where Publish ships to: a directory holding `rigs/`, `puppets/` and
/// `poses/`, the layout of `crates/gs_field/data` (whose contents game
/// clients compile in). `override_dir` if given, else that directory found
/// relative to the data root (`<data>/../crates/gs_field/data`).
///
/// An override that names a `rigs` directory (the old meaning of
/// `GS_STUDIO_PUBLISH`) means its parent.
pub fn resolve_publish_dir(data_root: &Path, override_dir: Option<&Path>) -> PathBuf {
    match override_dir.filter(|d| !d.as_os_str().is_empty()) {
        Some(dir) if dir.file_name().is_some_and(|n| n == "rigs") => dir.parent().unwrap_or(dir).to_path_buf(),
        Some(dir) => dir.to_path_buf(),
        None => data_root.join("..").join("crates").join("gs_field").join("data"),
    }
}

/// Something that would throw away the current rig.
#[derive(Debug, Clone, PartialEq)]
pub enum DocAction {
    OpenFamily(String),
    NewFromTemplate { template: String, view: View },
    Quit,
}

impl DocAction {
    pub fn describe(&self) -> String {
        match self {
            DocAction::OpenFamily(f) => format!("open '{f}'"),
            DocAction::NewFromTemplate { template, view } => {
                format!("start a new {template} rig ({} view)", view.as_str())
            }
            DocAction::Quit => "quit".to_owned(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct BakeResult {
    pub baked: Baked,
    pub long_side_px: usize,
    pub revision: u64,
    /// Wall time of label + metrics (the mask is cached), milliseconds.
    pub ms: f32,
    pub warnings: Vec<String>,
    /// Changes whenever a new bake replaces the old one (texture upload key).
    pub serial: u64,
}

pub struct Session {
    pub data_root: PathBuf,
    pub outlines: Outlines,
    pub editor: Editor,
    /// Waiting on "discard unsaved changes?".
    pub pending: Option<DocAction>,
    /// Set once a Quit has been confirmed (or was never blocked).
    pub quit_approved: bool,
    /// The canvas should re-fit (a new silhouette was opened).
    pub fit_requested: bool,
    pub status: String,
    /// Where Publish ships (`rigs/`, `puppets/`, `poses/` under it).
    pub publish_dir: PathBuf,
    /// The art pool. Save installs the bake and the puppet into its
    /// `global/rigs/` and `global/puppets/`, so the stage draws them at
    /// once. None = do not install (tests, or no pool).
    pub pool: Option<PathBuf>,
    /// Save also publishes (when the publish directory is usable).
    pub publish_on_save: bool,
    masks: HashMap<usize, Mask>,
    bake: Option<BakeResult>,
    bake_serial: u64,
    /// The OTHER view's silhouette, rasterized once per open document.
    /// `Some(None)` = looked, and this family has no second view.
    other_mask: Option<Option<Mask>>,
    /// The depth guesses, re-read whenever the rig changes.
    depth: Option<(u64, DepthModel)>,
    /// The 3D puppet, as of an editor revision (Err = why none builds yet).
    puppet: Option<(u64, Result<Puppet, String>)>,
}

impl Session {
    /// Load `<data_root>/outlines.json` and open a first family.
    pub fn open(data_root: &Path, publish_dir: PathBuf, pool: Option<PathBuf>) -> Result<Session, String> {
        let outlines = Outlines::load(&data_root.join("outlines.json")).map_err(|e| e.to_string())?;
        let families = outlines.families();
        let first = families
            .iter()
            .find(|f| *f == "humanoid")
            .or(families.first())
            .ok_or("outlines.json has no silhouettes")?
            .clone();
        let placeholder = templates().remove(0);
        let mut session = Session {
            data_root: data_root.to_owned(),
            outlines,
            editor: Editor::new(placeholder.new_rig(&first, placeholder.view, 1.0, "crown"), placeholder),
            pending: None,
            quit_approved: false,
            fit_requested: true,
            status: String::new(),
            publish_dir,
            pool,
            publish_on_save: true,
            masks: HashMap::new(),
            bake: None,
            bake_serial: 0,
            other_mask: None,
            depth: None,
            puppet: None,
        };
        session.open_family(&first);
        Ok(session)
    }

    pub fn rigs_dir(&self) -> PathBuf {
        self.data_root.join("rigs")
    }

    pub fn rig_path(&self, family: &str) -> PathBuf {
        self.rigs_dir().join(Rig::file_name(family))
    }

    pub fn has_saved_rig(&self, family: &str) -> bool {
        self.rig_path(family).is_file()
    }

    /// The silhouette the current rig is drawn on.
    pub fn outline(&self) -> Option<&OutlineEntry> {
        let rig = self.editor.rig();
        self.outlines.get(&rig.family, rig.view)
    }

    /// Do `action` now, or park it behind the discard prompt if there are
    /// unsaved changes.
    pub fn request(&mut self, action: DocAction) {
        if self.editor.is_dirty() {
            self.pending = Some(action);
        } else {
            self.apply(action);
        }
    }

    pub fn confirm_pending(&mut self) {
        if let Some(action) = self.pending.take() {
            self.apply(action);
        }
    }

    pub fn cancel_pending(&mut self) {
        self.pending = None;
    }

    fn apply(&mut self, action: DocAction) {
        match action {
            DocAction::OpenFamily(family) => self.open_family(&family),
            DocAction::NewFromTemplate { template, view } => {
                let family = self.editor.rig().family.clone();
                self.new_from_template(&family, &template, view);
            }
            DocAction::Quit => self.quit_approved = true,
        }
    }

    fn set_editor(&mut self, rig: Rig, template: Template) {
        let mirror = self.editor.mirror;
        self.editor = Editor::new(rig, template);
        self.editor.mirror = mirror;
        self.masks.clear();
        self.bake = None;
        self.other_mask = None;
        self.depth = None;
        self.puppet = None;
        self.fit_requested = true;
    }

    /// Load the family's saved rig if there is one, else start from its
    /// default template.
    fn open_family(&mut self, family: &str) {
        let path = self.rig_path(family);
        if path.is_file() {
            let loaded = std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|text| Rig::from_toml(&text).map_err(|e| e.to_string()));
            match loaded {
                Ok(mut rig) => {
                    let tpl = template(&rig.body_type)
                        .or_else(|| family_default(family).and_then(|d| template(d.template)))
                        .unwrap_or_else(|| templates().remove(0));
                    // An older file gains its hierarchy here; say so. It is
                    // not "unsaved": the file on disk still says the same rig.
                    let notes = rig.migrate(Some(&tpl));
                    self.status = format!("Loaded {}", path.display());
                    if !notes.is_empty() {
                        self.status += &format!(" ({})", notes.join("; "));
                    }
                    self.set_editor(rig, tpl);
                    return;
                }
                Err(e) => {
                    self.new_default(family);
                    self.status = format!("Could not load saved rig ({e}); started from the template");
                    return;
                }
            }
        }
        self.new_default(family);
    }

    fn new_default(&mut self, family: &str) {
        let (tpl, view) = family_default(family).map_or(("biped", View::Front), |d| (d.template, d.view));
        self.new_from_template(family, tpl, view);
    }

    fn new_from_template(&mut self, family: &str, template_name: &str, view: View) {
        let tpl = template(template_name).unwrap_or_else(|| templates().remove(0));
        // Fall back to the other view if this family lacks the asked-for one.
        let view = [view, other_view(view)]
            .into_iter()
            .find(|v| self.outlines.get(family, *v).is_some())
            .unwrap_or(view);
        let ref_ft = self.outlines.get(family, view).map_or(1.0, |o| o.ref_ft);
        let axis = family_default(family).map_or("crown", |d| d.axis);
        self.status = format!("New {} rig for {family} ({} view)", tpl.name, view.as_str());
        self.set_editor(tpl.new_rig(family, view, ref_ft, axis), tpl);
    }

    /// The bake for the editor's current revision, re-running it if stale.
    /// `interactive` = a drag is in progress: bake small and fast.
    pub fn baked(&mut self, interactive: bool) -> Option<&BakeResult> {
        let px = if interactive { DRAG_PX } else { FULL_PX };
        let revision = self.editor.revision();
        let fresh = self.bake.as_ref().is_some_and(|b| b.revision == revision && b.long_side_px == px);
        if !fresh {
            let outline = self.outline()?.outline();
            let mask = self.masks.entry(px).or_insert_with(|| rasterize(&outline, px));
            let start = Instant::now();
            let baked = bake_mask(self.editor.rig(), mask);
            let ms = start.elapsed().as_secs_f32() * 1e3;
            let warnings = warnings(self.editor.rig(), &baked, mask);
            self.bake_serial += 1;
            self.bake =
                Some(BakeResult { baked, long_side_px: px, revision, ms, warnings, serial: self.bake_serial });
        }
        self.bake.as_ref()
    }

    /// The most recent bake, without re-running it.
    pub fn last_bake(&self) -> Option<&BakeResult> {
        self.bake.as_ref()
    }

    /// The OTHER view's silhouette, for DEPTH mode to draw joints against.
    /// None = this family has only the one the rig is clicked onto.
    pub fn other_outline(&self) -> Option<&OutlineEntry> {
        let rig = self.editor.rig();
        self.outlines.get(&rig.family, other_view(rig.view))
    }

    /// Re-read the depth guesses if the rig has changed since they were
    /// taken. The other view's mask behind them is rasterized once per open
    /// document. Call this before drawing; then read [`Session::depth`],
    /// which borrows immutably and so can be used beside `self.editor`.
    pub fn refresh_depth(&mut self) {
        let revision = self.editor.revision();
        if self.depth.as_ref().is_some_and(|(at, _)| *at == revision) {
            return;
        }
        self.ensure_other_mask();
        let mask = self.other_mask.as_ref().and_then(Clone::clone);
        self.depth = Some((revision, DepthModel::new(self.editor.rig(), mask)));
    }

    fn ensure_other_mask(&mut self) {
        if self.other_mask.is_none() {
            let outline = self.other_outline().map(|o| o.outline());
            self.other_mask = Some(outline.map(|o| rasterize(&o, FULL_PX)));
        }
    }

    /// The rig's 3D puppet, rebuilt only when the rig has changed since the
    /// last call. Built from the same full-size masks the bake and the depth
    /// guesses use (`FULL_PX` = `gs_puppet::MASK_PX`), so it is exactly what
    /// Save would write. Err says why there is none yet.
    pub fn puppet(&mut self) -> &Result<Puppet, String> {
        let revision = self.editor.revision();
        if self.puppet.as_ref().is_none_or(|(at, _)| *at != revision) {
            self.ensure_other_mask();
            let own = self.outline().map(|o| o.outline());
            if let Some(outline) = own {
                self.masks.entry(FULL_PX).or_insert_with(|| rasterize(&outline, FULL_PX));
            }
            let own = self.masks.get(&FULL_PX);
            let other = self.other_mask.as_ref().and_then(Option::as_ref);
            let (front, side) = match self.editor.rig().view {
                View::Front => (own, other),
                View::Side => (other, own),
            };
            let built = Puppet::build_masks(self.editor.rig(), front, side).map_err(|e| e.to_string());
            self.puppet = Some((revision, built));
        }
        &self.puppet.as_ref().expect("just built").1
    }

    /// The depth guesses as [`Session::refresh_depth`] last took them. An
    /// empty model (no guesses, no mask) until the first refresh.
    pub fn depth(&self) -> &DepthModel {
        static EMPTY: std::sync::OnceLock<DepthModel> = std::sync::OnceLock::new();
        match &self.depth {
            Some((_, model)) => model,
            None => EMPTY.get_or_init(DepthModel::default),
        }
    }


    /// True when this family has a second silhouette, so depths can be edited.
    pub fn depth_available(&self) -> bool {
        self.other_outline().is_some()
    }

    /// Ok(the directory) when publishing can work: the directory exists, or
    /// its parent does (it is then created on first publish). Err says why
    /// not, for the disabled button's hover text.
    pub fn publish_target(&self) -> Result<PathBuf, String> {
        let dir = &self.publish_dir;
        let parent_ok = dir.parent().is_some_and(|p| p.as_os_str().is_empty() || p.is_dir());
        if dir.is_dir() || parent_ok {
            Ok(dir.clone())
        } else {
            Err(format!(
                "Nowhere to publish: {} does not exist (and neither does its parent). Set {PUBLISH_ENV_VAR} to a \
                 directory, or run from the gs_studio workspace.",
                dir.display()
            ))
        }
    }

    /// True when `<data>/rigs/` holds a saved bake of this family to publish.
    pub fn has_saved_bake(&self, family: &str) -> bool {
        let dir = self.rigs_dir();
        dir.join(format!("{family}.png")).is_file() && dir.join(format!("{family}.toml")).is_file()
    }

    /// The SAVED files of a family that ship: `(file name, subdirectory)`.
    /// The puppet is there only when the last save could build one.
    fn shipped_files(&self, family: &str) -> Vec<(String, &'static str)> {
        let mut files = vec![(format!("{family}.png"), "rigs"), (format!("{family}.toml"), "rigs")];
        let puppet = Puppet::baked_file_name(family);
        if self.rigs_dir().join(&puppet).is_file() {
            files.push((puppet, "puppets"));
        }
        files
    }

    /// Ship the SAVED bake (`rigs/<family>.png` + `.toml`), the saved puppet
    /// (`puppets/<family>.puppet.toml`) and every pose set in
    /// `<data>/poses/` (`poses/`) into the publish directory. Never the
    /// `.rig.toml` source. Publishes what is on disk: unsaved edits are not
    /// in it.
    pub fn publish(&mut self) -> Result<PathBuf, String> {
        let dir = self.publish_target()?;
        let family = self.editor.rig().family.clone();
        if !self.has_saved_bake(&family) {
            return Err(format!("{family} has no saved bake yet: save first"));
        }
        let mut shipped = Vec::new();
        for (name, sub) in self.shipped_files(&family) {
            copy_into(&self.rigs_dir().join(&name), &dir.join(sub))?;
            shipped.push(format!("{sub}/{name}"));
        }
        let mut pose_count = 0;
        for entry in std::fs::read_dir(self.data_root.join("poses")).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "toml") {
                copy_into(&path, &dir.join("poses"))?;
                pose_count += 1;
            }
        }
        if pose_count > 0 {
            shipped.push(format!("{pose_count} pose sets"));
        }
        self.status = format!("Published {} to {}", shipped.join(", "), dir.display());
        Ok(dir)
    }

    /// Write `<family>.rig.toml`, `<family>.png`, `<family>.toml` and (when
    /// the rig makes one) `<family>.puppet.toml` into `<data>/rigs/`; install
    /// the bake and the puppet into the pool so the stage shows them; then,
    /// with "publish on save" on and somewhere to publish to, publish. Only
    /// the rig and its bake can fail the save: a puppet that cannot be built
    /// yet, or a pool or publish that cannot be written, is said on the
    /// status line.
    pub fn save(&mut self) -> Result<(), String> {
        let family = self.editor.rig().family.clone();
        let path = self.rig_path(&family);
        let dir = self.rigs_dir();
        let baked = self.baked(false).ok_or("no silhouette for this family/view")?.baked.clone();
        let rig = self.editor.rig();
        rig.save(&path).map_err(|e| e.to_string())?;
        write_baked(rig, &baked, &dir).map_err(|e| e.to_string())?;
        self.editor.mark_saved();
        let mut notes = vec![format!("Saved {}", path.display())];

        let puppet_path = dir.join(Puppet::baked_file_name(&family));
        match Puppet::build(self.editor.rig(), &self.outlines).and_then(|p| p.to_baked_toml()) {
            Ok(text) => match std::fs::write(&puppet_path, text) {
                Ok(()) => notes.push("puppet built".into()),
                Err(e) => notes.push(format!("PUPPET NOT WRITTEN: {}: {e}", puppet_path.display())),
            },
            Err(e) => {
                // A stale puppet must not outlive the rig it came from.
                let _ = std::fs::remove_file(&puppet_path);
                notes.push(format!("no puppet yet ({e})"));
            }
        }

        if let Some(pool) = self.pool.clone() {
            match self.install_in_pool(&family, &pool) {
                Ok(()) => notes.push("installed in the pool".into()),
                Err(e) => notes.push(format!("NOT INSTALLED IN THE POOL: {e}")),
            }
            if let Err(e) = self.install_poses() {
                notes.push(e);
            }
        }

        match self.publish_on_save.then(|| self.publish_target()) {
            Some(Ok(_)) => match self.publish() {
                Ok(dir) => notes.push(format!("published to {}", dir.display())),
                Err(e) => notes.push(format!("PUBLISH FAILED: {e}")),
            },
            Some(Err(_)) => notes.push("not published (no publish directory)".into()),
            None => {}
        }
        self.status = notes.join("; ");
        Ok(())
    }

    /// Put every pose set in `<data>/poses/` into `<pool>/global/puppets/`,
    /// where it replaces the built-in set of that body type on the stage, and
    /// drop gs_field's caches so the stage re-reads them. A file that does
    /// not parse is left out (the stage keeps what it had) and named in the
    /// error. Returns how many were installed.
    pub fn install_poses(&mut self) -> Result<usize, String> {
        let pool = self.pool.clone().ok_or("no pool to install poses into")?;
        let into = pool.join("global").join("puppets");
        let mut installed = 0;
        let mut broken = Vec::new();
        for entry in std::fs::read_dir(self.data_root.join("poses")).into_iter().flatten().flatten() {
            let path = entry.path();
            if !path.extension().is_some_and(|e| e == "toml") {
                continue;
            }
            let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| gs_puppet::PoseSet::from_toml(&t)) {
                Ok(_) => {
                    copy_into(&path, &into)?;
                    installed += 1;
                }
                Err(e) => broken.push(format!("{name}: {e}")),
            }
        }
        crate::pool_dir::invalidate_caches();
        if broken.is_empty() {
            Ok(installed)
        } else {
            Err(format!("{installed} pose sets installed; NOT installed (fix and reload): {}", broken.join("; ")))
        }
    }

    /// Copy the saved bake into `<pool>/global/rigs/` and the saved puppet
    /// into `<pool>/global/puppets/` (removing a pool puppet this family no
    /// longer has), then drop gs_field's caches so they are read again.
    fn install_in_pool(&self, family: &str, pool: &Path) -> Result<(), String> {
        let global = pool.join("global");
        let files = self.shipped_files(family);
        if !files.iter().any(|(_, sub)| *sub == "puppets") {
            let _ = std::fs::remove_file(global.join("puppets").join(Puppet::baked_file_name(family)));
        }
        for (name, sub) in files {
            copy_into(&self.rigs_dir().join(&name), &global.join(sub))?;
        }
        crate::pool_dir::invalidate_caches();
        Ok(())
    }
}

/// Copy `file` into directory `dir` (created if missing), keeping its name.
fn copy_into(file: &Path, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let to = dir.join(file.file_name().unwrap_or_default());
    std::fs::copy(file, &to).map(|_| ()).map_err(|e| format!("{} -> {}: {e}", file.display(), to.display()))
}

pub fn other_view(view: View) -> View {
    match view {
        View::Front => View::Side,
        View::Side => View::Front,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTLINES: &str = r#"{
        "humanoid-front": {"ref_ft": 6, "subpaths": [[[-1,0],[1,0],[1,6],[-1,6]]]},
        "humanoid":       {"ref_ft": 6, "subpaths": [[[-0.5,0],[0.5,0],[0.5,6],[-0.5,6]]]},
        "worm":           {"ref_ft": 2, "subpaths": [[[-4,0],[4,0],[4,2],[-4,2]]]}
    }"#;

    struct TempData(PathBuf);

    /// A session with nowhere to publish and no pool to install into.
    fn open(data: &Path) -> Result<Session, String> {
        Session::open(data, data.join("no/such/publish/dir"), None)
    }

    impl TempData {
        fn new(tag: &str) -> TempData {
            let dir = std::env::temp_dir().join(format!("gs_studio_{tag}_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("outlines.json"), OUTLINES).unwrap();
            TempData(dir)
        }
    }

    impl Drop for TempData {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn opens_humanoid_from_its_default_template() {
        let data = TempData::new("open");
        let s = open(&data.0).unwrap();
        let rig = s.editor.rig();
        assert_eq!((rig.family.as_str(), rig.body_type.as_str(), rig.view), ("humanoid", "biped", View::Front));
        assert_eq!((rig.ref_ft, rig.axis.as_str()), (6.0, "crown"));
        assert!(!s.editor.is_dirty() && s.fit_requested);
    }

    #[test]
    fn a_missing_data_dir_is_an_error_not_a_panic() {
        assert!(open(Path::new("definitely/not/here")).is_err());
    }

    #[test]
    fn save_writes_three_files_and_reopening_loads_them() {
        let data = TempData::new("save");
        let mut s = open(&data.0).unwrap();
        s.editor.place_current([0.0, 5.8]);
        s.editor.place_current([0.0, 5.0]);
        assert!(s.editor.is_dirty());
        s.save().unwrap();
        assert!(!s.editor.is_dirty());
        for f in ["humanoid.rig.toml", "humanoid.png", "humanoid.toml"] {
            assert!(data.0.join("rigs").join(f).is_file(), "{f}");
        }

        let mut again = open(&data.0).unwrap();
        assert_eq!(again.editor.rig(), s.editor.rig());
        assert_eq!(again.editor.current(), Some("neck_base"));
        assert!(again.status.starts_with("Loaded"));
        // "New from template" resets, without touching the file.
        again.request(DocAction::NewFromTemplate { template: "biped".into(), view: View::Front });
        assert!(again.editor.rig().joints.is_empty());
        assert!(again.has_saved_rig("humanoid"));
    }

    #[test]
    fn the_publish_dir_is_the_workspace_crate_unless_overridden() {
        let default = resolve_publish_dir(Path::new("ws/data"), None);
        assert!(default.ends_with("crates/gs_field/data") && default.starts_with("ws/data/.."), "{default:?}");
        assert_eq!(resolve_publish_dir(Path::new("ws/data"), Some(Path::new(""))), default);
        assert_eq!(resolve_publish_dir(Path::new("ws/data"), Some(Path::new("out/ship"))), PathBuf::from("out/ship"));
        // The old meaning (the rigs directory itself) still lands right.
        assert_eq!(resolve_publish_dir(Path::new("ws/data"), Some(Path::new("out/data/rigs"))), PathBuf::from("out/data"));
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> =
            std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn save_publishes_the_baked_pair_and_the_poses_but_never_the_source() {
        let data = TempData::new("pub");
        std::fs::create_dir_all(data.0.join("poses")).unwrap();
        std::fs::write(data.0.join("poses/biped.toml"), "body_type = \"biped\"\n").unwrap();
        let mut s = open(&data.0).unwrap();
        // Nowhere to publish: unavailable, and saving still works.
        assert!(s.publish_target().unwrap_err().contains("GS_STUDIO_PUBLISH"));
        assert!(s.publish().is_err());
        s.editor.place_current([0.0, 5.8]);
        s.save().unwrap();
        assert!(s.status.contains("not published"), "{}", s.status);

        // A directory whose parent exists is created on demand.
        let out = data.0.join("published");
        s.publish_dir = out.clone();
        assert_eq!(s.publish_target(), Ok(out.clone()));
        s.editor.place_current([0.0, 5.0]);
        s.save().unwrap();
        assert!(s.status.contains("published to"), "{}", s.status);
        // Two joints already make one bone, so a (one-piece) puppet ships.
        assert!(s.status.contains("puppet built"), "{}", s.status);
        assert_eq!(names(&out), ["poses", "puppets", "rigs"]);
        assert_eq!(names(&out.join("puppets")), ["humanoid.puppet.toml"]);
        assert_eq!(names(&out.join("rigs")), ["humanoid.png", "humanoid.toml"]);
        assert_eq!(names(&out.join("poses")), ["biped.toml"]);
        let read = |p: PathBuf| std::fs::read(p).unwrap();
        assert_eq!(read(out.join("rigs/humanoid.png")), read(data.0.join("rigs/humanoid.png")));

        // With the box unticked Save leaves the published copy alone; the
        // button still publishes the last SAVED bake.
        s.publish_on_save = false;
        s.editor.place_current([0.0, 4.9]);
        s.save().unwrap();
        assert!(!s.status.contains("ublish"), "{}", s.status);
        assert_ne!(read(out.join("rigs/humanoid.toml")), read(data.0.join("rigs/humanoid.toml")));
        s.publish().unwrap();
        assert_eq!(read(out.join("rigs/humanoid.toml")), read(data.0.join("rigs/humanoid.toml")));

        // A family never saved has nothing to publish.
        s.request(DocAction::OpenFamily("worm".into()));
        assert!(s.publish().unwrap_err().contains("save first"));
    }

    /// The real humanoid rig against the real silhouettes: Save builds the
    /// puppet, installs bake + puppet into the pool, and Publish ships it.
    #[test]
    fn save_builds_the_puppet_and_puts_it_on_the_stage() {
        let _guard = gs_field::data_dirs::VELLUM_FE_DIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let data = TempData::new("puppet");
        std::fs::copy(repo.join("outlines.json"), data.0.join("outlines.json")).unwrap();
        std::fs::create_dir_all(data.0.join("rigs")).unwrap();
        std::fs::copy(repo.join("rigs/humanoid.rig.toml"), data.0.join("rigs/humanoid.rig.toml")).unwrap();
        let pool = data.0.join("pool");
        let out = data.0.join("published");
        let mut s = Session::open(&data.0, out.clone(), Some(pool.clone())).unwrap();
        s.save().unwrap();
        assert!(s.status.contains("puppet built") && s.status.contains("installed in the pool"), "{}", s.status);
        let baked = std::fs::read_to_string(data.0.join("rigs/humanoid.puppet.toml")).unwrap();
        assert!(Puppet::from_baked_toml(&baked).is_ok());
        assert_eq!(names(&pool.join("global/puppets")), ["humanoid.puppet.toml"]);
        assert_eq!(names(&pool.join("global/rigs")), ["humanoid.png", "humanoid.toml"]);
        assert_eq!(names(&out.join("puppets")), ["humanoid.puppet.toml"]);

        // The pool copy is what gs_field now serves.
        gs_field::data_dirs::set_base_dir(pool.clone());
        crate::pool_dir::invalidate_caches();
        let set = gs_field::puppets::puppets();
        assert_eq!(set.get("humanoid").unwrap().source, gs_field::puppets::PuppetSource::Override);
        gs_field::data_dirs::clear_base_dir();
        crate::pool_dir::invalidate_caches();

        // A rig that can no longer make a puppet (one joint: no bone) takes
        // its old one with it.
        s.request(DocAction::NewFromTemplate { template: "biped".into(), view: View::Front });
        s.editor.place_current([0.0, 5.8]);
        s.save().unwrap();
        assert!(s.status.contains("no puppet yet"), "{}", s.status);
        assert!(!data.0.join("rigs/humanoid.puppet.toml").exists());
        assert!(!pool.join("global/puppets/humanoid.puppet.toml").exists());
    }

    /// Reload poses puts the data root's pose sets where the stage reads
    /// them; a file that does not parse stays out and is named.
    #[test]
    fn edited_poses_reach_the_stage_and_broken_ones_do_not() {
        let _guard = gs_field::data_dirs::VELLUM_FE_DIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let data = TempData::new("poses");
        std::fs::copy(repo.join("outlines.json"), data.0.join("outlines.json")).unwrap();
        std::fs::create_dir_all(data.0.join("poses")).unwrap();
        let biped = std::fs::read_to_string(repo.join("poses/biped.toml")).unwrap();
        let edited = biped.replace("[pose.kneel]", "[pose.bow]
torso_upper = { rot = [40.0, 0.0, 0.0] }

[pose.kneel]");
        std::fs::write(data.0.join("poses/biped.toml"), &edited).unwrap();
        std::fs::write(data.0.join("poses/avian.toml"), "this is not a pose file [").unwrap();
        let pool = data.0.join("pool");
        let mut s = Session::open(&data.0, data.0.join("published"), Some(pool.clone())).unwrap();

        let err = s.install_poses().unwrap_err();
        assert!(err.contains("1 pose sets installed") && err.contains("avian.toml"), "{err}");
        assert_eq!(names(&pool.join("global/puppets")), ["biped.toml"]);

        gs_field::data_dirs::set_base_dir(pool.clone());
        crate::pool_dir::invalidate_caches();
        let set = gs_field::puppets::puppets();
        let file = set.pose_file("biped").unwrap();
        assert_eq!(file.source, gs_field::puppets::PuppetSource::Override);
        assert!(gs_puppet::PoseSet::from_toml(&file.toml).unwrap().get("bow").is_some());
        // The broken avian file did not replace the built-in set.
        assert_eq!(set.pose_file("avian").unwrap().source, gs_field::puppets::PuppetSource::Embedded);
        gs_field::data_dirs::clear_base_dir();
        crate::pool_dir::invalidate_caches();
    }

    #[test]
    fn an_old_rig_file_loads_with_its_hierarchy_filled_in_and_is_not_dirty() {
        let data = TempData::new("migrate");
        let mut old = template("biped").unwrap().new_rig("humanoid", View::Front, 6.0, "crown");
        old.version = 0;
        old.joint_order.clear();
        for b in &mut old.bones {
            (b.parent, b.pivot) = (None, None);
        }
        std::fs::create_dir_all(data.0.join("rigs")).unwrap();
        std::fs::write(data.0.join("rigs/humanoid.rig.toml"), old.to_toml().unwrap()).unwrap();
        let s = open(&data.0).unwrap();
        assert!(s.status.contains("filled 10 bone parent(s)"), "{}", s.status);
        assert!(!s.editor.is_dirty());
        assert_eq!(s.editor.rig().hierarchy_issues(), []);
        assert_eq!(s.editor.current(), Some("crown"));
    }

    #[test]
    fn unsaved_changes_park_the_action_until_confirmed() {
        let data = TempData::new("confirm");
        let mut s = open(&data.0).unwrap();
        s.editor.place_current([0.0, 5.8]);
        s.request(DocAction::OpenFamily("worm".into()));
        assert_eq!(s.pending, Some(DocAction::OpenFamily("worm".into())));
        assert_eq!(s.editor.rig().family, "humanoid");
        s.cancel_pending();
        assert!(s.pending.is_none() && s.editor.is_dirty());

        s.request(DocAction::Quit);
        assert!(!s.quit_approved);
        s.cancel_pending();

        s.request(DocAction::OpenFamily("worm".into()));
        s.confirm_pending();
        let rig = s.editor.rig();
        assert_eq!((rig.family.as_str(), rig.body_type.as_str(), rig.view), ("worm", "serpentine", View::Side));
        assert_eq!((rig.ref_ft, rig.axis.as_str()), (2.0, "diameter"));

        // Clean document: quitting needs no prompt.
        s.request(DocAction::Quit);
        assert!(s.quit_approved && s.pending.is_none());
    }

    #[test]
    fn a_missing_view_falls_back_to_the_one_that_exists() {
        let data = TempData::new("view");
        let mut s = open(&data.0).unwrap();
        s.request(DocAction::OpenFamily("worm".into()));
        s.request(DocAction::NewFromTemplate { template: "serpentine".into(), view: View::Front });
        assert_eq!(s.editor.rig().view, View::Side);
        assert!(s.outline().is_some());
    }

    #[test]
    fn bakes_are_cached_per_revision_and_size() {
        let data = TempData::new("bake");
        let mut s = open(&data.0).unwrap();
        let first = s.baked(false).unwrap().serial;
        assert_eq!(s.baked(false).unwrap().serial, first, "no change, no re-bake");
        assert_eq!(s.baked(false).unwrap().baked.map.grid.h, FULL_PX);

        s.editor.place_current([0.0, 5.8]);
        s.editor.begin_drag("crown", [0.0, 5.8]);
        s.editor.drag_to([0.1, 5.7]);
        let dragging = s.editor.is_dragging();
        let small = s.baked(dragging).unwrap();
        assert_eq!(small.baked.map.grid.h, DRAG_PX);
        assert!(small.serial > first);
        s.editor.end_drag();
        let dragging = s.editor.is_dragging();
        assert_eq!(s.baked(dragging).unwrap().baked.map.grid.h, FULL_PX);
        assert!(!s.baked(false).unwrap().warnings.is_empty());
    }

    #[test]
    fn depth_guesses_follow_the_rig_and_need_a_second_silhouette() {
        let data = TempData::new("depth");
        let mut s = open(&data.0).unwrap();
        assert!(s.depth_available(), "humanoid has both views in the fixture");
        s.editor.place_current([0.0, 5.8]);
        s.refresh_depth();
        assert!(s.depth().joint(s.editor.rig(), "crown").value.is_finite());
        // Move the joint: the guess is re-read, not stale.
        s.editor.set_current("crown");
        s.editor.place_current([0.0, 1.0]);
        s.refresh_depth();
        assert!(s.depth().guess("crown").is_some());
        assert!(s.depth().available());

        // 'worm' has only a side outline: depth mode is unavailable, not a panic.
        s.request(DocAction::OpenFamily("worm".into()));
        s.confirm_pending(); // the edits above made it dirty
        assert_eq!(s.editor.rig().family, "worm");
        assert!(!s.depth_available());
        s.refresh_depth();
        assert!(!s.depth().available());
        let rig = s.editor.rig().clone();
        assert!(s.depth().unavailable_reason(&rig).contains("front"));
    }

    #[test]
    fn saving_writes_overrides_and_leaves_guesses_out() {
        let data = TempData::new("depthsave");
        let mut s = open(&data.0).unwrap();
        s.editor.place_current([0.0, 5.8]);
        s.editor.place_current([0.0, 5.0]);
        s.editor.set_joint_depth("crown", 0.31);
        s.editor.end_continuous_edit();
        s.save().unwrap();
        let text = std::fs::read_to_string(data.0.join("rigs/humanoid.rig.toml")).unwrap();
        let depth_table = text.split("[depth]").nth(1).unwrap_or_default();
        let depth_table = depth_table.split("\n[").next().unwrap();
        assert!(depth_table.contains("crown = 0.31"), "{text}");
        assert_eq!(
            depth_table.lines().filter(|l| l.contains('=')).count(),
            1,
            "only the override is written; the guesses are not: {depth_table}"
        );

        let again = open(&data.0).unwrap();
        assert_eq!(again.editor.rig().depth.len(), 1);
        assert!((again.editor.rig().depth["crown"] - 0.31).abs() < 1e-4);
    }

    #[test]
    fn the_preview_puppet_follows_the_rig_and_is_cached_per_revision() {
        let data = TempData::new("preview");
        let mut s = open(&data.0).unwrap();
        assert!(s.puppet().is_err(), "nothing placed yet: no puppet");
        s.editor.place_current([0.0, 5.8]);
        s.editor.place_current([0.0, 5.0]);
        let first = s.puppet().as_ref().unwrap().clone();
        assert_eq!(first.family, "humanoid");
        assert!(std::ptr::eq(s.puppet(), s.puppet()), "same revision: the cached build");
        // Move a joint: rebuilt, and it shows.
        s.editor.set_current("crown");
        s.editor.place_current([0.0, 5.5]);
        assert_ne!(s.puppet().as_ref().unwrap(), &first);
    }

    #[test]
    fn a_corrupt_saved_rig_falls_back_to_the_template() {
        let data = TempData::new("corrupt");
        std::fs::create_dir_all(data.0.join("rigs")).unwrap();
        std::fs::write(data.0.join("rigs/humanoid.rig.toml"), "this is = not [a rig").unwrap();
        let s = open(&data.0).unwrap();
        assert!(s.status.starts_with("Could not load"), "{}", s.status);
        assert_eq!(s.editor.rig().body_type, "biped");
    }
}
