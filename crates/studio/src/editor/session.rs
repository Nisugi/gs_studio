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

use super::state::Editor;
use super::warnings::warnings;

/// Map long side while something is being dragged / once it is released.
pub const DRAG_PX: usize = 256;
pub const FULL_PX: usize = 512;

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
    masks: HashMap<usize, Mask>,
    bake: Option<BakeResult>,
    bake_serial: u64,
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
            masks: HashMap::new(),
            bake: None,
            bake_serial: 0,
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
        self.fit_requested = true;
    }

    /// Load the family's saved rig if there is one, else start from its
    /// default template.
    fn open_family(&mut self, family: &str) {
        let path = self.rig_path(family);
        if path.is_file() {
            match Rig::load(&path) {
                Ok(rig) => {
                    let tpl = template(&rig.body_type)
                        .or_else(|| family_default(family).and_then(|d| template(d.template)))
                        .unwrap_or_else(|| templates().remove(0));
                    self.status = format!("Loaded {}", path.display());
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
            let warnings = warnings(self.editor.rig(), self.editor.template(), &baked, mask);
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

    /// Write `<family>.rig.toml`, `<family>.png` and `<family>.toml` into
    /// `<data>/rigs/`.
    pub fn save(&mut self) -> Result<(), String> {
        let family = self.editor.rig().family.clone();
        let path = self.rig_path(&family);
        let dir = self.rigs_dir();
        let baked = self.baked(false).ok_or("no silhouette for this family/view")?.baked.clone();
        let rig = self.editor.rig();
        rig.save(&path).map_err(|e| e.to_string())?;
        write_baked(rig, &baked, &dir).map_err(|e| e.to_string())?;
        self.editor.mark_saved();
        self.status = format!("Saved {} (+ {family}.png, {family}.toml)", path.display());
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
    fn a_corrupt_saved_rig_falls_back_to_the_template() {
        let data = TempData::new("corrupt");
        std::fs::create_dir_all(data.0.join("rigs")).unwrap();
        std::fs::write(data.0.join("rigs/humanoid.rig.toml"), "this is = not [a rig").unwrap();
        let s = Session::open(&data.0).unwrap();
        assert!(s.status.starts_with("Could not load"), "{}", s.status);
        assert_eq!(s.editor.rig().body_type, "biped");
    }
}
