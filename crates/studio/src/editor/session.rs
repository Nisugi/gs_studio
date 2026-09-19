//! The open document: which family, its silhouette, the editor, the cached
//! bake, and the save / open / discard rules.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use rig::{
    bake_mask, family_default, template, templates, write_baked, Baked, OutlineEntry, Outlines,
    Rig, Template, View,
};
use rig_bake::{rasterize, Mask};

use super::depth::DepthModel;
use super::state::Editor;
use super::warnings::warnings;

/// Map long side while something is being dragged / once it is released.
pub const DRAG_PX: usize = 256;
pub const FULL_PX: usize = 512;

/// Environment override for where baked rigs are published.
pub const PUBLISH_ENV_VAR: &str = "GS_STUDIO_PUBLISH";

/// Where baked rigs go for game clients to embed: `override_dir` if given,
/// else the workspace's `crates/gs_field/data/rigs`, found relative to the
/// data root (`<data>/../crates/gs_field/data/rigs`).
pub fn resolve_publish_dir(data_root: &Path, override_dir: Option<&str>) -> PathBuf {
    match override_dir.map(str::trim).filter(|d| !d.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => data_root.join("..").join("crates").join("gs_field").join("data").join("rigs"),
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
    /// Where Publish copies `<family>.png` + `<family>.toml`.
    pub publish_dir: PathBuf,
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
}

impl Session {
    /// Load `<data_root>/outlines.json` and open a first family.
    pub fn open(data_root: &Path) -> Result<Session, String> {
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
            publish_dir: resolve_publish_dir(data_root, std::env::var(PUBLISH_ENV_VAR).ok().as_deref()),
            publish_on_save: true,
            masks: HashMap::new(),
            bake: None,
            bake_serial: 0,
            other_mask: None,
            depth: None,
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
        if self.other_mask.is_none() {
            let outline = self.other_outline().map(|o| o.outline());
            self.other_mask = Some(outline.map(|o| rasterize(&o, FULL_PX)));
        }
        let mask = self.other_mask.as_ref().and_then(Clone::clone);
        self.depth = Some((revision, DepthModel::new(self.editor.rig(), mask)));
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

    /// Copy the SAVED `<family>.png` + `<family>.toml` (never the
    /// `.rig.toml` source) into the publish directory. Publishes what is on
    /// disk: unsaved edits are not in it.
    pub fn publish(&mut self) -> Result<PathBuf, String> {
        let dir = self.publish_target()?;
        let family = self.editor.rig().family.clone();
        if !self.has_saved_bake(&family) {
            return Err(format!("{family} has no saved bake yet: save first"));
        }
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for name in [format!("{family}.png"), format!("{family}.toml")] {
            let (from, to) = (self.rigs_dir().join(&name), dir.join(&name));
            std::fs::copy(&from, &to).map_err(|e| format!("{} -> {}: {e}", from.display(), to.display()))?;
        }
        self.status = format!("Published {family}.png + {family}.toml to {}", dir.display());
        Ok(dir)
    }

    /// Write `<family>.rig.toml`, `<family>.png` and `<family>.toml` into
    /// `<data>/rigs/`; then, with "publish on save" on and somewhere to
    /// publish to, copy the two baked files there as well. A publish that
    /// fails does not fail the save: the status line says what happened.
    pub fn save(&mut self) -> Result<(), String> {
        let family = self.editor.rig().family.clone();
        let path = self.rig_path(&family);
        let dir = self.rigs_dir();
        let baked = self.baked(false).ok_or("no silhouette for this family/view")?.baked.clone();
        let rig = self.editor.rig();
        rig.save(&path).map_err(|e| e.to_string())?;
        write_baked(rig, &baked, &dir).map_err(|e| e.to_string())?;
        self.editor.mark_saved();
        let saved = format!("Saved {} (+ {family}.png, {family}.toml)", path.display());
        self.status = match self.publish_on_save.then(|| self.publish_target()) {
            Some(Ok(_)) => match self.publish() {
                Ok(dir) => format!("{saved}; published to {}", dir.display()),
                Err(e) => format!("{saved}; PUBLISH FAILED: {e}"),
            },
            Some(Err(_)) => format!("{saved}; not published (no publish directory)"),
            None => saved,
        };
        Ok(())
    }
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
        let s = Session::open(&data.0).unwrap();
        let rig = s.editor.rig();
        assert_eq!((rig.family.as_str(), rig.body_type.as_str(), rig.view), ("humanoid", "biped", View::Front));
        assert_eq!((rig.ref_ft, rig.axis.as_str()), (6.0, "crown"));
        assert!(!s.editor.is_dirty() && s.fit_requested);
    }

    #[test]
    fn a_missing_data_dir_is_an_error_not_a_panic() {
        assert!(Session::open(Path::new("definitely/not/here")).is_err());
    }

    #[test]
    fn save_writes_three_files_and_reopening_loads_them() {
        let data = TempData::new("save");
        let mut s = Session::open(&data.0).unwrap();
        s.editor.place_current([0.0, 5.8]);
        s.editor.place_current([0.0, 5.0]);
        assert!(s.editor.is_dirty());
        s.save().unwrap();
        assert!(!s.editor.is_dirty());
        for f in ["humanoid.rig.toml", "humanoid.png", "humanoid.toml"] {
            assert!(data.0.join("rigs").join(f).is_file(), "{f}");
        }

        let mut again = Session::open(&data.0).unwrap();
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
        assert!(default.ends_with("crates/gs_field/data/rigs") && default.starts_with("ws/data/.."), "{default:?}");
        assert_eq!(resolve_publish_dir(Path::new("ws/data"), Some("  ")), default);
        assert_eq!(resolve_publish_dir(Path::new("ws/data"), Some(" out/rigs ")), PathBuf::from("out/rigs"));
    }

    #[test]
    fn save_publishes_the_baked_pair_but_never_the_source() {
        let data = TempData::new("pub");
        let mut s = Session::open(&data.0).unwrap();
        // Default location: <data>/../crates/... is not there -> unavailable,
        // and saving still works.
        s.publish_dir = resolve_publish_dir(&data.0.join("nested/data"), None);
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
        let mut names: Vec<String> =
            std::fs::read_dir(&out).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["humanoid.png", "humanoid.toml"]);
        assert_eq!(std::fs::read(out.join("humanoid.png")).unwrap(), std::fs::read(data.0.join("rigs/humanoid.png")).unwrap());

        // With the box unticked Save leaves the published copy alone; the
        // button still publishes the last SAVED bake.
        s.publish_on_save = false;
        s.editor.place_current([0.0, 4.9]);
        s.save().unwrap();
        assert!(!s.status.contains("ublish"), "{}", s.status);
        assert_ne!(std::fs::read(out.join("humanoid.toml")).unwrap(), std::fs::read(data.0.join("rigs/humanoid.toml")).unwrap());
        s.publish().unwrap();
        assert_eq!(std::fs::read(out.join("humanoid.toml")).unwrap(), std::fs::read(data.0.join("rigs/humanoid.toml")).unwrap());

        // A family never saved has nothing to publish.
        s.request(DocAction::OpenFamily("worm".into()));
        assert!(s.publish().unwrap_err().contains("save first"));
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
        let s = Session::open(&data.0).unwrap();
        assert!(s.status.contains("filled 10 bone parent(s)"), "{}", s.status);
        assert!(!s.editor.is_dirty());
        assert_eq!(s.editor.rig().hierarchy_issues(), []);
        assert_eq!(s.editor.current(), Some("crown"));
    }

    #[test]
    fn unsaved_changes_park_the_action_until_confirmed() {
        let data = TempData::new("confirm");
        let mut s = Session::open(&data.0).unwrap();
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
        let mut s = Session::open(&data.0).unwrap();
        s.request(DocAction::OpenFamily("worm".into()));
        s.request(DocAction::NewFromTemplate { template: "serpentine".into(), view: View::Front });
        assert_eq!(s.editor.rig().view, View::Side);
        assert!(s.outline().is_some());
    }

    #[test]
    fn bakes_are_cached_per_revision_and_size() {
        let data = TempData::new("bake");
        let mut s = Session::open(&data.0).unwrap();
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
        let mut s = Session::open(&data.0).unwrap();
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
        let mut s = Session::open(&data.0).unwrap();
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

        let again = Session::open(&data.0).unwrap();
        assert_eq!(again.editor.rig().depth.len(), 1);
        assert!((again.editor.rig().depth["crown"] - 0.31).abs() < 1e-4);
    }

    #[test]
    fn a_corrupt_saved_rig_falls_back_to_the_template() {
        let data = TempData::new("corrupt");
        std::fs::create_dir_all(data.0.join("rigs")).unwrap();
        std::fs::write(data.0.join("rigs/humanoid.rig.toml"), "this is = not [a rig").unwrap();
        let s = Session::open(&data.0).unwrap();
        assert!(s.status.starts_with("Could not load"), "{}", s.status);
        assert_eq!(s.editor.rig().body_type, "biped");
    }
}
