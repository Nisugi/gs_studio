//! The open document: which family, its silhouette, the editor, the cached
//! bake, and the save / open / discard rules.

use gs_puppet_author::prelude::*;
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

/// [`Session::publish_target`] for a directory, without a session (a
/// worker thread asks it: see `fs_cache::Background`).
pub fn publish_target_of(dir: &Path) -> Result<PathBuf, String> {
    let parent_ok = dir.parent().is_some_and(|p| p.as_os_str().is_empty() || p.is_dir());
    if dir.is_dir() || parent_ok {
        Ok(dir.to_path_buf())
    } else {
        Err(format!(
            "Nowhere to publish: {} does not exist (and neither does its parent). Set {PUBLISH_ENV_VAR} to a \
             directory, or run from the gs_studio workspace.",
            dir.display()
        ))
    }
}

/// [`Session::has_saved_bake`] for a rigs directory.
pub fn has_saved_bake_in(rigs: &Path, family: &str) -> bool {
    rigs.join(format!("{family}.png")).is_file() && rigs.join(format!("{family}.toml")).is_file()
}

/// Every family with a saved `<family>.rig.toml` in a rigs directory.
pub fn saved_rigs_in(rigs: &Path) -> std::collections::BTreeSet<String> {
    std::fs::read_dir(rigs)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_suffix(".rig.toml").map(str::to_owned))
        .collect()
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
        if let Some(Ok(n)) = session.sync_pool_poses() {
            session.status = format!("{n} pose sets refreshed in the pool");
        }
        Ok(session)
    }

    /// Bring the pool's pose sets up to date with `<data>/poses/` when any
    /// differs or is missing there (the pool's copy shadows the built-in
    /// one on the stage, so a pose file that gained a table -- `[idle]` --
    /// would otherwise stay invisible until the next Save). Runs at open
    /// and when the pool changes. None = nothing to do (no pool, no poses,
    /// or all the same); else [`install_poses`](Self::install_poses)'s
    /// result.
    pub fn sync_pool_poses(&mut self) -> Option<Result<usize, String>> {
        let pool = self.pool.clone()?;
        let into = pool.join("global").join("puppets");
        let mut stale = false;
        for entry in std::fs::read_dir(self.data_root.join("poses")).into_iter().flatten().flatten() {
            let path = entry.path();
            if !path.extension().is_some_and(|e| e == "toml") {
                continue;
            }
            let Ok(ours) = std::fs::read(&path) else { continue };
            let theirs = std::fs::read(into.join(path.file_name().unwrap_or_default())).ok();
            if theirs.as_deref() != Some(ours.as_slice()) {
                stale = true;
                break;
            }
        }
        stale.then(|| self.install_poses())
    }

    pub fn rigs_dir(&self) -> PathBuf {
        self.data_root.join("rigs")
    }

    pub fn rig_path(&self, family: &str) -> PathBuf {
        self.rigs_dir().join(Rig::file_name(family))
    }

    #[cfg_attr(not(test), allow(dead_code))]
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
        let _t = crate::perf_log::span("rigs: open family");
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

    /// [`Session::other_outline`] as DEPTH mode draws it: for a rig drawn
    /// from above, turned head up (world (z, y) -> (-y, z)) and slid along
    /// the plan so the two are centre on centre (`gs_puppet_author::plan_shift`,
    /// here from the outlines' boxes), the frame its depths are read and
    /// edited in.
    pub fn other_outline_shown(&self) -> Option<OutlineEntry> {
        let entry = self.other_outline()?.clone();
        let rig = self.editor.rig();
        if !rig.from_above {
            return Some(entry);
        }
        let mid = |e: &OutlineEntry, axis: usize| {
            let v = e.subpaths.iter().flatten().map(|p| p[axis]);
            let (lo, hi) = v.fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)));
            (lo + hi) / 2.0
        };
        let shift = self.outlines.get(&rig.family, rig.view).map_or(0.0, |plan| mid(plan, 1) - mid(&entry, 0));
        Some(OutlineEntry {
            ref_ft: entry.ref_ft,
            subpaths: entry.subpaths.iter().map(|sub| sub.iter().map(|p| [-p[1], p[0] + shift]).collect()).collect(),
        })
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
        if let Some(outline) = self.outline().map(|o| o.outline()) {
            self.masks.entry(FULL_PX).or_insert_with(|| rasterize(&outline, FULL_PX));
        }
        let mask = self.other_mask.as_ref().and_then(Clone::clone);
        self.depth = Some((revision, DepthModel::new(self.editor.rig(), mask, self.masks.get(&FULL_PX))));
    }

    fn ensure_other_mask(&mut self) {
        if self.other_mask.is_none() {
            let outline = self.other_outline().map(|o| o.outline());
            self.other_mask = Some(outline.map(|o| rasterize(&o, FULL_PX)));
        }
    }

    /// The rig's 3D puppet, rebuilt only when the rig has changed since the
    /// last call. Built from the same full-size masks the bake and the depth
    /// guesses use (`FULL_PX` = `gs_puppet_author::MASK_PX`), so it is exactly what
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

    /// The puppet [`Session::puppet`] last built, even if the rig has moved
    /// on since: for readouts that must not rebuild it mid-drag.
    pub fn last_puppet(&self) -> Option<&Puppet> {
        self.puppet.as_ref().and_then(|(_, p)| p.as_ref().ok())
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
        publish_target_of(&self.publish_dir)
    }

    /// True when `<data>/rigs/` holds a saved bake of this family to publish.
    pub fn has_saved_bake(&self, family: &str) -> bool {
        has_saved_bake_in(&self.rigs_dir(), family)
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
    /// (`puppets/<family>.puppet.toml`), its painted textures (the family's
    /// and its creatures', `puppets/skins/<family>/`), every pose set in
    /// `<data>/poses/` (`poses/`), the held-props file (`props/weapons.toml`)
    /// and the clothing kit and outfits (`outfits/`) into the publish
    /// directory. Never the
    /// `.rig.toml` source. Publishes what is on disk: unsaved edits are not
    /// in it.
    pub fn publish(&mut self) -> Result<PathBuf, String> {
        let dir = self.publish_target()?;
        let family = self.editor.rig().family.clone();
        if !self.has_saved_bake(&family) {
            return Err(format!("{family} has no saved bake yet: save first"));
        }
        // The hand-edited files are checked before anything is copied: a
        // pose set that did not parse replaced the good published one under
        // a report of success, and the game lost that body type's poses.
        let broken = self.broken_shared_files();
        if !broken.is_empty() {
            return Err(format!("nothing published; fix first: {}", broken.join("; ")));
        }
        let (mut shipped, mut removed) = (Vec::new(), Vec::new());
        // What the family no longer has goes too. Left behind, a puppet of
        // a rig that could no longer build one, and a texture whose sheet
        // was gone, stayed published under a report of success.
        let mut take_away = |stale: PathBuf, what: String| -> Result<(), String> {
            if stale.is_file() {
                std::fs::remove_file(&stale).map_err(|e| format!("{}: {e}", stale.display()))?;
                removed.push(what);
            }
            Ok(())
        };
        let files = self.shipped_files(&family);
        if !files.iter().any(|(_, sub)| *sub == "puppets") {
            let puppet = Puppet::baked_file_name(&family);
            take_away(dir.join("puppets").join(&puppet), format!("puppets/{puppet}"))?;
        }
        for (name, sub) in files {
            copy_into(&self.rigs_dir().join(&name), &dir.join(sub))?;
            shipped.push(format!("{sub}/{name}"));
        }
        // A sheet built for another layout of the rig (it changed since, at
        // the same size) is not shipped: the game would smear its paint.
        let saved = self.saved_puppet(&family);
        let figure = saved.as_ref();
        let mut refused = Vec::new();
        let sheets = [
            (super::texture::sheet_path(&self.data_root, &family), gs_field::puppets::texture_file_name(&family)),
            (super::texture::normal_sheet_path(&self.data_root, &family), gs_field::puppets::normal_file_name(&family)),
        ];
        for (sheet, published) in sheets {
            let stale = figure.filter(|_| sheet.is_file()).and_then(|p| super::texture::stale_sheet_for(&sheet, p));
            if sheet.is_file() && stale.is_none() {
                let to = dir.join("puppets").join(&published);
                std::fs::create_dir_all(dir.join("puppets")).map_err(|e| e.to_string())?;
                std::fs::copy(&sheet, &to).map_err(|e| format!("{} -> {}: {e}", sheet.display(), to.display()))?;
                shipped.push(format!("puppets/{published}"));
            } else {
                take_away(dir.join("puppets").join(&published), format!("puppets/{published}"))?;
            }
            refused.extend(stale);
        }
        let (skins, stale_skins) = super::texture::ship_skins(&self.data_root, &family, saved.as_ref(), &self.outlines, &dir.join("puppets"))?;
        refused.extend(stale_skins);
        if skins > 0 {
            shipped.push(format!("{skins} creature skin(s)"));
        }
        for name in super::texture::ship_outfits(&self.data_root, &dir.join("outfits"))? {
            shipped.push(format!("outfits/{name}"));
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
        let props = self.props_path();
        if props.is_file() {
            copy_into(&props, &dir.join("props"))?;
            shipped.push("props/weapons.toml".into());
        }
        self.status = format!("Published {} to {}", shipped.join(", "), dir.display());
        if !removed.is_empty() {
            self.status += &format!("; removed {}", removed.join(", "));
        }
        if !refused.is_empty() {
            self.status += &format!("; NOT SHIPPED, build again: {}", refused.join("; "));
        }
        Ok(dir)
    }

    /// The family's puppet as saved (`<data>/rigs/<family>.puppet.toml`):
    /// what is installed and published, and what the game lays a sheet out
    /// for.
    fn saved_puppet(&self, family: &str) -> Option<Puppet> {
        let text = std::fs::read_to_string(self.rigs_dir().join(Puppet::baked_file_name(family))).ok()?;
        Puppet::from_baked_toml(&text).ok()
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
        self.baked(false).ok_or("no silhouette for this family/view")?;
        self.editor.rig().save(&path).map_err(|e| e.to_string())?;
        // Bake and build from the rig AS WRITTEN: the file rounds joints to
        // a ten-thousandth of a foot, and a bake of the unrounded ones would
        // disagree with its own rig file by a step.
        let written = rig::Rig::load(&path).map_err(|e| e.to_string())?;
        let mask = self.masks.get(&FULL_PX).ok_or("no silhouette for this family/view")?;
        let baked = bake_mask(&written, mask);
        write_baked(&written, &baked, &dir).map_err(|e| e.to_string())?;
        self.editor.mark_saved();
        let mut notes = vec![format!("Saved {}", path.display())];

        let puppet_path = dir.join(Puppet::baked_file_name(&family));
        match Puppet::build(&written, &self.outlines).and_then(|p| p.to_baked_toml()) {
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
                Ok(refused) => {
                    notes.push("installed in the pool".into());
                    if !refused.is_empty() {
                        notes.push(format!("NOT INSTALLED, build again: {}", refused.join("; ")));
                    }
                }
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

    /// `<data>/props/weapons.toml`: what puppets can hold, and its shapes.
    pub fn props_path(&self) -> PathBuf {
        self.data_root.join("props").join("weapons.toml")
    }

    /// Every pose set in `<data>/poses/`, each with why it does not parse.
    fn pose_sets(&self) -> Vec<(PathBuf, Result<(), String>)> {
        std::fs::read_dir(self.data_root.join("poses"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
            .map(|p| {
                let parsed = std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|t| gs_puppet::PoseSet::from_toml(&t));
                (p, parsed.map(|_| ()))
            })
            .collect()
    }

    /// The held-props file, when there is one, with why it does not parse.
    fn props_file(&self) -> Option<(PathBuf, Result<(), String>)> {
        let props = self.props_path();
        props.is_file().then(|| {
            let parsed = std::fs::read_to_string(&props).map_err(|e| e.to_string()).and_then(|t| gs_puppet::PropSet::from_toml(&t));
            (props, parsed.map(|_| ()))
        })
    }

    /// What is wrong with the hand-edited files every publish ships (pose
    /// sets, held props, clothing kit and outfits), one line per file.
    fn broken_shared_files(&self) -> Vec<String> {
        let mut broken: Vec<String> = self
            .pose_sets()
            .into_iter()
            .filter_map(|(p, parsed)| parsed.err().map(|e| format!("poses/{}: {e}", p.file_name().unwrap_or_default().to_string_lossy())))
            .collect();
        if let Some((_, Err(e))) = self.props_file() {
            broken.push(format!("props/weapons.toml: {e}"));
        }
        if let Err(e) = super::texture::outfits(&self.data_root) {
            broken.push(format!("outfits: {e}"));
        }
        broken
    }

    /// Put every pose set in `<data>/poses/` into `<pool>/global/puppets/`,
    /// where it replaces the built-in set of that body type on the stage, the
    /// held-props file into `<pool>/global/props/`, and the clothing kit and
    /// outfits into `<pool>/global/outfits/`; then drop gs_field's
    /// caches so the stage re-reads them. A file that does not parse is left
    /// out (the stage keeps what it had) and named in the error. Returns how
    /// many pose sets were installed.
    pub fn install_poses(&mut self) -> Result<usize, String> {
        let pool = self.pool.clone().ok_or("no pool to install poses into")?;
        let into = pool.join("global").join("puppets");
        let mut installed = 0;
        let mut broken = Vec::new();
        for (path, parsed) in self.pose_sets() {
            match parsed {
                Ok(()) => {
                    copy_into(&path, &into)?;
                    installed += 1;
                }
                Err(e) => broken.push(format!("{}: {e}", path.file_name().unwrap_or_default().to_string_lossy())),
            }
        }
        match self.props_file() {
            Some((props, Ok(()))) => copy_into(&props, &pool.join("global").join("props"))?,
            Some((_, Err(e))) => broken.push(format!("props/weapons.toml: {e}")),
            None => {}
        }
        if let Err(e) = super::texture::ship_outfits(&self.data_root, &pool.join("global").join("outfits")) {
            broken.push(e);
        }
        crate::pool_dir::invalidate_caches();
        if broken.is_empty() {
            Ok(installed)
        } else {
            Err(format!("{installed} pose sets installed; NOT installed (fix and reload): {}", broken.join("; ")))
        }
    }

    /// Copy the saved bake into `<pool>/global/rigs/` and the saved puppet,
    /// its painted texture and its creatures' skins into
    /// `<pool>/global/puppets/` (removing what this family no longer has),
    /// then drop gs_field's caches so they are read again.
    /// Sheets built for another layout of the rig are not installed (and an
    /// installed copy is taken away): the list says which.
    fn install_in_pool(&self, family: &str, pool: &Path) -> Result<Vec<String>, String> {
        let global = pool.join("global");
        let files = self.shipped_files(family);
        if !files.iter().any(|(_, sub)| *sub == "puppets") {
            let _ = std::fs::remove_file(global.join("puppets").join(Puppet::baked_file_name(family)));
        }
        for (name, sub) in files {
            copy_into(&self.rigs_dir().join(&name), &global.join(sub))?;
        }
        // The painted texture rides beside the puppet (or goes with it), and
        // so do its creatures' skins.
        let saved = self.saved_puppet(family);
        let figure = saved.as_ref();
        let mut refused = Vec::new();
        let sheets = [
            (super::texture::sheet_path(&self.data_root, family), gs_field::puppets::texture_file_name(family)),
            (super::texture::normal_sheet_path(&self.data_root, family), gs_field::puppets::normal_file_name(family)),
        ];
        for (sheet, name) in sheets {
            let installed = global.join("puppets").join(name);
            let stale = figure.filter(|_| sheet.is_file()).and_then(|p| super::texture::stale_sheet_for(&sheet, p));
            if sheet.is_file() && stale.is_none() {
                std::fs::create_dir_all(global.join("puppets")).map_err(|e| e.to_string())?;
                std::fs::copy(&sheet, &installed).map_err(|e| format!("{} -> {}: {e}", sheet.display(), installed.display()))?;
            } else {
                let _ = std::fs::remove_file(&installed);
            }
            refused.extend(stale);
        }
        let (_, stale_skins) = super::texture::ship_skins(&self.data_root, family, saved.as_ref(), &self.outlines, &global.join("puppets"))?;
        refused.extend(stale_skins);
        crate::pool_dir::invalidate_caches();
        Ok(refused)
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

    /// Timing probe (run by hand): what opening each family from the Rigs
    /// dropdown costs, step by step.
    #[test]
    #[ignore]
    fn open_family_timing() {
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let mut s = Session::open(&data, data.join("no/such/publish/dir"), None).unwrap();
        let families: Vec<String> = s.outlines.families().into_iter().filter(|f| s.has_saved_rig(f)).collect();
        for f in families {
            let t = std::time::Instant::now();
            s.request(DocAction::OpenFamily(f.clone()));
            let open = t.elapsed();
            let t = std::time::Instant::now();
            s.baked(false);
            let bake = t.elapsed();
            let t = std::time::Instant::now();
            s.refresh_depth();
            let depth = t.elapsed();
            let t = std::time::Instant::now();
            let ok = s.puppet().is_ok();
            let puppet = t.elapsed();
            let t = std::time::Instant::now();
            if let Ok(p) = s.puppet() {
                let posed = p.pose(&gs_puppet::Pose::default());
                let cam = gs_puppet::Camera::framing(p.height.max(0.5), [380.0, 600.0]);
                let _ = posed.skin(&|v| cam.project(v), &gs_puppet::Look::default());
            }
            let skin = t.elapsed();
            eprintln!("{f:12} open {open:>9.1?} bake {bake:>9.1?} depth {depth:>9.1?} puppet {puppet:>9.1?} ({ok}) skin {skin:>9.1?}");
        }
    }

    /// Where a Save's time goes (run by hand, `--ignored --nocapture`), on a
    /// throwaway copy of the data root.
    #[test]
    #[ignore]
    fn save_timing() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        for sub in ["rigs", "poses", "props"] {
            std::fs::create_dir_all(data.join(sub)).unwrap();
            for e in std::fs::read_dir(repo.join(sub)).unwrap().flatten() {
                std::fs::copy(e.path(), data.join(sub).join(e.file_name())).unwrap();
            }
        }
        std::fs::copy(repo.join("outlines.json"), data.join("outlines.json")).unwrap();
        let pool = dir.path().join("pool");
        let mut s = Session::open(&data, dir.path().join("publish"), Some(pool)).unwrap();
        for f in ["humanoid", "elemental", "canine"] {
            s.request(DocAction::OpenFamily(f.into()));
            s.baked(false);
            let t = std::time::Instant::now();
            s.save().unwrap();
            let save = t.elapsed();
            let rig = s.editor.rig().clone();
            let t = std::time::Instant::now();
            let _ = Puppet::build(&rig, &s.outlines);
            let build = t.elapsed();
            let t = std::time::Instant::now();
            let _ = s.publish();
            let publish = t.elapsed();
            let t = std::time::Instant::now();
            let _ = s.install_poses();
            let install = t.elapsed();
            eprintln!("{f:10} save {save:>9.1?}  (puppet build {build:>9.1?}, publish {publish:>9.1?}, install poses {install:>9.1?})");
        }
    }

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

    /// The pool's pose sets are refreshed from `<data>/poses/` at open when
    /// one differs (or is missing), and left alone when they all match.
    #[test]
    fn opening_refreshes_stale_pose_sets_in_the_pool() {
        let data = TempData::new("sync_poses");
        std::fs::create_dir_all(data.0.join("poses")).unwrap();
        let ours = "body_type = \"biped\"
[pose.stand]
[idle]
period = 4.0
neck = { rot = [1.0, 0.0, 0.0] }
";
        std::fs::write(data.0.join("poses/biped.toml"), ours).unwrap();
        let pool = data.0.join("pool");
        let installed = pool.join("global/puppets/biped.toml");
        std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
        std::fs::write(&installed, "body_type = \"biped\"
[pose.stand]
").unwrap();

        let s = Session::open(&data.0, data.0.join("no/publish"), Some(pool.clone())).unwrap();
        assert_eq!(std::fs::read_to_string(&installed).unwrap(), ours, "the stale copy was replaced");
        assert!(s.status.contains("1 pose sets refreshed"), "{}", s.status);
        crate::pool_dir::invalidate_caches();

        let mut s = Session::open(&data.0, data.0.join("no/publish"), Some(pool.clone())).unwrap();
        assert!(!s.status.contains("refreshed"), "all the same: nothing said ({})", s.status);
        assert!(s.sync_pool_poses().is_none());
        std::fs::remove_file(&installed).unwrap();
        assert_eq!(s.sync_pool_poses().unwrap().unwrap(), 1, "a missing copy is installed");
        assert!(installed.is_file());
        crate::pool_dir::invalidate_caches();
    }

    /// A joint placed between ten-thousandths of a foot is saved rounded;
    /// the bake written beside it is the bake of the rig AS WRITTEN.
    #[test]
    fn a_save_bakes_the_rig_as_written() {
        let data = TempData::new("save_rounded");
        let mut s = open(&data.0).unwrap();
        s.editor.place_current([0.123456, 5.876543]);
        s.editor.place_current([-0.000049, 5.000051]);
        s.save().unwrap();
        let written = rig::Rig::load(&data.0.join("rigs/humanoid.rig.toml")).unwrap();
        let mask = s.masks.get(&FULL_PX).unwrap();
        let fresh = bake_mask(&written, mask);
        let text = std::fs::read_to_string(data.0.join("rigs/humanoid.toml")).unwrap();
        assert_eq!(rig::BakedMeta::from_toml(&text).unwrap(), rig::BakedMeta::new(&written, &fresh));
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

    /// What the puppet builder reads from a rig beyond its joints and bones
    /// -- `[mirror]`, `[[ignore]]` -- survives a save and a reopen. A save
    /// from an app built before those tables existed dropped the canine's
    /// mirrored foreleg (its loader passes over what it does not know).
    #[test]
    fn a_save_keeps_the_mirror_and_ignore_tables() {
        let data = TempData::new("save_tables");
        let mut s = open(&data.0).unwrap();
        s.editor.place_current([0.0, 5.8]);
        s.editor.rig.mirror.insert("arm_r".into(), "arm_l".into());
        s.editor.rig.ignore.push(rig::IgnoreArea { view: View::Side, points: vec![[0.0, 4.0], [2.0, 4.0], [2.0, 5.0]] });
        s.save().unwrap();
        let mut again = open(&data.0).unwrap();
        assert_eq!(again.editor.rig().mirror.get("arm_r").map(String::as_str), Some("arm_l"));
        assert_eq!(again.editor.rig().ignore.len(), 1, "{:?}", again.editor.rig().ignore);
        // And through the family list, the way a family is opened in the app.
        again.request(DocAction::NewFromTemplate { template: "biped".into(), view: View::Front });
        again.request(DocAction::OpenFamily("humanoid".into()));
        assert_eq!(again.editor.rig().mirror.get("arm_r").map(String::as_str), Some("arm_l"));
        assert_eq!(again.editor.rig().ignore.len(), 1);
        again.save().unwrap();
        assert!(std::fs::read_to_string(data.0.join("rigs/humanoid.rig.toml")).unwrap().contains("[mirror]"));
    }

    /// Publish takes away what the family no longer has: a puppet its rig
    /// can no longer build, a texture whose sheet is gone. Left behind, the
    /// stage kept a puppet of a rig that no longer existed, under a report
    /// of success.
    #[test]
    fn publish_removes_what_the_family_no_longer_has() {
        let data = TempData::new("publish_stale");
        let out = data.0.join("published");
        let mut s = open(&data.0).unwrap();
        s.publish_dir = out.clone();
        s.publish_on_save = false;
        std::fs::create_dir_all(data.0.join("textures/humanoid")).unwrap();
        std::fs::write(data.0.join("textures/humanoid/sheet.png"), b"png").unwrap();
        s.editor.place_current([0.0, 5.8]);
        s.editor.place_current([0.0, 5.0]);
        s.save().unwrap();
        s.publish().unwrap();
        assert_eq!(names(&out.join("puppets")), ["humanoid.puppet.toml", "humanoid.texture.png"]);
        // The sheet goes, and the rig loses its puppet (as a save of a rig
        // that can no longer build one leaves it).
        std::fs::remove_file(data.0.join("textures/humanoid/sheet.png")).unwrap();
        std::fs::remove_file(data.0.join("rigs/humanoid.puppet.toml")).unwrap();
        s.publish().unwrap();
        assert!(names(&out.join("puppets")).is_empty(), "{:?}", names(&out.join("puppets")));
        assert!(s.status.contains("removed puppets/humanoid.puppet.toml, puppets/humanoid.texture.png"), "{}", s.status);
    }

    /// A hand-edited file that does not parse stops the whole publish before
    /// anything is copied: the good published pose set, and the rest of the
    /// last publish, stay as they were.
    #[test]
    fn publish_ships_nothing_while_a_pose_set_does_not_parse() {
        let data = TempData::new("publish_broken");
        let out = data.0.join("published");
        std::fs::create_dir_all(data.0.join("poses")).unwrap();
        std::fs::write(data.0.join("poses/biped.toml"), "body_type = \"biped\"\n").unwrap();
        let mut s = open(&data.0).unwrap();
        s.publish_dir = out.clone();
        s.publish_on_save = false;
        s.editor.place_current([0.0, 5.8]);
        s.save().unwrap();
        s.publish().unwrap();
        let good = std::fs::read(out.join("poses/biped.toml")).unwrap();
        let bake = std::fs::read(out.join("rigs/humanoid.toml")).unwrap();

        std::fs::write(data.0.join("poses/biped.toml"), "body_type = \"biped\"\n[pose.sit\n").unwrap();
        s.editor.place_current([0.0, 5.0]);
        s.save().unwrap();
        let err = s.publish().unwrap_err();
        assert!(err.contains("nothing published") && err.contains("poses/biped.toml"), "{err}");
        assert_eq!(std::fs::read(out.join("poses/biped.toml")).unwrap(), good, "the good pose set stays");
        assert_eq!(std::fs::read(out.join("rigs/humanoid.toml")).unwrap(), bake, "nothing else moved either");

        std::fs::write(data.0.join("poses/biped.toml"), "body_type = \"biped\"\n").unwrap();
        std::fs::create_dir_all(data.0.join("props")).unwrap();
        std::fs::write(data.0.join("props/weapons.toml"), "[weapon\n").unwrap();
        assert!(s.publish().unwrap_err().contains("props/weapons.toml"));
        std::fs::remove_file(data.0.join("props/weapons.toml")).unwrap();
        s.publish().unwrap();
        assert_ne!(std::fs::read(out.join("rigs/humanoid.toml")).unwrap(), bake, "fixed, it publishes");
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
        assert_eq!(names(&out), ["outfits", "poses", "puppets", "rigs"]);
        // no outfits in the data root: the shipped kit and nobody dressed,
        // as the editor has it (not the embedded assignments left standing)
        assert_eq!(names(&out.join("outfits")), ["kit.toml", "outfits.toml"]);
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

    /// A creature painted in its outfit: Save installs its skin and the
    /// outfit files beside the puppet, Publish ships them, and the stage
    /// dresses the creature and wears the skin -- the studio's sheet fits
    /// the stage's dressed figure, rebuilt from the bake.
    #[test]
    fn a_creature_skin_reaches_the_stage_on_its_dressed_puppet() {
        let _guard = gs_field::data_dirs::VELLUM_FE_DIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let data = TempData::new("skins");
        std::fs::copy(repo.join("outlines.json"), data.0.join("outlines.json")).unwrap();
        std::fs::create_dir_all(data.0.join("rigs")).unwrap();
        std::fs::copy(repo.join("rigs/humanoid.rig.toml"), data.0.join("rigs/humanoid.rig.toml")).unwrap();
        std::fs::create_dir_all(data.0.join("outfits")).unwrap();
        std::fs::write(data.0.join("outfits/outfits.toml"), "[squamous_reptilian_mutant]\nwear = [\"robe\", \"hood\"]\n").unwrap();
        let (pool, out) = (data.0.join("pool"), data.0.join("published"));
        let mut s = Session::open(&data.0, out.clone(), Some(pool.clone())).unwrap();

        // Paint the front flat red over its outfit's template, and build.
        let creature = "squamous_reptilian_mutant";
        let puppet = s.puppet().clone().unwrap();
        let d = super::super::texture::Dressed::creature(&data.0, &puppet, &s.outlines, "humanoid", creature).unwrap();
        let grid = d.front.as_ref().unwrap().grid;
        let folder = super::super::texture::creature_dir(&data.0, "humanoid", creature);
        std::fs::create_dir_all(&folder).unwrap();
        let red: Vec<u8> = [220, 30, 30, 255].repeat(grid.w * grid.h);
        super::super::texture::write_png(&folder.join("front.png"), grid.w, grid.h, &red).unwrap();
        let (built, failed) = super::super::texture::build_every_skin(&data.0, "humanoid", &puppet, &s.outlines).unwrap();
        assert_eq!((built.as_slice(), failed.len()), ([creature.to_string()].as_slice(), 0), "{failed:?}");

        s.save().unwrap();
        let skin = format!("skins/humanoid/{creature}.png");
        assert!(pool.join("global/puppets").join(&skin).is_file(), "{}", s.status);
        assert!(pool.join("global/outfits/outfits.toml").is_file());
        assert!(out.join("puppets").join(&skin).is_file() && out.join("outfits/outfits.toml").is_file(), "{}", s.status);

        // What the stage does with it (gs_field_egui's PuppetCache): the
        // installed bake, dressed from the installed outfits, lays out a
        // sheet exactly the size of the installed skin.
        gs_field::data_dirs::set_base_dir(pool.clone());
        crate::pool_dir::invalidate_caches();
        let set = gs_field::puppets::puppets();
        assert!(set.puppet_for("squamous reptilian mutant", Some("mutant")).is_some_and(|d| d.family == "humanoid"));
        let kit = gs_puppet::Kit::from_toml(set.kit()).unwrap();
        let outfit = gs_puppet::Outfits::from_toml(set.outfits(), &kit).unwrap().get(creature).cloned().unwrap();
        let stage = Puppet::from_baked_toml(&set.get("humanoid").unwrap().baked).unwrap().wearing(&outfit);
        let layout = gs_puppet::UvLayout::fit(&stage, gs_puppet::SHEET_SIDE);
        let installed = std::fs::read(pool.join("global/puppets").join(&skin)).unwrap();
        assert_eq!(set.skin("humanoid", creature).map(|b| b.to_vec()), Some(installed));
        let (w, h, _) = super::super::texture::read_png(&pool.join("global/puppets").join(&skin)).unwrap();
        assert_eq!((w, h), (layout.w, layout.h), "the studio's sheet fits the stage's dressed figure");
        gs_field::data_dirs::clear_base_dir();
        crate::pool_dir::invalidate_caches();

        // No sheet any more: the next Save takes the skin off the stage.
        std::fs::remove_file(folder.join("sheet.png")).unwrap();
        s.save().unwrap();
        assert!(!pool.join("global/puppets").join(&skin).exists());
        assert!(!out.join("puppets").join(&skin).exists());
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
