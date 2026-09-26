//! The rig editor's egui layer. It draws `editor::Session` and forwards
//! input to it; every rule lives in `crate::editor`.

use std::path::{Path, PathBuf};

use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};
use rig::{templates, View, AXES, PARTS};

use crate::editor::depth::{axis_label, other_view};
use crate::editor::palette::{label_rgba, LABEL_RGB};
use crate::editor::pose_edit::{self, Origin, PairState, PoseDoc, Row, Scope, FREE, ROOT};
use crate::editor::prompt::{NamePrompt, PromptKind};
use crate::editor::puppet_view::{PoseSource, PuppetView};
use crate::editor::state::Editor;
use crate::editor::structure::{AnchorSlot, MAX_NAME_LEN};
use crate::editor::warnings::MIN_SHARE;
use crate::editor::{Canvas, DepthModel, DocAction, EditView, Session, ViewTransform};

const PREVIEW_ALPHA: u8 = 215;
const FIT_MARGIN_PX: f32 = 36.0;
/// The joint and bone lists split the left panel's spare height; neither
/// may be squeezed below this.
const MIN_LIST_HEIGHT: f32 = 110.0;
/// Bones get this share of the split by default (the divider is draggable).
const BONES_SHARE: f32 = 0.5;

pub struct RigTool {
    session: Session,
    view: ViewTransform,
    /// DEPTH mode's own pan / zoom: it looks at the OTHER silhouette, which
    /// is a different shape, so it keeps its own camera and Fit.
    depth_view: ViewTransform,
    /// DEPTH mode has not been framed on the other silhouette yet.
    depth_fit: bool,
    texture: Option<(u64, egui::TextureHandle)>,
    /// A radius drag-value is being dragged: bake small, like a joint drag.
    radius_drag: bool,
    /// Likewise the eye-radius drag-value.
    eye_drag: bool,
    /// A depth DragValue in the joint list is being dragged.
    depth_value_drag: bool,
    /// A bone's depth / tip thickness box is being dragged.
    thickness_drag: bool,
    /// The open Add / Rename prompt, and whether its text box still needs focus.
    prompt: Option<(NamePrompt, bool)>,
    /// "Show on stage" was pressed for this family; the app takes it.
    show_on_stage: Option<String>,
    /// The live 3D puppet panel.
    puppet_view: PuppetView,
    /// Its last smooth skin, and what it was rendered from.
    skin: Option<(SkinKey, egui::TextureHandle, Vec2)>,
    /// How many splats the cached cloud holds (0 when the skin is drawn).
    splat_count: usize,
    /// The painted sheet, and the texture it makes for the current rig.
    sheet: Option<SheetFile>,
    sheet_serial: u64,
    sheet_texture: Option<(u64, u64, Result<Option<std::sync::Arc<gs_puppet::Texture>>, String>)>,
    /// The family's fitted cloud (`<data>/clouds/<family>.ply`), read once
    /// per family and file time; None inside = the family has none.
    cloud: Option<CloudFile>,
    /// That cloud bound to the puppet of a rig revision.
    bound: Option<(u64, u64, std::sync::Arc<gs_puppet::splat::BoundCloud>)>,
    /// The creatures the preview can show on this family, and for which
    /// family and sheet serial they were listed.
    creatures: Option<(String, u64, Vec<String>)>,
    /// The shown creature's figure (the puppet in its outfit).
    dressed: Option<Figure>,
    /// A running "Export jobs" (every family's templates and jobs.json).
    jobs_export: Option<std::sync::mpsc::Receiver<Result<crate::editor::texture::JobsExport, String>>>,
    /// Publish target / saved bake / saved puppet, re-checked at most every
    /// couple of seconds and after a save or publish -- never per frame.
    /// Asked on a worker thread: a stalled `stat` must not freeze a frame.
    files: crate::fs_cache::Background<(String, PathBuf, PathBuf, bool), FileFacts>,
    /// The families with a saved rig, for the family list (same reason).
    saved_rigs: crate::fs_cache::Background<PathBuf, std::collections::BTreeSet<String>>,
    /// The pose editor: on, the body type's pose file open for editing,
    /// which section it writes, the row (bone) being edited.
    pose_edit: bool,
    pose_doc: Option<PoseDoc>,
    pose_scope: Scope,
    pose_row: Option<String>,
    new_pose_name: String,
    /// Closing with unsaved poses: asking, or answered.
    pose_quit: PoseQuit,
    /// "Play death" was pressed at this moment: the preview falls from the
    /// shown pose into the dead one (`gs_puppet::Fall`).
    death: Option<std::time::Instant>,
    /// Counts the presses: each death takes the next variety (which dead
    /// pose, the fall's timing), as different creatures do on the stage.
    death_seed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PoseQuit {
    No,
    Asking,
    Approved,
}

/// Everything a preview skin depends on; a new one is rendered only when
/// this changes, not on every repaint.
#[derive(Debug, Clone, PartialEq)]
struct SkinKey {
    revision: u64,
    pose: String,
    view: [u32; 5],
    /// The texture worn (its load serial), if any.
    texture: Option<u64>,
    /// The pose file's edit revision (the pose changes under one name).
    poses: u64,
    /// What each hand holds.
    hold: [String; 2],
    /// The creature shown ("" = the family bare).
    creature: String,
    /// Rendered at half size while something moves (see `DRAFT_SCALE`).
    draft: bool,
    /// As a splat cloud, with its shape (spacing, overlap, opacity, quantized).
    splats: Option<[u32; 3]>,
    /// The fitted cloud file's stamp when one is drawn instead.
    cloud: Option<u64>,
}

/// While the preview is being turned, zoomed or its rig dragged, the skin is
/// rendered at this scale and stretched: a quarter of the pixels, so the drag
/// keeps up. Letting go renders it sharp.
const DRAFT_SCALE: f32 = 0.5;

/// The family's `sheet.png`, as read from disk (re-read on `serial`).
/// A creature's figure in the preview: the family's puppet in its outfit,
/// and the rig revision, creature and sheet serial it was built for.
struct Figure {
    revision: u64,
    creature: String,
    serial: u64,
    puppet: Result<std::sync::Arc<gs_puppet::Puppet>, String>,
}

struct CloudFile {
    family: String,
    /// The file's modification time, as a serial: re-read when it changes.
    stamp: u64,
    gaussians: Result<Option<std::sync::Arc<Vec<gs_puppet::splat::Gaussian>>>, String>,
}

struct SheetFile {
    /// `<family>` or `<family>/<creature>`: whose sheet it is.
    who: String,
    serial: u64,
    pixels: Result<Option<(usize, usize, std::sync::Arc<Vec<u8>>)>, String>,
}

/// What the document controls need to know about files on disk.
struct FileFacts {
    target: Result<PathBuf, String>,
    saved: bool,
    puppet_saved: bool,
}

fn label_color(label: usize) -> Color32 {
    let [r, g, b] = LABEL_RGB[label.min(PARTS.len())];
    Color32::from_rgb(r, g, b)
}

fn pos(p: [f32; 2]) -> Pos2 {
    Pos2::new(p[0], p[1])
}

impl crate::quit::QuitGuard for RigTool {
    fn approve_quit(&mut self) -> bool {
        RigTool::approve_quit(self)
    }
    fn quit_asking(&self) -> bool {
        RigTool::quit_asking(self)
    }
    fn quit_approved(&self) -> bool {
        RigTool::quit_approved(self)
    }
    fn cancel_quit(&mut self) {
        RigTool::cancel_quit(self)
    }
}

impl RigTool {
    pub fn open(data_root: &Path, publish_dir: PathBuf, pool: Option<PathBuf>) -> Result<RigTool, String> {
        Ok(RigTool {
            session: Session::open(data_root, publish_dir, pool)?,
            view: ViewTransform::default(),
            depth_view: ViewTransform::default(),
            depth_fit: true,
            texture: None,
            radius_drag: false,
            eye_drag: false,
            depth_value_drag: false,
            thickness_drag: false,
            prompt: None,
            show_on_stage: None,
            puppet_view: PuppetView::default(),
            skin: None,
            splat_count: 0,
            sheet: None,
            sheet_serial: 0,
            sheet_texture: None,
            cloud: None,
            bound: None,
            creatures: None,
            dressed: None,
            jobs_export: None,
            files: Default::default(),
            saved_rigs: Default::default(),
            pose_edit: false,
            pose_doc: None,
            pose_scope: Scope::Shared,
            pose_row: None,
            new_pose_name: String::new(),
            pose_quit: PoseQuit::No,
            death: None,
            death_seed: 0,
        })
    }

    /// Where Publish ships from now on.
    pub fn set_publish_dir(&mut self, dir: PathBuf) {
        self.files.invalidate();
        self.saved_rigs.invalidate();
        self.session.publish_dir = dir;
    }

    /// The pool Save installs bakes and puppets into.
    pub fn set_pool(&mut self, pool: PathBuf) {
        self.session.pool = Some(pool);
        match self.session.sync_pool_poses() {
            Some(Ok(n)) => self.session.status = format!("{n} pose sets refreshed in the pool"),
            Some(Err(e)) => self.session.status = e,
            None => {}
        }
    }

    /// Open the 3D preview with the pose editor on `[<family>/]<pose>[:<row>]`
    /// (`GS_STUDIO_POSE_EDIT`, for visual checks); the row's bone is
    /// selected in the bone list too.
    pub fn open_pose_editor(&mut self, spec: &str) {
        let (family, spec) = spec.split_once('/').unwrap_or(("", spec));
        if !family.is_empty() {
            self.session.request(DocAction::OpenFamily(family.to_owned()));
        }
        let (pose, row) = spec.split_once(':').unwrap_or((spec, ""));
        self.puppet_view.show = true;
        if std::env::var_os("GS_STUDIO_SPLATS").is_some() {
            self.puppet_view.splats = true;
        }
        self.pose_edit = true;
        self.puppet_view.pose = if pose.is_empty() { "stand".into() } else { pose.to_owned() };
        self.pose_row = (!row.is_empty()).then(|| row.to_owned());
        let bones = &self.session.editor.rig().bones;
        let bone = [row.to_owned(), format!("{row}_l")].iter().find_map(|name| bones.iter().position(|b| &b.name == name));
        self.session.editor.select_bone(bone);
    }

    /// The family whose puppet "Show on stage" asked for, once.
    pub fn take_show_on_stage(&mut self) -> Option<String> {
        self.show_on_stage.take()
    }

    pub fn is_dirty(&self) -> bool {
        self.session.editor.is_dirty() || self.poses_dirty()
    }

    fn poses_dirty(&self) -> bool {
        self.pose_doc.as_ref().is_some_and(PoseDoc::is_dirty)
    }

    /// The window wants to close. True = go ahead; false = a discard prompt
    /// is now showing (unsaved poses are asked about first).
    pub fn approve_quit(&mut self) -> bool {
        if self.poses_dirty() && self.pose_quit != PoseQuit::Approved {
            self.pose_quit = PoseQuit::Asking;
            return false;
        }
        if !self.session.quit_approved {
            self.session.request(DocAction::Quit);
        }
        self.session.quit_approved
    }

    pub fn quit_approved(&self) -> bool {
        self.session.quit_approved && (!self.poses_dirty() || self.pose_quit == PoseQuit::Approved)
    }

    /// A quit question (the poses', or the rig's) is up.
    fn quit_asking(&self) -> bool {
        self.pose_quit == PoseQuit::Asking || self.session.pending == Some(DocAction::Quit)
    }

    /// The close is off: drop any agreement, so the next one asks again.
    fn cancel_quit(&mut self) {
        self.session.quit_approved = false;
        if self.session.pending == Some(DocAction::Quit) {
            self.session.cancel_pending();
        }
        self.pose_quit = PoseQuit::No;
    }

    pub fn status(&self, ui: &mut egui::Ui) {
        let s = &self.session;
        let rig = s.editor.rig();
        if s.editor.is_dirty() {
            ui.colored_label(ui.visuals().warn_fg_color, "* unsaved");
        } else {
            ui.weak("saved");
        }
        ui.separator();
        ui.label(format!("{} / {}{} / {}", rig.family, rig.view.as_str(), if rig.from_above { " (from above)" } else { "" }, rig.body_type));
        ui.separator();
        let mode = s.editor.edit_view();
        ui.label(mode.label());
        if mode == EditView::Depth {
            ui.weak(format!("{} of {} overridden", s.editor.depth_override_count(), rig.joints.len()));
        }
        if let Some(b) = s.last_bake() {
            ui.separator();
            ui.weak(format!("bake {} px: {:.1} ms", b.long_side_px, b.ms));
        }
        ui.separator();
        ui.label(&s.status);
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.shortcuts(ui);
        // Bake before drawing so every panel shows the same, current map.
        let interactive = self.session.editor.is_dragging()
            || self.session.editor.is_depth_dragging()
            || self.radius_drag
            || self.eye_drag
            || self.depth_value_drag
            || self.thickness_drag;
        {
            let _t = crate::perf_log::span("rigs: bake");
            self.session.baked(interactive);
        }
        // The depth guesses move with the joints, so re-read them the same way.
        {
            let _t = crate::perf_log::span("rigs: depth guess");
            self.session.refresh_depth();
        }
        // A family with no second silhouette cannot be edited in DEPTH.
        if !self.session.depth_available() {
            self.session.editor.set_edit_view(EditView::Primary);
        }

        egui::Panel::left("rig_left").default_size(330.0).min_size(260.0).show(ui, |ui| {
            let _t = crate::perf_log::span("rigs: left panel");
            self.left_panel(ui)
        });
        egui::Panel::right("rig_right").default_size(270.0).min_size(200.0).show(ui, |ui| {
            let _t = crate::perf_log::span("rigs: right panel");
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.right_panel(ui));
        });
        if self.puppet_view.show {
            egui::Panel::right("rig_puppet")
                .resizable(true)
                .default_size(380.0)
                .min_size(220.0)
                .show(ui, |ui| {
                    let _t = crate::perf_log::span("rigs: 3D panel");
                    self.puppet_panel(ui)
                });
        }
        egui::CentralPanel::default().show(ui, |ui| {
            let _t = crate::perf_log::span("rigs: canvas");
            self.centre(ui)
        });
        self.name_prompt(ui.ctx());
    }

    /// The questions that guard unsaved work (the rig, the poses), drawn by
    /// the app over whichever tab is showing: a window close asked from
    /// another tab waits on them, and drawn only here they left the window
    /// seemingly refusing to close.
    pub fn modal_prompts(&mut self, ctx: &egui::Context) {
        self.discard_prompt(ctx);
        self.pose_quit_prompt(ctx);
    }

    fn ask(&mut self, kind: PromptKind) {
        self.prompt = Some((NamePrompt::new(kind, &self.session.editor), true));
    }

    fn shortcuts(&mut self, ui: &mut egui::Ui) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        if self.session.pending.is_some() || self.prompt.is_some() {
            return;
        }
        let pressed = |ui: &mut egui::Ui, mods: Modifiers, key: Key| {
            ui.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(mods, key)))
        };
        // While a pose is being edited, undo is the pose's.
        let poses = self.pose_doc.as_mut().filter(|_| self.pose_edit && self.puppet_view.show);
        if pressed(ui, Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) || pressed(ui, Modifiers::COMMAND, Key::Y) {
            match poses {
                Some(doc) => doc.redo(),
                None => {
                    self.session.editor.redo();
                }
            }
        } else if pressed(ui, Modifiers::COMMAND, Key::Z) {
            match poses {
                Some(doc) => doc.undo(),
                None => {
                    self.session.editor.undo();
                }
            }
        }
        if pressed(ui, Modifiers::COMMAND, Key::S) {
            self.save();
        }
        let typing = ui.ctx().egui_wants_keyboard_input();
        let depth_mode = self.session.editor.edit_view() == EditView::Depth;
        if !typing && ui.input(|i| i.key_pressed(Key::Delete)) {
            // In DEPTH, Delete resets the joint to its guess: un-placing is a
            // primary-view act and must not happen behind the artist's back.
            match (depth_mode, self.session.editor.selected().map(str::to_owned)) {
                (true, Some(joint)) => {
                    self.session.editor.reset_joint_depth(&joint);
                }
                (true, None) => {}
                (false, _) => {
                    self.session.editor.unplace_selected();
                }
            }
        }
        if !typing && ui.input(|i| i.key_pressed(Key::Escape)) {
            self.session.editor.set_chain_pick(None);
        }
    }

    fn save(&mut self) {
        self.files.invalidate();
        self.saved_rigs.invalidate();
        if let Err(e) = self.session.save() {
            self.session.status = format!("Save failed: {e}");
        }
    }

    // ---------------------------------------------------------------- left

    /// Document controls on top at their natural height; below them the
    /// joint list and the bone list share ALL the remaining height, each in
    /// its own scroll area, with a draggable divider between them. Nothing
    /// is clipped: a list that does not fit scrolls.
    fn left_panel(&mut self, ui: &mut egui::Ui) {
        {
            let _t = crate::perf_log::span("rigs: document controls");
            self.document_controls(ui);
        }
        ui.separator();
        let spare = ui.available_height();
        let most = (spare - MIN_LIST_HEIGHT).max(MIN_LIST_HEIGHT);
        egui::Panel::bottom("rig_bones_split")
            .resizable(true)
            .frame(egui::Frame::NONE.inner_margin(egui::Margin { top: 6, ..Default::default() }))
            .default_size((spare * BONES_SHARE).clamp(MIN_LIST_HEIGHT, most))
            .size_range(MIN_LIST_HEIGHT..=most)
            .show(ui, |ui| {
                let _t = crate::perf_log::span("rigs: bone list");
                self.bone_list(ui)
            });
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            let _t = crate::perf_log::span("rigs: joint list");
            self.joint_list(ui)
        });
    }

    fn document_controls(&mut self, ui: &mut egui::Ui) {
        let (target, saved, puppet_saved) = {
            let session = &self.session;
            let family = session.editor.rig().family.clone();
            let (publish, rigs, pooled) = (session.publish_dir.clone(), session.rigs_dir(), session.pool.is_some());
            let key = (family.clone(), publish.clone(), rigs.clone(), pooled);
            let facts = self.files.get(key, move || FileFacts {
                target: crate::editor::session::publish_target_of(&publish),
                saved: crate::editor::session::has_saved_bake_in(&rigs, &family),
                puppet_saved: pooled && rigs.join(gs_puppet::Puppet::baked_file_name(&family)).is_file(),
            });
            let known = match facts {
                Some(f) => (f.target.clone(), f.saved, f.puppet_saved),
                // Not known yet (a moment after opening a family).
                None => (Err("Checking the publish directory...".to_owned()), false, false),
            };
            if self.files.pending() || self.saved_rigs.pending() {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            }
            known
        };
        let rigs_dir = self.session.rigs_dir();
        let saved_rigs = self.saved_rigs.get(rigs_dir.clone(), move || crate::editor::session::saved_rigs_in(&rigs_dir)).cloned().unwrap_or_default();
        // A save or publish below changes those answers: re-check after it.
        let mut wrote = false;
        let s = &mut self.session;
        let (family, view, body_type, axis, ref_ft) = {
            let r = s.editor.rig();
            (r.family.clone(), r.view, r.body_type.clone(), r.axis.clone(), r.ref_ft)
        };

        egui::Grid::new("rig_doc").num_columns(2).show(ui, |ui| {
            ui.label("Family");
            let mut open = None;
            egui::ComboBox::from_id_salt("family").selected_text(&family).width(170.0).show_ui(ui, |ui| {
                let _t = crate::perf_log::span("rigs: family list");
                for f in s.outlines.families() {
                    let text = if saved_rigs.contains(&f) { format!("{f}  (saved)") } else { f.clone() };
                    if ui.selectable_label(f == family, text).clicked() && f != family {
                        open = Some(f);
                    }
                }
            });
            if let Some(f) = open {
                s.request(DocAction::OpenFamily(f));
            }
            ui.end_row();

            ui.label("View");
            ui.horizontal(|ui| {
                for v in [View::Front, View::Side] {
                    let exists = s.outlines.get(&family, v).is_some();
                    let button = ui
                        .add_enabled(exists, egui::Button::selectable(v == view, v.as_str()))
                        .on_disabled_hover_text("No silhouette for this view")
                        .on_hover_text("Switching view starts a new rig: joints belong to one silhouette");
                    if button.clicked() && v != view {
                        s.request(DocAction::NewFromTemplate { template: body_type.clone(), view: v });
                    }
                }
                let mut above = s.editor.rig().from_above;
                if ui
                    .add_enabled(view == View::Front, egui::Checkbox::new(&mut above, "from above"))
                    .on_hover_text(
                        "The silhouette is a drawing from ABOVE (a spider, a crab): up in it is the                          creature's front, and the coordinate it cannot show is HEIGHT, read from the side                          silhouette. The 3D puppet lies flat; the bake is unchanged. Set by the multileg template.",
                    )
                    .changed()
                {
                    s.editor.set_from_above(above);
                }
            });
            ui.end_row();

            ui.label("Editing").on_hover_text(
                "PRIMARY: the rig's own silhouette -- place and move joints.\n\
                 DEPTH: the family's OTHER silhouette, each joint at its off-plane coordinate.",
            );
            ui.horizontal(|ui| {
                let now = s.editor.edit_view();
                let available = s.depth_available();
                let why = s.depth().unavailable_reason(s.editor.rig());
                for choice in EditView::ALL {
                    let enabled = available || choice == EditView::Primary;
                    let button = ui
                        .add_enabled(enabled, egui::Button::selectable(choice == now, choice.label()))
                        .on_disabled_hover_text(&why);
                    let button = match choice {
                        EditView::Primary => button.on_hover_text("Place and move joints on this rig's silhouette"),
                        EditView::Depth => button.on_hover_text(format!(
                            "Correct each joint's {}, against the {} silhouette. Dragging changes depth only.",
                            crate::editor::depth::axis_label_of(s.editor.rig()),
                            other_view(s.editor.rig().view).as_str(),
                        )),
                    };
                    if button.clicked() {
                        s.editor.set_edit_view(choice);
                    }
                }
            });
            ui.end_row();

            ui.label("Template");
            let mut pick = None;
            egui::ComboBox::from_id_salt("template").selected_text(&body_type).width(170.0).show_ui(ui, |ui| {
                for t in templates() {
                    let item = ui.selectable_label(t.name == body_type, t.name).on_hover_text(t.blurb);
                    if item.clicked() && t.name != body_type {
                        pick = Some(t.name.to_owned());
                    }
                }
            });
            if let Some(template) = pick {
                s.request(DocAction::NewFromTemplate { template, view });
            }
            ui.end_row();

            ui.label("Reference");
            ui.horizontal(|ui| {
                ui.label(format!("{ref_ft} ft at"));
                egui::ComboBox::from_id_salt("axis").selected_text(&axis).width(90.0).show_ui(ui, |ui| {
                    for a in AXES {
                        if ui.selectable_label(a == axis, a).clicked() {
                            s.editor.set_axis(a);
                        }
                    }
                });
            });
            ui.end_row();
        });

        ui.horizontal(|ui| {
            let label = if s.editor.is_dirty() { "Save *" } else { "Save" };
            if ui.button(label).on_hover_text("Write the rig and its baked map + metadata (Ctrl+S)").clicked() {
                wrote = true;
                if let Err(e) = s.save() {
                    s.status = format!("Save failed: {e}");
                }
            }
            if ui.button("New from template").on_hover_text("Un-place everything and reset the bones").clicked() {
                s.request(DocAction::NewFromTemplate { template: body_type.clone(), view });
            }
        });
        ui.horizontal(|ui| {
            let target = target.clone();
            let button = ui.add_enabled(target.is_ok() && saved, egui::Button::new("Publish"));
            let button = match &target {
                Ok(dir) if saved => button.on_hover_text(format!(
                    "Copy the SAVED bake ({family}.png + .toml), its puppet and the pose sets into {} \
                     (rigs/, puppets/, poses/) for game clients to embed.\n\
                     Unsaved edits are not included.",
                    dir.display()
                )),
                Ok(_) => button.on_disabled_hover_text("Nothing to publish yet: save this rig first"),
                Err(why) => button.on_disabled_hover_text(why),
            };
            if button.clicked() {
                wrote = true;
                if let Err(e) = s.publish() {
                    s.status = format!("Publish failed: {e}");
                }
            }
            ui.add_enabled(target.is_ok(), egui::Checkbox::new(&mut s.publish_on_save, "publish on save"))
                .on_hover_text("Save also publishes")
                .on_disabled_hover_text(target.err().unwrap_or_default());
        });
        let mut show = false;
        ui.horizontal(|ui| {
            if ui.add_enabled(s.editor.can_undo(), egui::Button::new("Undo")).clicked() {
                s.editor.undo();
            }
            if ui.add_enabled(s.editor.can_redo(), egui::Button::new("Redo")).clicked() {
                s.editor.redo();
            }
            ui.separator();
            ui.toggle_value(&mut self.puppet_view.show, "3D preview")
                .on_hover_text("Show the rig's 3D puppet beside the canvas, rebuilt live as you edit");
            show = ui
                .add_enabled(puppet_saved, egui::Button::new("Show on stage"))
                .on_hover_text("Open Preview with this family's puppet spawned (the SAVED puppet, from the pool)")
                .on_disabled_hover_text("Save first: saving builds the puppet and puts it in the pool")
                .clicked();
        });
        if show {
            self.show_on_stage = Some(family);
        }
        if wrote {
            self.files.invalidate();
            self.saved_rigs.invalidate();
        }
    }

    // --------------------------------------------------------- 3D preview

    /// The live puppet: built from the rig as it stands (unsaved edits
    /// included), posed from the body type's pose set, orbited by dragging.
    fn puppet_panel(&mut self, ui: &mut egui::Ui) {
        let (family, body_type) = {
            let rig = self.session.editor.rig();
            (rig.family.clone(), rig.body_type.clone())
        };
        let data_root = self.session.data_root.clone();
        if self.creatures.as_ref().is_none_or(|(f, s, _)| *f != family || *s != self.sheet_serial) {
            let list = crate::editor::texture::family_creatures(&data_root, &family);
            self.creatures = Some((family.clone(), self.sheet_serial, list));
        }
        let creatures = self.creatures.as_ref().map(|(_, _, c)| c.clone()).unwrap_or_default();
        if !self.puppet_view.creature.is_empty() && !creatures.contains(&self.puppet_view.creature) {
            self.puppet_view.creature.clear();
        }
        // The pose file open in the editor, when it is this body type's:
        // the preview shows its (unsaved) poses.
        let editing = self.pose_doc.as_mut().filter(|d| d.body_type() == body_type);
        let doc_names = editing.map(|d| d.pose_names(&family));
        let pv = &mut self.puppet_view;
        let mut reload_stage = false;
        let (mut export, mut build, mut reload_sheet) = (false, false, false);
        let (mut build_all, mut project, mut export_jobs) = (false, None, false);

        let (names, source, error) = match pv.poses(&data_root, &body_type, &family) {
            Ok((set, source)) => (set.names().into_iter().map(str::to_owned).collect::<Vec<_>>(), Some(*source), None),
            Err(e) => (Vec::new(), None, Some(e.clone())),
        };
        let names = doc_names.unwrap_or(names);
        let file = match source {
            Some(PoseSource::DataRoot) => format!("data/poses/{body_type}.toml"),
            _ => format!("the built-in {body_type} poses"),
        };
        // The pose: which one, edit it, kill it, re-read the files.
        ui.horizontal_wrapped(|ui| {
            ui.strong("Pose");
            let shown = if pv.pose.is_empty() { "rest (as clicked)".to_owned() } else { pv.pose.clone() };
            egui::ComboBox::from_id_salt("puppet_pose")
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut pv.pose, String::new(), "rest (as clicked)");
                    for name in &names {
                        ui.selectable_value(&mut pv.pose, name.clone(), name);
                    }
                })
                .response
                .on_hover_text(format!("Poses from {file}"));
            if ui
                .toggle_value(&mut self.pose_edit, "Edit pose")
                .on_hover_text("Change the shown pose's angles and joint limits: pick a bone (click it, or its name), then its sliders")
                .clicked()
                && self.pose_edit
                && pv.pose.is_empty()
            {
                pv.pose = "stand".into();
            }
            if ui.button("Play death").on_hover_text("Fall down dead from this pose, as the stage does when a creature dies").clicked() {
                self.death = Some(std::time::Instant::now());
                self.death_seed += 1;
            }
            if ui.button("Reload poses").on_hover_text(format!("Read {file} again after editing it by hand, here AND on the stage")).clicked() {
                pv.reload_poses();
                reload_stage = true;
            }
        });
        if let Some(e) = error {
            ui.colored_label(ui.visuals().warn_fg_color, e);
        }
        // How it is drawn.
        ui.horizontal_wrapped(|ui| {
            ui.strong("View");
            ui.checkbox(&mut pv.skin, "Smooth skin")
                .on_hover_text("One continuous surface with a depth test per pixel. Off: the flat shapes the game drew before");
            ui.add_enabled(pv.skin, egui::Checkbox::new(&mut pv.texture, "Texture"))
                .on_hover_text("Dress the skin in data/textures/<family>/sheet.png (a creature's own: <family>/<creature>/sheet.png)");
            ui.add_enabled(pv.skin, egui::Checkbox::new(&mut pv.splats, "Splats"))
                .on_hover_text("Draw the puppet as a Gaussian splat cloud instead of the smooth skin: one soft splat per ring sample, coloured from the sheet, sorted and blended. The look test for a splat renderer");
            ui.label("Creature");
            let shown = if pv.creature.is_empty() { format!("({family})") } else { pv.creature.clone() };
            egui::ComboBox::from_id_salt("preview_creature").selected_text(shown).height(400.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut pv.creature, String::new(), format!("({family})"));
                for c in &creatures {
                    ui.selectable_value(&mut pv.creature, c.clone(), c);
                }
            })
            .response
            .on_hover_text("Show a creature: this puppet in its outfit (data/outfits/outfits.toml), in its own sheet -- as the stage will. Build sheet and projection follow it.");
            if ui.button("Reset view").on_hover_text("Face front, default zoom").clicked() {
                pv.reset_camera();
            }
        });
        if pv.skin && pv.splats {
            let p = &mut pv.splat;
            ui.horizontal_wrapped(|ui| {
                ui.strong("Splats");
                ui.add(egui::Slider::new(&mut p.spacing, 0.01..=0.12).logarithmic(true).text("spacing ft"))
                    .on_hover_text("Sample step along each ring and between rings: finer = more, smaller splats");
                ui.add(egui::Slider::new(&mut p.overlap, 0.3..=1.5).text("overlap")).on_hover_text("How far each splat reaches past its step: 0.5 = neighbours just touch");
                ui.add(egui::Slider::new(&mut p.alpha, 0.1..=1.0).text("opacity")).on_hover_text("Each splat's peak opacity");
                if ui.small_button("Defaults").clicked() {
                    *p = crate::editor::puppet_view::PuppetView::default().splat;
                }
            });
        }
        // What it holds: any weapon or shield the props file knows.
        let weapons: Vec<(String, String)> = match pv.props(&data_root) {
            Ok((set, _)) => {
                let mut w: Vec<(String, String)> = set.weapons.values().map(|w| (w.key.clone(), w.base.clone())).collect();
                w.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
                w
            }
            Err(e) => {
                ui.colored_label(ui.visuals().warn_fg_color, e.clone());
                Vec::new()
            }
        };
        ui.horizontal_wrapped(|ui| {
            ui.strong("Hold").on_hover_text(
                "Preview a weapon or shield in its hands (data/props/weapons.toml). On the stage a creature holds \
                 the first weapon its bestiary entry attacks with.",
            );
            for (id, label, pick) in [("hold_right", "right", &mut pv.hold), ("hold_left", "left", &mut pv.hold_left)] {
                let shown = weapons.iter().find(|(k, _)| k == pick).map_or("(nothing)".to_owned(), |(_, b)| b.clone());
                ui.label(label);
                egui::ComboBox::from_id_salt(id).selected_text(shown).height(400.0).show_ui(ui, |ui| {
                    ui.selectable_value(pick, String::new(), "(nothing)");
                    for (key, base) in &weapons {
                        ui.selectable_value(pick, key.clone(), base);
                    }
                });
            }
        });
        egui::CollapsingHeader::new("Texture painting").id_salt("rig_texture_tools").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                export = ui
                    .button("Export templates")
                    .on_hover_text("Write the front / side / back paint-over silhouettes and the pattern sheet to data/textures/<family>/")
                    .clicked();
                build = ui
                    .button("Build sheet from paintings")
                    .on_hover_text("Project front.png / side.png / back.png from that folder (the creature's, when one is shown) onto the pattern: writes sheet.png")
                    .clicked();
                reload_sheet = ui.button("Reload").on_hover_text("Read sheet.png again after painting on it").clicked();
                build_all = ui
                    .button("Build every skin")
                    .on_hover_text("Build sheet.png in every creature folder of this family (data/textures/<family>/<creature>/), each in its outfit")
                    .clicked();
            });
            ui.horizontal_wrapped(|ui| {
                use crate::editor::texture::view_name;
                // The family's painting order (generator.toml's
                // projection_order): each view after the first is rendered
                // from the paintings before it.
                let order = crate::editor::texture::projection_order(&data_root, &family);
                if let Err(e) = &order {
                    ui.colored_label(ui.visuals().warn_fg_color, e);
                }
                let order = order.unwrap_or(crate::editor::texture::DEFAULT_ORDER);
                let [a, b, c] = order.map(view_name);
                ui.label("Projection-first:");
                if ui.button(format!("{a} -> {b}")).on_hover_text(format!("Render what {a}.png shows from the {b}, and what is left to paint: projected/{b}_*.png")).clicked() {
                    project = Some((order[1], order));
                }
                if ui
                    .button(format!("{a} + {b} -> {c}"))
                    .on_hover_text(format!("The same from the {c}, from {a}.png and {b}.png: projected/{c}_*.png"))
                    .clicked()
                {
                    project = Some((order[2], order));
                }
                let running = self.jobs_export.is_some();
                export_jobs = ui
                    .add_enabled(!running, egui::Button::new(if running { "Exporting jobs..." } else { "Export jobs" }))
                    .on_hover_text("Every family's and outfit's templates, and data/textures/jobs.json for the texture generator (all families; takes a while)")
                    .clicked();
                match self.sheet.as_ref().map(|s| &s.pixels) {
                    Some(Err(e)) => {
                        ui.colored_label(ui.visuals().warn_fg_color, e);
                    }
                    Some(Ok(None)) | None => {
                        ui.weak("no sheet.png yet");
                    }
                    Some(Ok(Some(_))) => {}
                }
                if let Some((_, _, Err(e))) = &self.sheet_texture {
                    ui.colored_label(ui.visuals().warn_fg_color, e);
                }
            });
        });
        if build_all || project.is_some() {
            let outcome = match self.session.puppet().clone() {
                Err(why) => Err(format!("no puppet to texture yet: {why}")),
                Ok(puppet) => {
                    let outlines = &self.session.outlines;
                    match project {
                        Some((view, order)) => {
                            let creature = &self.puppet_view.creature;
                            let dressed = if creature.is_empty() {
                                Ok((crate::editor::texture::Dressed::new(&puppet, outlines, &family, None), crate::editor::texture::dir(&data_root, &family)))
                            } else {
                                crate::editor::texture::Dressed::creature(&data_root, &puppet, outlines, &family, creature)
                                    .map(|d| (d, crate::editor::texture::creature_dir(&data_root, &family, creature)))
                            };
                            dressed.and_then(|(d, folder)| crate::editor::texture::export_projected(&folder, &d, view, gs_puppet::GRAZING, order)).map(|files| {
                                format!("Projected: {}", files.iter().map(|f| f.display().to_string()).collect::<Vec<_>>().join(", "))
                            })
                        }
                        None => crate::editor::texture::build_every_skin(&data_root, &family, &puppet, outlines).map(|(built, failed)| {
                            let mut s = format!("Built {} skin(s): {}; Save puts them on the stage", built.len(), built.join(", "));
                            if !failed.is_empty() {
                                s += &format!("; FAILED: {}", failed.join("; "));
                            }
                            s
                        }),
                    }
                }
            };
            self.session.status = outcome.unwrap_or_else(|e| e);
            reload_sheet |= build_all;
        }
        if export_jobs && self.jobs_export.is_none() {
            let (tx, rx) = std::sync::mpsc::channel();
            let (root, outlines) = (data_root.clone(), self.session.outlines.clone());
            std::thread::spawn(move || {
                let _ = tx.send(crate::editor::texture::export_jobs(&root, &outlines));
            });
            self.jobs_export = Some(rx);
            self.session.status = "Exporting every template and jobs.json...".into();
        }
        if let Some(rx) = &self.jobs_export {
            match rx.try_recv() {
                Ok(result) => {
                    self.session.status = match result {
                        Ok(done) if done.skipped.is_empty() => format!("{} jobs written to {}", done.jobs, done.path.display()),
                        Ok(done) => format!("{} jobs written to {}; SKIPPED: {}", done.jobs, done.path.display(), done.skipped.join("; ")),
                        Err(e) => format!("EXPORT FAILED: {e}"),
                    };
                    self.jobs_export = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => ui.ctx().request_repaint_after(std::time::Duration::from_millis(250)),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.jobs_export = None,
            }
        }
        if export || build {
            let outcome = match self.session.puppet().clone() {
                Err(why) => Err(format!("no puppet to texture yet: {why}")),
                Ok(puppet) => {
                    let outlines = &self.session.outlines;
                    if export {
                        crate::editor::texture::export_templates(&data_root, &family, &puppet, outlines)
                            .map(|dir| format!("Templates written to {}", dir.display()))
                    } else if self.puppet_view.creature.is_empty() {
                        crate::editor::texture::build_sheet(&data_root, &family, &puppet, outlines)
                            .map(|(path, n)| format!("{} built from {n} painting(s); Save puts it on the stage", path.display()))
                    } else {
                        let creature = &self.puppet_view.creature;
                        crate::editor::texture::Dressed::creature(&data_root, &puppet, outlines, &family, creature)
                            .and_then(|d| crate::editor::texture::build_sheet_in(&crate::editor::texture::creature_dir(&data_root, &family, creature), &d))
                            .map(|(path, n)| format!("{} built from {n} painting(s); Save puts it on the stage", path.display()))
                    }
                }
            };
            self.session.status = outcome.unwrap_or_else(|e| e);
            reload_sheet |= build;
        }
        if reload_sheet {
            self.sheet_serial += 1;
        }
        if reload_stage {
            // Reload means the files on disk: drop unsaved pose edits'
            // parse so the editor re-reads them too (unless unsaved).
            if !self.poses_dirty() {
                self.pose_doc = None;
            }
            self.session.status = match self.session.install_poses() {
                Ok(n) => format!("Poses reloaded: {n} pose sets now on the stage too"),
                Err(e) => e,
            };
        }
        if self.pose_edit {
            egui::Panel::bottom("pose_editor")
                .resizable(true)
                // Half the panel: sliders and limits need the room.
                .default_size((ui.available_height() * 0.5).max(250.0))
                .min_size(120.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.pose_editor(ui, &family, &body_type));
                });
        }
        ui.weak(if self.pose_edit { "drag: turn   wheel: zoom   click: pick a bone" } else { "drag: turn   wheel: zoom" });

        let pv = &mut self.puppet_view;
        // The dead pose a death falls into: this press's variety, or the one
        // nearest a pose already lying down (as the stage picks).
        let seed = self.death_seed;
        let dead_of = |set: &gs_puppet::PoseSet, from: &gs_puppet::Pose| {
            set.dead_name(set.dead_hash(from, seed)).and_then(|n| set.get(n)).cloned().unwrap_or_default()
        };
        // The pose shown (held inside the joint limits, as the stage holds
        // it), the dead pose a death falls into, and the set's limits.
        let shown_of = |set: &gs_puppet::PoseSet, name: &str| {
            let pose = if name.is_empty() { gs_puppet::Pose::default() } else { set.get(name).cloned().unwrap_or_default() };
            let pose = set.clamp(&pose);
            let dead = dead_of(set, &pose);
            (pose, dead, set.clone())
        };
        let pose_name = pv.pose.clone();
        let from_doc = self.pose_doc.as_mut().filter(|d| d.body_type() == body_type).map(|d| {
            let revision = d.revision();
            let (pose, dead, set) = match d.set_for(&family) {
                Ok(set) => {
                    let (pose, dead, set) = shown_of(set, &pose_name);
                    (pose, dead, Some(set))
                }
                Err(_) => (gs_puppet::Pose::default(), gs_puppet::Pose::default(), None),
            };
            (pose, dead, set, revision + 1)
        });
        let (pose, dead, limits, poses_revision) = match from_doc {
            Some(p) => p,
            None => match pv.poses(&data_root, &body_type, &family) {
                Ok((set, _)) => {
                    let (pose, dead, set) = shown_of(set, &pose_name);
                    (pose, dead, Some(set), 0)
                }
                Err(_) => (gs_puppet::Pose::default(), gs_puppet::Pose::default(), None, 0),
            },
        };
        // Playing a death: the fall, then the dead pose held a moment.
        let falling = self.death.map(|start| start.elapsed().as_secs_f32());
        if falling.is_some_and(|t| t > gs_puppet::fall::FALL_SECS + 1.0) {
            self.death = None;
        }
        let falling = falling.filter(|_| self.death.is_some());
        if falling.is_some() {
            ui.ctx().request_repaint();
        }
        let (yaw, pitch, zoom) = (pv.yaw, pv.pitch, pv.zoom);
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = response.rect;
        if response.dragged() {
            let d = response.drag_delta();
            self.puppet_view.orbit(d.x, d.y);
        }
        let mut zooming = false;
        if response.hovered() {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            zooming = scroll != 0.0 || pinch != 1.0;
            self.puppet_view.zoom_by((scroll * 0.002).exp() * pinch);
        }
        let draft = response.dragged()
            || zooming
            || self.session.editor.is_dragging()
            || self.session.editor.is_depth_dragging()
            || self.radius_drag
            || self.thickness_drag
            || self.depth_value_drag;

        let dark = ui.visuals().dark_mode;
        let (bg, grid, shadow) = if dark {
            (Color32::from_rgb(0x2b, 0x30, 0x36), [0x4a, 0x52, 0x5c, 255], [0x1d, 0x21, 0x26, 255])
        } else {
            (Color32::from_rgb(0xe8, 0xea, 0xed), [0xc4, 0xc9, 0xd0, 255], [0xb8, 0xbd, 0xc4, 255])
        };
        let painter = painter.with_clip_rect(rect);
        painter.rect_filled(rect, 0.0, bg);
        let revision = self.session.editor.revision();
        // The sheet on disk, read once per family / reload; the texture it
        // makes, rebuilt when the rig changes (its layout follows the rig).
        let creature = self.puppet_view.creature.clone();
        // A creature is the family's puppet in its outfit, rebuilt when the
        // rig, the creature or the files change.
        if !creature.is_empty() && self.dressed.as_ref().is_none_or(|f| f.revision != revision || f.creature != creature || f.serial != self.sheet_serial) {
            let made = match self.session.puppet() {
                Ok(p) => crate::editor::texture::outfits(&data_root).map(|(_, outfits)| {
                    std::sync::Arc::new(match outfits.get(&creature) {
                        Some(o) if !o.pieces.is_empty() => p.wearing(o),
                        _ => p.clone(),
                    })
                }),
                Err(e) => Err(e.clone()),
            };
            self.dressed = Some(Figure { revision, creature: creature.clone(), serial: self.sheet_serial, puppet: made });
            self.sheet_texture = None;
        }
        let who = if creature.is_empty() { family.clone() } else { format!("{family}/{creature}") };
        let want_texture = self.puppet_view.skin && self.puppet_view.texture;
        if want_texture && self.sheet.as_ref().is_none_or(|s| s.who != who || s.serial != self.sheet_serial) {
            let path = if creature.is_empty() {
                crate::editor::texture::sheet_path(&data_root, &family)
            } else {
                crate::editor::texture::creature_dir(&data_root, &family, &creature).join(crate::editor::texture::SHEET)
            };
            let pixels = if path.is_file() {
                crate::editor::texture::read_png(&path).map(|(w, h, px)| Some((w, h, std::sync::Arc::new(px))))
            } else {
                Ok(None)
            };
            self.sheet = Some(SheetFile { who: who.clone(), serial: self.sheet_serial, pixels });
            self.sheet_texture = None;
        }
        // The figure shown: the creature's, else the family's.
        let dressed = if creature.is_empty() { None } else { self.dressed.as_ref().map(|f| &f.puppet) };
        let figure: Result<&gs_puppet::Puppet, String> = {
            let _t = crate::perf_log::span("rigs: build puppet");
            match dressed {
                Some(Ok(p)) => Ok(p.as_ref()),
                Some(Err(e)) => Err(e.clone()),
                None => self.session.puppet().as_ref().map_err(|e| e.clone()),
            }
        };
        if want_texture && self.sheet_texture.as_ref().is_none_or(|(r, s, _)| *r != revision || *s != self.sheet_serial) {
            let made = match (self.sheet.as_ref().map(|s| &s.pixels), &figure) {
                (Some(Ok(Some((w, h, px)))), Ok(puppet)) => {
                    crate::editor::texture::sheet_texture(puppet, *w, *h, px.as_ref().clone()).map(|t| Some(std::sync::Arc::new(t)))
                }
                _ => Ok(None),
            };
            self.sheet_texture = Some((revision, self.sheet_serial, made));
        }
        let texture = if want_texture {
            self.sheet_texture.as_ref().and_then(|(_, _, t)| t.as_ref().ok().cloned().flatten())
        } else {
            None
        };
        let texture_serial = texture.as_ref().map(|_| self.sheet_serial);
        // A fitted cloud, when the family has one and splats are on: read
        // once per file, bound once per rig revision (~70 ms for 70k).
        if self.puppet_view.skin && self.puppet_view.splats {
            let path = gs_puppet::splat::cloud_path(&data_root, &family);
            let stamp = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            if self.cloud.as_ref().is_none_or(|c| c.family != family || c.stamp != stamp) {
                let gaussians = if stamp == 0 {
                    Ok(None)
                } else {
                    let _t = crate::perf_log::span("rigs: read cloud");
                    gs_puppet::splat::load_ply(&path).map(|mut g| {
                        g.retain(|x| x.alpha >= 0.02);
                        Some(std::sync::Arc::new(g))
                    }).map_err(|e| e.to_string())
                };
                self.cloud = Some(CloudFile { family: family.clone(), stamp, gaussians });
                self.bound = None;
            }
        }
        let cloud_stamp = self.cloud.as_ref().filter(|c| matches!(c.gaussians, Ok(Some(_)))).map(|c| c.stamp);
        let held = self.puppet_view.held(&data_root);
        let hold = [self.puppet_view.hold.clone(), self.puppet_view.hold_left.clone()];
        match figure {
            Ok(puppet) => {
                let armed = (!held.is_empty()).then(|| puppet.holding(&held.iter().map(|(h, p)| (*h, p)).collect::<Vec<_>>()));
                let puppet = armed.as_ref().unwrap_or(puppet);
                let mut cam = gs_puppet::Camera::framing(puppet.height.max(0.5), [rect.width(), rect.height()]);
                cam.center = [rect.center().x, rect.center().y];
                cam.focal *= zoom;
                cam.pitch_deg = pitch;
                let look = gs_puppet::Look { yaw, outline_px: 1.5, ..gs_puppet::Look::default() };
                let shown = match (falling, limits.as_ref()) {
                    (Some(t), Some(set)) => set.clamp(&gs_puppet::Fall::new(puppet, &pose, &dead, self.death_seed).at(t)),
                    (Some(t), None) => gs_puppet::Fall::new(puppet, &pose, &dead, self.death_seed).at(t),
                    (None, _) => pose.clone(),
                };
                let posed = puppet.pose(&shown);
                let project = |p: gs_puppet::math::V3| cam.project(p);
                gs_puppet::paint(&painter, Vec2::ZERO, &posed.ground_shadow(&project, &look, shadow));
                gs_puppet::paint(&painter, Vec2::ZERO, &gs_puppet::ground_grid(&project, 5, grid, 1.0));
                if self.puppet_view.skin {
                    // Rendered about the panel's own corner, so moving the
                    // panel does not re-render it.
                    let key = SkinKey {
                        revision,
                        pose: match falling {
                            Some(t) => format!("{}@{:.3}", self.puppet_view.pose, t.min(gs_puppet::fall::FALL_SECS)),
                            None => self.puppet_view.pose.clone(),
                        },
                        view: [yaw, pitch, zoom, rect.width(), rect.height()].map(f32::to_bits),
                        texture: texture_serial,
                        poses: poses_revision,
                        hold: hold.clone(),
                        creature: creature.clone(),
                        draft,
                        splats: self.puppet_view.splats.then(|| {
                            let p = self.puppet_view.splat;
                            [(p.spacing * 1000.0).round() as u32, (p.overlap * 100.0).round() as u32, (p.alpha * 100.0).round() as u32]
                        }),
                        cloud: cloud_stamp,
                    };
                    if self.skin.as_ref().is_none_or(|(k, _, _)| *k != key) {
                        let s = if draft { DRAFT_SCALE } else { 1.0 };
                        let mut local = cam;
                        local.focal *= s;
                        local.center = [rect.width() * 0.5 * s, rect.height() * 0.5 * s];
                        let _t = crate::perf_log::span("rigs: skin");
                        let fitted = cloud_stamp.and_then(|stamp| {
                            let gaussians = self.cloud.as_ref()?.gaussians.as_ref().ok()?.clone()?;
                            if self.bound.as_ref().is_none_or(|(r, s, _)| *r != revision || *s != stamp) {
                                let _t = crate::perf_log::span("rigs: bind cloud");
                                let mut rest = gaussians.as_ref().clone();
                                gs_puppet::splat::fit_to_height(&mut rest, puppet.height);
                                self.bound = Some((revision, stamp, std::sync::Arc::new(gs_puppet::splat::BoundCloud::bind(puppet, rest))));
                            }
                            self.bound.as_ref().map(|(_, _, b)| b.clone())
                        });
                        let skin = if let (true, Some(bound)) = (self.puppet_view.splats, fitted) {
                            // The family's fitted cloud, moved by the rig. While
                            // something moves, every fourth splat: the sort and
                            // blend are per splat, and a 68k cloud is 130 ms.
                            let mut cloud = bound.posed(&posed);
                            if draft {
                                cloud = cloud.into_iter().step_by(4).collect();
                            }
                            self.splat_count = cloud.len();
                            gs_puppet::splat::render(&cloud, &|p| local.project(p), look.yaw)
                        } else if self.puppet_view.splats {
                            // Twice the spacing while something moves: a quarter
                            // of the splats (a canine at 0.015 ft is 248k).
                            let mut params = self.puppet_view.splat;
                            if draft {
                                params.spacing *= 2.0;
                            }
                            let cloud = gs_puppet::splat::cloud(&posed, texture.as_deref(), look.body, &params);
                            self.splat_count = cloud.len();
                            gs_puppet::splat::render(&cloud, &|p| local.project(p), look.yaw)
                        } else {
                            self.splat_count = 0;
                            posed.skin_textured(&|p| local.project(p), &look, texture.as_deref())
                        };
                        let image = egui::ColorImage::from_rgba_unmultiplied([skin.w, skin.h], &skin.rgba);
                        let tex = ui.ctx().load_texture("rig_puppet_skin", image, egui::TextureOptions::LINEAR);
                        self.skin = Some((key, tex, Vec2::new(skin.origin[0], skin.origin[1])));
                    }
                    if let Some((k, tex, origin)) = &self.skin {
                        let s = if k.draft { DRAFT_SCALE } else { 1.0 };
                        let at = egui::Rect::from_min_size(rect.min + *origin / s, tex.size_vec2() / s);
                        painter.image(tex.id(), at, egui::Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                    }
                } else {
                    gs_puppet::paint(&painter, Vec2::ZERO, &posed.draw_list(&project, &look));
                }
                if self.pose_edit {
                    // Click a bone to edit it; the edited one is outlined.
                    let split = |stem: &str| {
                        self.pose_doc.as_ref().is_some_and(|d| d.pair_is_split(self.pose_scope, &family, &self.puppet_view.pose, stem))
                    };
                    let rows = pose_edit::rows(puppet, &split);
                    if response.clicked() {
                        if let Some(at) = response.interact_pointer_pos() {
                            let picked = pose_edit::pick_bone(&posed, yaw, &project, [at.x, at.y], 14.0);
                            self.pose_row = Some(picked.and_then(|b| pose_edit::row_of(&rows, &b)).map_or(ROOT.to_owned(), |r| r.key.clone()));
                        }
                    }
                    if let Some(row) = rows.iter().find(|r| Some(&r.key) == self.pose_row.as_ref()) {
                        for [a, b] in pose_edit::bone_lines(&posed, yaw, &project, &row.bones) {
                            painter.line_segment([Pos2::new(a[0], a[1]), Pos2::new(b[0], b[1])], Stroke::new(3.0, Color32::from_rgb(255, 210, 60)));
                            painter.circle_filled(Pos2::new(a[0], a[1]), 3.5, Color32::from_rgb(255, 210, 60));
                        }
                    }
                }
                painter.text(
                    rect.left_bottom() + Vec2::new(8.0, -8.0),
                    egui::Align2::LEFT_BOTTOM,
                    if self.puppet_view.skin && self.puppet_view.splats && self.splat_count > 0 {
                        let fitted = if cloud_stamp.is_some() { "fitted cloud, " } else { "" };
                        format!("{} segments, {:.1} ft, {fitted}{} splats", puppet.segments.len(), puppet.height, self.splat_count)
                    } else {
                        format!("{} segments, {:.1} ft", puppet.segments.len(), puppet.height)
                    },
                    egui::FontId::proportional(12.0),
                    guide_text(dark),
                );
            }
            Err(why) => {
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("No puppet yet: {why}"),
                    egui::FontId::proportional(14.0),
                    guide_text(dark),
                );
            }
        }
    }

    fn joint_list(&mut self, ui: &mut egui::Ui) {
        let editor = &mut self.session.editor;
        let order = editor.joint_order();
        let placed = order.iter().filter(|j| editor.is_placed(j)).count();
        ui.horizontal(|ui| {
            ui.strong(format!("Joints  {placed}/{}", order.len()));
            let front = editor.rig().view == View::Front;
            ui.add_enabled(front, egui::Checkbox::new(&mut editor.mirror, "Mirror"))
                .on_hover_text("Placing X_r (or X_l) also places its unplaced twin, reflected across x = 0")
                .on_disabled_hover_text("Mirroring only applies to front views");
        });
        let current = editor.current().map(str::to_owned);
        let selected = editor.selected().map(str::to_owned);
        let mut ask = None;
        ui.horizontal(|ui| {
            if ui.button("Add").on_hover_text("A new joint, after the current one in the click order").clicked() {
                ask = Some(PromptKind::AddJoint);
            }
            let Some(joint) = current.clone() else {
                return;
            };
            if ui.button("Rename").on_hover_text("Renames it in every bone, anchor and eye too").clicked() {
                ask = Some(PromptKind::RenameJoint(joint.clone()));
            }
            let users = editor.joint_users(&joint);
            let hint = if users.is_empty() {
                format!("Delete '{joint}' (no bone uses it)")
            } else {
                format!("Delete '{joint}' and take it out of: {}", users.join(", "))
            };
            if ui.button("Delete").on_hover_text(hint).clicked() {
                editor.delete_joint(&joint);
            }
            if ui.button("Up").on_hover_text("Earlier in the click order").clicked() {
                editor.move_joint(&joint, true);
            }
            if ui.button("Down").on_hover_text("Later in the click order").clicked() {
                editor.move_joint(&joint, false);
            }
            let unused = editor.is_unused(&joint);
            if ui
                .selectable_label(unused, "Unused")
                .on_hover_text(
                    "This family does not have this joint (a worm's eye, a missing leg pair): it is un-placed, skipped by the click-through and not warned about. Click again to bring it back.",
                )
                .clicked()
            {
                editor.set_unused(&joint, !unused);
            }
        });
        // Above the list, so the list can take every pixel that is left.
        match &current {
            Some(j) => ui.weak(format!("Next click places: {j}")),
            None => ui.weak("All joints placed. Click a joint in the list to re-place it."),
        };
        let model = self.session.depth().clone();
        let editor = &mut self.session.editor;
        let depth_available = model.available();
        let mut reset = None;
        egui::ScrollArea::vertical().id_salt("joints").auto_shrink([false, false]).show(ui, |ui| {
            for joint in &order {
                let is_current = current.as_deref() == Some(joint);
                let placed = editor.is_placed(joint);
                let label = if editor.is_unused(joint) { format!("{joint}  (unused)") } else { joint.clone() };
                let mut text = egui::RichText::new(label);
                if is_current {
                    text = text.strong();
                } else if !placed {
                    text = text.weak();
                }
                if selected.as_deref() == Some(joint) {
                    text = text.underline();
                }
                let item = ui
                    .horizontal(|ui| {
                        placed_marker(ui, placed);
                        let item = ui.selectable_label(is_current, text);
                        if depth_available {
                            if let Some(j) = depth_cell(ui, editor, &model, joint) {
                                reset = Some(j);
                            }
                        }
                        item
                    })
                    .inner;
                if item.clicked() {
                    editor.set_current(joint);
                }
                if item.secondary_clicked() {
                    editor.unplace(joint);
                }
            }
        });
        if let Some(joint) = reset {
            editor.reset_joint_depth(&joint);
        }
        if let Some(kind) = ask {
            self.ask(kind);
        }
    }

    fn bone_list(&mut self, ui: &mut egui::Ui) {
        let mut ask = None;
        ui.horizontal(|ui| {
            ui.strong("Bones");
            let editor = &mut self.session.editor;
            let selected = editor.selected_bone();
            if ui.button("Add").on_hover_text("A new bone, hung off the selected bone (a root if none is)").clicked() {
                ask = Some(PromptKind::AddBone);
            }
            if ui.add_enabled(selected.is_some(), egui::Button::new("Rename")).clicked() {
                ask = selected.map(PromptKind::RenameBone);
            }
            let delete = ui
                .add_enabled(selected.is_some(), egui::Button::new("Delete"))
                .on_hover_text("Its children move up to its parent; its joints stay");
            if let (true, Some(bone)) = (delete.clicked(), selected) {
                editor.delete_bone(bone);
            }
        });
        if let Some(kind) = ask {
            self.ask(kind);
        }
        // Both ways: the grid is as wide as its widest row wants to be.
        egui::ScrollArea::both().id_salt("bones").auto_shrink([false, false]).show(ui, |ui| {
            self.bone_grid(ui);
            if self.session.editor.selected_bone().is_none() {
                ui.weak("Click a bone's name: its details open on the right.");
            }
        });
    }

    /// One row per bone, children indented under their parents.
    fn bone_grid(&mut self, ui: &mut egui::Ui) {
        let (radii, pixels): (Vec<Option<f32>>, Vec<usize>) = self
            .session
            .last_bake()
            .map(|b| (b.baked.bone_radii_ft.clone(), b.baked.bone_pixels.clone()))
            .unwrap_or_default();
        let editor = &mut self.session.editor;
        let mut dragging = false;
        let mut released = false;

        egui::Grid::new("rig_bones").num_columns(4).striped(true).show(ui, |ui| {
            ui.weak("Bone").on_hover_text("Click a bone for its parent, pivot, thickness and chain");
            ui.weak("Part").on_hover_text("Which body part its pixels are labelled");
            ui.weak("Width").on_hover_text("Half its thickness across the silhouette. Auto measures it; untick to set it");
            ui.label("");
            ui.end_row();
            for (i, depth) in editor.rig().hierarchy_order() {
                let bone = editor.rig().bones[i].clone();
                let is_selected = editor.selected_bone() == Some(i);
                let chain = if bone.joints.is_empty() { "(no joints yet)".to_owned() } else { bone.joints.join(" -> ") };
                let hover = format!("{chain}\n{} px", pixels.get(i).copied().unwrap_or(0));
                let name = ui
                    .horizontal(|ui| {
                        ui.add_space(depth as f32 * 10.0);
                        ui.selectable_label(is_selected, &bone.name).on_hover_text(hover)
                    })
                    .inner;
                if name.clicked() {
                    editor.select_bone((!is_selected).then_some(i));
                }

                let shown = bone.part.as_deref().unwrap_or("(none)");
                egui::ComboBox::from_id_salt(("bone_part", i)).selected_text(shown).width(88.0).show_ui(ui, |ui| {
                    if ui.selectable_label(bone.part.is_none(), "(none)").clicked() {
                        editor.set_bone_part(i, None);
                    }
                    for (k, part) in PARTS.iter().enumerate() {
                        let text = egui::RichText::new(*part).color(label_color(k + 1));
                        if ui.selectable_label(bone.part.as_deref() == Some(part), text).clicked() {
                            editor.set_bone_part(i, Some(part));
                        }
                    }
                });

                let used = radii.get(i).copied().flatten();
                let mut auto = bone.radius_ft.is_none();
                if ui.checkbox(&mut auto, "Auto").on_hover_text("Measure the silhouette's half-thickness").changed() {
                    // Turning Auto off starts from whatever was measured.
                    editor.set_bone_radius(i, (!auto).then(|| used.unwrap_or(0.25)));
                }
                match bone.radius_ft {
                    Some(mut r) => {
                        let drag = ui.add(
                            egui::DragValue::new(&mut r).speed(0.005).range(0.0..=30.0).fixed_decimals(2).suffix(" ft"),
                        );
                        if drag.changed() {
                            editor.set_bone_radius(i, Some(r));
                        }
                        dragging |= drag.dragged();
                        released |= drag.drag_stopped() || drag.lost_focus();
                    }
                    None => {
                        match used {
                            Some(r) => ui.weak(format!("{r:.2} ft")),
                            None => ui.weak("--").on_hover_text("Not baked: needs a part and two placed joints"),
                        };
                    }
                }
                ui.end_row();
            }
        });
        if released {
            editor.end_continuous_edit();
        }
        self.radius_drag = dragging;
    }

    /// The selected bone (the right panel's inspector), in collapsible
    /// sections: parent and pivot, its thickness overrides, its joint chain.
    fn bone_details(&mut self, ui: &mut egui::Ui, index: usize) {
        let (pixels, width_used) = self
            .session
            .last_bake()
            .map(|b| (b.baked.bone_pixels.get(index).copied(), b.baked.bone_radii_ft.get(index).copied().flatten()))
            .unwrap_or_default();
        // The depth the builder measured, read off the puppet: rebuilt when
        // idle, the last one built while something is being dragged.
        if !self.thickness_drag && !self.radius_drag && !self.session.editor.is_dragging() {
            let _ = self.session.puppet();
        }
        let Some(bone) = self.session.editor.rig().bones.get(index).cloned() else {
            return;
        };
        let depth_used = self.session.last_puppet().and_then(|p| {
            p.segments.iter().filter(|s| s.bone == bone.name).flat_map(|s| s.stations.iter().map(|st| st.d)).reduce(f32::max)
        });
        let editor = &mut self.session.editor;
        let depth_set = editor.rig().radius_depth_ft.get(&bone.name).copied();
        let tip_set = editor.rig().radius_tip_ft.get(&bone.name).copied();
        let off_plane = match editor.rig().view {
            View::Front => "front-to-back",
            View::Side => "side-to-side",
        };

        if let Some(n) = pixels {
            ui.weak(format!("{n} px of the silhouette"));
        }

        egui::CollapsingHeader::new("Parent and pivot").id_salt("bone_sec_hierarchy").default_open(true).show(ui, |ui| {
            egui::Grid::new("rig_bone_details").num_columns(2).show(ui, |ui| {
                ui.label("Parent").on_hover_text("The bone this one hangs off. Its own children are not offered.");
                let shown = bone.parent.as_deref().unwrap_or("(root)");
                egui::ComboBox::from_id_salt("bone_parent").selected_text(shown).width(150.0).show_ui(ui, |ui| {
                    if ui.selectable_label(bone.parent.is_none(), "(root)").clicked() {
                        editor.set_bone_parent(index, None);
                    }
                    for choice in editor.parent_choices(index) {
                        if ui.selectable_label(bone.parent.as_deref() == Some(choice.as_str()), &choice).clicked() {
                            editor.set_bone_parent(index, Some(&choice));
                        }
                    }
                });
                ui.end_row();

                ui.label("Pivot").on_hover_text("The joint this bone's piece will rotate about when posed");
                let first = bone.joints.first().map_or("(first joint)".to_owned(), |j| format!("(first: {j})"));
                let shown = bone.pivot.clone().unwrap_or_else(|| first.clone());
                egui::ComboBox::from_id_salt("bone_pivot").selected_text(shown).width(150.0).show_ui(ui, |ui| {
                    if ui.selectable_label(bone.pivot.is_none(), first).clicked() {
                        editor.set_bone_pivot(index, None);
                    }
                    for joint in &bone.joints {
                        if ui.selectable_label(bone.pivot.as_deref() == Some(joint.as_str()), joint).clicked() {
                            editor.set_bone_pivot(index, Some(joint));
                        }
                    }
                });
                ui.end_row();
            });
        });

        let mut drag = DragState::default();
        egui::CollapsingHeader::new("Thickness").id_salt("bone_sec_thickness").default_open(true).show(ui, |ui| {
            egui::Grid::new("rig_bone_thickness").num_columns(2).show(ui, |ui| {
                ui.label("Width").on_hover_text(
                    "Half the bone's thickness across this silhouette, in feet (the same as its row in the list above).\n\
                     Auto measures it from the silhouette.",
                );
                if let Some(v) = thickness_row(ui, bone.radius_ft, width_used, Toggle::Auto, 0.25, &mut drag) {
                    editor.set_bone_radius(index, v);
                }
                ui.end_row();

                ui.label("Depth").on_hover_text(format!(
                    "Half the bone's thickness {off_plane}, the direction this silhouette cannot show, in feet.\n\
                     Auto measures it from the family's other silhouette. Set it for something the other view \
                     hides or gets wrong: a wing that is a thin blade, a flat tail, a tusk."
                ));
                if let Some(v) = thickness_row(ui, depth_set, depth_used, Toggle::Auto, 0.1, &mut drag) {
                    editor.set_bone_depth_radius(index, v);
                }
                ui.end_row();

                ui.label("Tip width").on_hover_text(
                    "Narrow the bone from its width at the first joint to this at the last, in feet, \
                     along its whole chain: a feather blade to its point, a horn, a tail.\n\
                     Off = no taper (the measured width all along).",
                );
                let start = bone.radius_ft.or(width_used).unwrap_or(0.1);
                if let Some(v) = thickness_row(ui, tip_set, None, Toggle::Taper, (start * 0.25).max(0.01), &mut drag) {
                    editor.set_bone_tip_radius(index, v);
                }
                ui.end_row();

                ui.label("Membrane").on_hover_text(
                    "A skin stretched between bones -- a bat's or a dragon's wing -- instead of a tube round this one. \
                     Tick every bone of the wing: the 3D puppet lays them flat in the plane of their joints, as wide as \
                     the drawing and about an inch thick (Depth sets the half-thickness), and they fold in that plane.",
                );
                let mut on = bone.membrane;
                if ui.checkbox(&mut on, "skin, not a tube").changed() {
                    editor.set_bone_membrane(index, on);
                }
                ui.end_row();
            });
            ui.weak("The 3D preview shows the change as you drag.");
        });
        if drag.released {
            editor.end_continuous_edit();
        }
        self.thickness_drag = drag.dragging;
        let editor = &mut self.session.editor;

        let chain_title = format!("Chain ({} joints)", bone.joints.len());
        egui::CollapsingHeader::new(chain_title).id_salt("bone_sec_chain").default_open(true).show(ui, |ui| {
            egui::Grid::new("rig_bone_chain").num_columns(5).show(ui, |ui| {
                for (k, joint) in bone.joints.iter().enumerate() {
                    ui.weak(format!("{}.", k + 1));
                    let mut text = egui::RichText::new(joint);
                    if !editor.is_placed(joint) {
                        text = text.weak();
                    }
                    ui.label(text);
                    if ui.add_enabled(k > 0, egui::Button::new("Up").small()).clicked() {
                        editor.chain_move(index, k, true);
                    }
                    if ui.add_enabled(k + 1 < bone.joints.len(), egui::Button::new("Down").small()).clicked() {
                        editor.chain_move(index, k, false);
                    }
                    if ui.add(egui::Button::new("Remove").small()).clicked() {
                        editor.chain_remove(index, k);
                    }
                    ui.end_row();
                }
            });
            if bone.joints.is_empty() {
                ui.weak("No joints yet.");
            }
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("bone_chain_add").selected_text("Append joint...").width(150.0).show_ui(ui, |ui| {
                    for joint in editor.chain_choices(index) {
                        if ui.selectable_label(false, &joint).clicked() {
                            editor.chain_insert(index, usize::MAX, &joint);
                        }
                    }
                });
                let picking = editor.chain_pick() == Some(index);
                let pick = ui
                    .add(egui::Button::selectable(picking, "Pick on canvas"))
                    .on_hover_text("While on, clicking a placed joint on the canvas appends it to this chain (Esc ends)");
                if pick.clicked() {
                    editor.set_chain_pick((!picking).then_some(index));
                }
            });
        });
    }

    // --------------------------------------------------------------- right

    /// Collapsible sections, what needs attention first: warnings, part
    /// shares, anchors and eyes, depth, then the map's numbers (closed).
    fn right_panel(&mut self, ui: &mut egui::Ui) {
        let Some(bake) = self.session.last_bake() else {
            ui.label("No silhouette for this family / view.");
            return;
        };
        // Depth warnings sit with the rest rather than in their own corner.
        let mut all = bake.warnings.clone();
        all.extend(self.session.depth().warnings(self.session.editor.rig()));
        let warn = ui.visuals().warn_fg_color;
        let title = egui::RichText::new(format!("Warnings ({})", all.len()));
        let title = if all.is_empty() { title } else { title.color(warn) };
        egui::CollapsingHeader::new(title).id_salt("rig_sec_warnings").default_open(true).show(ui, |ui| {
            if all.is_empty() {
                ui.weak("none");
            }
            for w in &all {
                ui.horizontal_wrapped(|ui| ui.colored_label(warn, format!("\u{26A0} {w}")));
            }
        });

        if let Some(index) = self.session.editor.selected_bone() {
            let name = self.session.editor.rig().bones.get(index).map(|b| b.name.clone()).unwrap_or_default();
            egui::CollapsingHeader::new(egui::RichText::new(format!("Bone: {name}")).strong())
                .id_salt("rig_sec_bone")
                .default_open(true)
                .show(ui, |ui| self.bone_details(ui, index));
        }

        let Some(bake) = self.session.last_bake() else {
            return;
        };
        let baked = &bake.baked;
        egui::CollapsingHeader::new("Parts").id_salt("rig_sec_parts").default_open(true).show(ui, |ui| {
            egui::Grid::new("rig_shares").num_columns(3).show(ui, |ui| {
                for (k, part) in PARTS.iter().enumerate() {
                    let share = baked.share(part);
                    let assigned = self.session.editor.rig().bones.iter().any(|b| b.part.as_deref() == Some(part));
                    if share == 0.0 && !assigned {
                        continue;
                    }
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                    ui.painter().rect_filled(rect, 2.0, label_color(k + 1));
                    ui.label(*part);
                    let text = format!("{:5.1}%", share * 100.0);
                    if share < MIN_SHARE {
                        ui.colored_label(warn, text).on_hover_text("Hardly any of the silhouette: is a bone missing its part?");
                    } else {
                        ui.monospace(text);
                    }
                    ui.end_row();
                }
            });
        });

        let eye_used = baked.eye_radius_ft;
        egui::CollapsingHeader::new("Anchors and eyes")
            .id_salt("rig_sec_anchors")
            .default_open(true)
            .show(ui, |ui| self.anchors_and_eyes(ui, eye_used));

        egui::CollapsingHeader::new("Depth").id_salt("rig_sec_depth").default_open(true).show(ui, |ui| self.depth_panel(ui));

        let Some(bake) = self.session.last_bake() else {
            return;
        };
        let baked = &bake.baked;
        egui::CollapsingHeader::new("Map").id_salt("rig_sec_map").default_open(false).show(ui, |ui| {
            let grid = baked.map.grid;
            ui.label(format!("{} x {} px at {:.1} px/ft", grid.w, grid.h, baked.px_per_ft));
            ui.label(format!("aspect {:.3}   span {:.3}", baked.metrics.aspect, baked.metrics.span));
            ui.label(format!("ground at {:.1}% down", baked.ground_frac * 100.0));
            ui.label(format!("orphan pixels: {}", baked.map.orphans));
        });
    }

    /// Depth: how many joints are corrected, the resets, and the selected
    /// joint's own value.
    fn depth_panel(&mut self, ui: &mut egui::Ui) {
        let model = self.session.depth().clone();
        let editor = &mut self.session.editor;
        let rig_view = editor.rig().view;
        if !model.available() {
            ui.weak("Not available for this family.");
            ui.horizontal_wrapped(|ui| ui.weak(model.unavailable_reason(editor.rig())));
            return;
        }
        ui.weak(format!("off-plane axis: {}", axis_label(rig_view)));
        let (over, placed) = model.override_counts(editor.rig());
        let text = format!("{over} of {placed} overridden");
        if over > 0 {
            ui.strong(text);
        } else {
            ui.weak(format!("{text} (all guessed)"));
        }
        ui.horizontal(|ui| {
            let reset = ui
                .add_enabled(over > 0, egui::Button::new("Reset all to guess"))
                .on_hover_text("Drop every override; the guesses apply again")
                .on_disabled_hover_text("Nothing is overridden");
            if reset.clicked() {
                editor.reset_all_depths();
            }
            if editor.edit_view() != EditView::Depth
                && ui.button("Edit in DEPTH").on_hover_text("Switch the canvas to the other silhouette").clicked()
            {
                editor.set_edit_view(EditView::Depth);
            }
        });

        // The selected joint, so a value can be typed instead of dragged.
        let Some(joint) = editor.selected().map(str::to_owned) else {
            ui.weak("Select a joint to set its depth by hand.");
            return;
        };
        let d = model.joint(editor.rig(), &joint);
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong(&joint);
            match (d.overridden, d.matches_guess()) {
                (true, true) => {
                    ui.label("overridden").on_hover_text(
                        "It is set by hand, but it sits on the guess: resetting it changes nothing on screen \
                         and lets a better guess apply later.",
                    );
                }
                (true, false) => {
                    ui.label("overridden");
                }
                (false, _) => {
                    ui.weak("guessed");
                }
            }
        });
        let mut dragging = false;
        let mut released = false;
        ui.horizontal(|ui| {
            let mut value = d.value;
            let drag = ui.add(
                egui::DragValue::new(&mut value).speed(0.005).range(-40.0..=40.0).fixed_decimals(3).suffix(" ft"),
            );
            if drag.changed() {
                editor.set_joint_depth(&joint, value);
            }
            dragging |= drag.dragged();
            released |= drag.drag_stopped() || drag.lost_focus();
            let reset = ui
                .add_enabled(d.overridden, egui::Button::new("Reset to guess"))
                .on_disabled_hover_text("This joint is already at its guess");
            if reset.clicked() {
                editor.reset_joint_depth(&joint);
            }
        });
        match model.guess(&joint) {
            Some(g) => {
                let how = model.guess_source(&joint).unwrap_or("");
                ui.weak(format!("guess {g:+.3} ft -- {how}"))
            }
            None => ui.weak("no guess for this joint"),
        };
        if released {
            editor.end_continuous_edit();
        }
        self.depth_value_drag = dragging;
    }

    fn anchors_and_eyes(&mut self, ui: &mut egui::Ui, eye_used: Option<f32>) {
        let editor = &mut self.session.editor;
        let joints = editor.joint_order();
        let side = editor.rig().view == View::Side;
        let mut dragging = false;
        let mut released = false;
        egui::Grid::new("rig_anchors").num_columns(2).show(ui, |ui| {
            for slot in AnchorSlot::ALL {
                ui.label(slot.label());
                let now = slot.get(editor.rig()).to_owned();
                let shown = if joints.contains(&now) { now.clone() } else { "(none)".to_owned() };
                egui::ComboBox::from_id_salt(("anchor", slot.label())).selected_text(shown).width(140.0).show_ui(ui, |ui| {
                    if ui.selectable_label(now.is_empty(), "(none)").clicked() {
                        editor.set_anchor(slot, "");
                    }
                    for j in &joints {
                        if ui.selectable_label(*j == now, j).clicked() {
                            editor.set_anchor(slot, j);
                        }
                    }
                });
                ui.end_row();
            }
            for (left, label) in [(true, "left eye"), (false, "right eye")] {
                let note = if side { "A side view shows one eye: leave the other at (none) or unplaced" } else { "" };
                let text = ui.label(label);
                if !note.is_empty() {
                    text.on_hover_text(note);
                }
                let now = editor.eye_joint(left).map(str::to_owned);
                let shown = now.clone().unwrap_or_else(|| "(none)".to_owned());
                egui::ComboBox::from_id_salt(("eye", left)).selected_text(shown).width(140.0).show_ui(ui, |ui| {
                    if ui.selectable_label(now.is_none(), "(none)").clicked() {
                        editor.set_eye_joint(left, None);
                    }
                    for j in &joints {
                        if ui.selectable_label(now.as_deref() == Some(j.as_str()), j).clicked() {
                            editor.set_eye_joint(left, Some(j));
                        }
                    }
                });
                ui.end_row();
            }
            ui.label("Eye radius").on_hover_text(
                "Each placed eye is painted as a disc of this radius over the head's pixels.\n\
                 Auto = 18% of the head bone's measured radius, never under 2 px at 512.",
            );
            ui.horizontal(|ui| {
                let set = editor.rig().eye_radius_ft;
                let mut auto = set.is_none();
                if ui.checkbox(&mut auto, "Auto").changed() {
                    editor.set_eye_radius((!auto).then(|| eye_used.unwrap_or(0.08)));
                }
                match set {
                    Some(mut r) => {
                        let drag = ui.add(
                            egui::DragValue::new(&mut r).speed(0.002).range(0.0..=5.0).fixed_decimals(3).suffix(" ft"),
                        );
                        if drag.changed() {
                            editor.set_eye_radius(Some(r));
                        }
                        dragging |= drag.dragged();
                        released |= drag.drag_stopped() || drag.lost_focus();
                    }
                    None => {
                        match eye_used {
                            Some(r) => ui.weak(format!("{r:.3} ft")),
                            None => ui.weak("--").on_hover_text("No eye is placed, or no head bone is baked"),
                        };
                    }
                }
            });
            ui.end_row();
        });
        if released {
            editor.end_continuous_edit();
        }
        self.eye_drag = dragging;
    }

    // -------------------------------------------------------------- centre

    fn centre(&mut self, ui: &mut egui::Ui) {
        if self.session.editor.edit_view() == EditView::Depth {
            self.centre_depth(ui);
            return;
        }
        let mut fit = std::mem::take(&mut self.session.fit_requested);
        // A new document reframes DEPTH too, next time it is shown.
        self.depth_fit |= fit;
        ui.horizontal_wrapped(|ui| {
            fit |= ui.button("Fit").clicked();
            ui.weak(format!("{:.0} px/ft", self.view.zoom));
            ui.separator();
            let picking = self.session.editor.chain_pick().and_then(|b| self.session.editor.rig().bones.get(b));
            match picking {
                Some(bone) => {
                    let text = format!("PICKING the chain of '{}': click joints in order. Esc or the button ends.", bone.name);
                    ui.colored_label(ui.visuals().warn_fg_color, text);
                }
                None => {
                    ui.weak("click: place   drag: move   right-click / Del: un-place   wheel: zoom   middle or Space+drag: pan");
                }
            }
        });

        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = response.rect;
        let canvas = Canvas { min: [rect.min.x, rect.min.y], size: [rect.width(), rect.height()] };
        if fit {
            if let Some(bounds) = self.session.outline().and_then(|o| o.bounds()) {
                self.view.fit(&canvas, bounds, FIT_MARGIN_PX);
            }
        }

        self.canvas_input(ui, &response, &canvas);
        self.upload_texture(ui.ctx());
        self.paint(ui, &painter, &response, &canvas);
    }

    /// DEPTH view: the OTHER silhouette, each placed joint plotted at
    /// (its depth, the height it was authored at). Dragging moves a joint
    /// horizontally only.
    fn centre_depth(&mut self, ui: &mut egui::Ui) {
        let mut fit = std::mem::take(&mut self.depth_fit);
        // A fit asked for by the document (a new family) reframes both views.
        fit |= std::mem::replace(&mut self.session.fit_requested, false);
        let rig_view = self.session.editor.rig().view;
        ui.horizontal_wrapped(|ui| {
            fit |= ui.button("Fit").clicked();
            ui.weak(format!("{:.0} px/ft", self.depth_view.zoom));
            ui.separator();
            ui.weak(format!(
                "DEPTH on the {} silhouette: across = {}   drag: depth only, y is locked   wheel: zoom   middle or Space+drag: pan",
                other_view(rig_view).as_str(),
                axis_label(rig_view),
            ));
        });

        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = response.rect;
        let canvas = Canvas { min: [rect.min.x, rect.min.y], size: [rect.width(), rect.height()] };
        if fit {
            if let Some(bounds) = self.session.other_outline_shown().and_then(|o| o.bounds()) {
                self.depth_view.fit(&canvas, bounds, FIT_MARGIN_PX);
            }
        }
        self.depth_input(ui, &response, &canvas);
        self.paint_depth(ui, &painter, &response, &canvas);
    }

    fn depth_input(&mut self, ui: &egui::Ui, response: &egui::Response, canvas: &Canvas) {
        use egui::PointerButton::{Middle, Primary};
        let view = &mut self.depth_view;
        let model = self.session.depth().clone();
        let editor = &mut self.session.editor;

        if response.contains_pointer() {
            let (scroll, pinch, at) = ui.input(|i| (i.smooth_scroll_delta().y, i.zoom_delta(), i.pointer.hover_pos()));
            let factor = (scroll * 0.0025).exp() * pinch;
            if let (Some(at), true) = (at, factor != 1.0) {
                view.zoom_at(canvas, [at.x, at.y], factor);
            }
        }

        let space = ui.input(|i| i.key_down(egui::Key::Space));
        if response.dragged_by(Middle) || (space && response.dragged_by(Primary) && !editor.is_depth_dragging()) {
            let d = response.drag_delta();
            view.pan([d.x, d.y]);
            return;
        }

        let world = |p: Pos2| view.screen_to_world(canvas, [p.x, p.y]);
        if response.drag_started_by(Primary) && !space {
            let origin = ui.input(|i| i.pointer.press_origin()).or(response.interact_pointer_pos());
            if let Some(origin) = origin {
                if let Some(joint) = editor.hit_joint_depth(&model, view, canvas, [origin.x, origin.y]) {
                    editor.begin_depth_drag(&model, &joint, world(origin));
                }
            }
        }
        if editor.is_depth_dragging() {
            if let Some(p) = response.interact_pointer_pos() {
                editor.depth_drag_to(world(p));
            }
            if response.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
                editor.end_depth_drag();
            }
        }

        // A click selects; an empty click places nothing (joints belong to
        // the primary view). Right-click resets that joint to its guess.
        if response.clicked_by(Primary) && !space {
            if let Some(p) = response.interact_pointer_pos() {
                if let Some(joint) = editor.hit_joint_depth(&model, view, canvas, [p.x, p.y]) {
                    editor.select(&joint);
                }
            }
        }
        if response.secondary_clicked() {
            if let Some(p) = response.interact_pointer_pos() {
                if let Some(joint) = editor.hit_joint_depth(&model, view, canvas, [p.x, p.y]) {
                    editor.reset_joint_depth(&joint);
                }
            }
        }
    }

    fn paint_depth(&self, ui: &egui::Ui, painter: &egui::Painter, response: &egui::Response, canvas: &Canvas) {
        let view = &self.depth_view;
        let editor = &self.session.editor;
        let model = self.session.depth();
        let rig = editor.rig();
        let rect = response.rect;
        let to_screen = |p: [f32; 2]| pos(view.world_to_screen(canvas, p));
        let visuals = ui.visuals();
        let dark = visuals.dark_mode;
        let ink = if dark { Color32::from_gray(235) } else { Color32::from_gray(20) };
        let guide = if dark { Color32::from_gray(70) } else { Color32::from_gray(170) };

        painter.rect_filled(rect, 0.0, visuals.extreme_bg_color);

        // The ground, and the zero line of the off-plane axis.
        let ground_y = to_screen([0.0, 0.0]).y;
        painter.line_segment([Pos2::new(rect.left(), ground_y), Pos2::new(rect.right(), ground_y)], Stroke::new(1.0, guide));
        let zero_x = to_screen([0.0, 0.0]).x;
        painter.line_segment([Pos2::new(zero_x, rect.top()), Pos2::new(zero_x, rect.bottom())], Stroke::new(1.0, guide));

        // The OTHER silhouette, drawn as it is (turned head up for a rig
        // drawn from above, the frame its depths live in).
        if let Some(outline) = self.session.other_outline_shown() {
            for sub in &outline.subpaths {
                let points: Vec<Pos2> = sub.iter().map(|p| to_screen(*p)).collect();
                painter.add(egui::Shape::closed_line(points, Stroke::new(1.0, ink)));
            }
        }

        // Bones, so a limb's depths can be read as a chain.
        let bone_ink = if dark { Color32::from_white_alpha(90) } else { Color32::from_black_alpha(100) };
        for bone in &rig.bones {
            let points: Vec<Pos2> =
                bone.joints.iter().filter_map(|j| model.plot(rig, j)).map(to_screen).collect();
            if points.len() >= 2 {
                painter.line(points, Stroke::new(2.0, bone_ink));
            }
        }

        // Joints: guessed hollow and dim, overridden filled and bright.
        let bright = Color32::from_rgb(255, 210, 0);
        let dim = if dark { Color32::from_gray(150) } else { Color32::from_gray(110) };
        let hover = response.hover_pos().and_then(|p| editor.hit_joint_depth(model, view, canvas, [p.x, p.y]));
        for name in rig.joints.keys() {
            let Some(p) = model.plot(rig, name) else { continue };
            let c = to_screen(p);
            let d = model.joint(rig, name);
            let is_selected = editor.selected() == Some(name.as_str());
            let is_hover = hover.as_deref() == Some(name.as_str());
            let radius = if is_selected || is_hover { 6.0 } else { 4.5 };
            if d.overridden {
                painter.circle(c, radius, bright, Stroke::new(1.5, Color32::BLACK));
            } else {
                painter.circle_stroke(c, radius, Stroke::new(1.5, dim));
            }
            // A guessed joint keeps a faint tick at the guess when it is
            // overridden, so the correction can be judged against it.
            if let (true, Some(g)) = (d.overridden, d.guess) {
                let at = to_screen([g, p[1]]);
                painter.circle_stroke(at, 3.0, Stroke::new(1.0, dim));
                painter.line_segment([at, c], Stroke::new(1.0, dim));
            }
            if is_selected {
                painter.circle_stroke(c, radius + 3.5, Stroke::new(2.0, bright));
            }
        }

        // Hover readout: the joint, its depth, and whether it is a guess.
        if let (Some(p), Some(name)) = (response.hover_pos(), &hover) {
            let d = model.joint(rig, name);
            let tag = if d.overridden { "override" } else { "guess" };
            let text = format!("{name}  {:+.3} ft  ({tag})", d.value);
            let at = p + Vec2::new(14.0, 14.0);
            let galley = painter.layout_no_wrap(text, egui::FontId::proportional(13.0), ink);
            let bg = Rect::from_min_size(at, galley.size()).expand(3.0);
            painter.rect_filled(bg, 3.0, visuals.extreme_bg_color.gamma_multiply(0.85));
            painter.galley(at, galley, ink);
        }
    }

    fn canvas_input(&mut self, ui: &egui::Ui, response: &egui::Response, canvas: &Canvas) {
        use egui::PointerButton::{Middle, Primary};
        let view = &mut self.view;
        let editor = &mut self.session.editor;

        if response.contains_pointer() {
            let (scroll, pinch, at) = ui.input(|i| (i.smooth_scroll_delta().y, i.zoom_delta(), i.pointer.hover_pos()));
            let factor = (scroll * 0.0025).exp() * pinch;
            if let (Some(at), true) = (at, factor != 1.0) {
                view.zoom_at(canvas, [at.x, at.y], factor);
            }
        }

        let space = ui.input(|i| i.key_down(egui::Key::Space));
        if response.dragged_by(Middle) || (space && response.dragged_by(Primary) && !editor.is_dragging()) {
            let d = response.drag_delta();
            view.pan([d.x, d.y]);
            return;
        }

        let world = |p: Pos2| view.screen_to_world(canvas, [p.x, p.y]);
        if response.drag_started_by(Primary) && !space {
            // The press position, not where the pointer is once egui decides
            // it is a drag.
            let origin = ui.input(|i| i.pointer.press_origin()).or(response.interact_pointer_pos());
            if let Some(origin) = origin {
                if let Some(joint) = editor.hit_joint(view, canvas, [origin.x, origin.y]) {
                    editor.begin_drag(&joint, world(origin));
                }
            }
        }
        if editor.is_dragging() {
            if let Some(p) = response.interact_pointer_pos() {
                editor.drag_to(world(p));
            }
            if response.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
                editor.end_drag();
            }
        }

        if response.clicked_by(Primary) && !space {
            if let Some(p) = response.interact_pointer_pos() {
                match editor.hit_joint(view, canvas, [p.x, p.y]) {
                    Some(joint) if editor.chain_pick().is_some() => {
                        editor.pick_joint(&joint);
                    }
                    Some(joint) => editor.select(&joint),
                    // While a chain is being picked a stray click must not
                    // drop the current joint somewhere.
                    None if editor.chain_pick().is_some() => {}
                    None => {
                        editor.place_current(world(p));
                    }
                }
            }
        }
        if response.secondary_clicked() {
            if let Some(p) = response.interact_pointer_pos() {
                if let Some(joint) = editor.hit_joint(view, canvas, [p.x, p.y]) {
                    editor.unplace(&joint);
                }
            }
        }
    }

    fn upload_texture(&mut self, ctx: &egui::Context) {
        let Some(bake) = self.session.last_bake() else {
            self.texture = None;
            return;
        };
        if self.texture.as_ref().is_some_and(|(serial, _)| *serial == bake.serial) {
            return;
        }
        let grid = bake.baked.map.grid;
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [grid.w, grid.h],
            &label_rgba(&bake.baked.map.labels, PREVIEW_ALPHA),
        );
        match &mut self.texture {
            Some((serial, handle)) => {
                handle.set(image, egui::TextureOptions::NEAREST);
                *serial = bake.serial;
            }
            None => {
                let handle = ctx.load_texture("rig_label_map", image, egui::TextureOptions::NEAREST);
                self.texture = Some((bake.serial, handle));
            }
        }
    }

    fn paint(&self, ui: &egui::Ui, painter: &egui::Painter, response: &egui::Response, canvas: &Canvas) {
        let view = &self.view;
        let editor = &self.session.editor;
        let rig = editor.rig();
        let rect = response.rect;
        let to_screen = |p: [f32; 2]| pos(view.world_to_screen(canvas, p));
        let visuals = ui.visuals();
        let dark = visuals.dark_mode;
        let ink = if dark { Color32::from_gray(235) } else { Color32::from_gray(20) };
        let guide = if dark { Color32::from_gray(70) } else { Color32::from_gray(170) };

        painter.rect_filled(rect, 0.0, visuals.extreme_bg_color);

        // Guides: the ground, and (face-on) the mirror line.
        let ground_y = to_screen([0.0, 0.0]).y;
        painter.line_segment([Pos2::new(rect.left(), ground_y), Pos2::new(rect.right(), ground_y)], Stroke::new(1.0, guide));
        if rig.view == View::Front {
            let x = to_screen([0.0, 0.0]).x;
            painter.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], Stroke::new(1.0, guide));
        }

        // The label map, stretched over exactly the world rect it was baked from.
        if let (Some((_, texture)), Some(bake)) = (&self.texture, self.session.last_bake()) {
            let g = bake.baked.map.grid;
            let top_left = to_screen([g.min_x, g.max_y]);
            let size = Vec2::new(g.w as f32, g.h as f32) / g.px_per_ft * view.zoom;
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            painter.image(texture.id(), Rect::from_min_size(top_left, size), uv, Color32::WHITE);
        }

        // The silhouette itself.
        if let Some(outline) = self.session.outline() {
            for sub in &outline.subpaths {
                let points: Vec<Pos2> = sub.iter().map(|p| to_screen(*p)).collect();
                painter.add(egui::Shape::closed_line(points, Stroke::new(1.0, ink)));
            }
        }

        // Hierarchy: a faint link from each parent's pivot to its child's.
        let link = if dark { Color32::from_white_alpha(70) } else { Color32::from_black_alpha(80) };
        for (i, bone) in rig.bones.iter().enumerate() {
            let parent = rig.parent_index(i).and_then(|p| rig.bone_pivot(&rig.bones[p]));
            if let (Some(from), Some(to)) = (parent, rig.bone_pivot(bone)) {
                painter.line_segment([to_screen(from), to_screen(to)], Stroke::new(1.0, link));
            }
        }

        // Bones: dark casing under a part-coloured core so they read on any fill.
        let highlight = Color32::from_rgb(255, 210, 0);
        for (i, bone) in rig.bones.iter().enumerate() {
            let points: Vec<Pos2> = rig.bone_points(bone).into_iter().map(to_screen).collect();
            let is_selected = editor.selected_bone() == Some(i);
            if is_selected {
                // Its pivot, even with a single joint placed.
                if let Some(p) = rig.bone_pivot(bone) {
                    painter.rect_stroke(
                        Rect::from_center_size(to_screen(p), Vec2::splat(15.0)),
                        0.0,
                        Stroke::new(2.0, highlight),
                        egui::StrokeKind::Middle,
                    );
                }
            }
            if points.len() < 2 {
                continue;
            }
            if is_selected {
                painter.line(points.clone(), Stroke::new(9.0, highlight));
            }
            let color = bone
                .part
                .as_deref()
                .and_then(rig_bake::part_index)
                .map_or(Color32::GRAY, |l| label_color(l as usize));
            painter.line(points.clone(), Stroke::new(5.0, Color32::from_black_alpha(200)));
            painter.line(points, Stroke::new(2.5, color));
        }

        // Joints.
        let hover = response.hover_pos().and_then(|p| editor.hit_joint(view, canvas, [p.x, p.y]));
        for (name, p) in &rig.joints {
            let c = to_screen(*p);
            let is_selected = editor.selected() == Some(name.as_str());
            let is_hover = hover.as_deref() == Some(name.as_str());
            let radius = if is_selected || is_hover { 6.0 } else { 4.5 };
            painter.circle(c, radius, Color32::WHITE, Stroke::new(1.5, Color32::BLACK));
            if is_selected {
                painter.circle_stroke(c, radius + 3.5, Stroke::new(2.0, Color32::from_rgb(255, 210, 0)));
            }
        }

        // Eye discs, at the radius the bake used.
        if let Some(r) = self.session.last_bake().and_then(|b| b.baked.eye_radius_ft) {
            let (left, right) = rig.eye_points();
            for p in [left, right].into_iter().flatten() {
                painter.circle_stroke(to_screen(p), (r * view.zoom).max(2.0), Stroke::new(1.0, ink));
            }
        }

        // Anchors and eyes get a small tag so they can be checked at a glance.
        for (tag, joint) in [("head", &rig.anchors.head), ("mouth", &rig.anchors.mouth), ("saddle", &rig.anchors.saddle)] {
            if let Some(p) = rig.joints.get(joint) {
                let c = to_screen(*p) + Vec2::new(9.0, -9.0);
                painter.text(c, egui::Align2::LEFT_BOTTOM, format!("\u{2691} {tag}"), egui::FontId::proportional(11.0), ink);
            }
        }

        // Hover name, or what the next click will place.
        if let Some(p) = response.hover_pos() {
            let picking = editor.chain_pick().is_some();
            let (text, color) = match (&hover, editor.current()) {
                (Some(name), _) if picking => (format!("+ chain: {name}"), ink),
                (Some(name), _) => (name.clone(), ink),
                (None, _) if picking => (String::new(), ink),
                (None, Some(current)) if !editor.is_dragging() => (format!("+ {current}"), guide_text(dark)),
                _ => (String::new(), ink),
            };
            if !text.is_empty() {
                let at = p + Vec2::new(14.0, 14.0);
                let galley = painter.layout_no_wrap(text, egui::FontId::proportional(13.0), color);
                let bg = Rect::from_min_size(at, galley.size()).expand(3.0);
                painter.rect_filled(bg, 3.0, visuals.extreme_bg_color.gamma_multiply(0.85));
                painter.galley(at, galley, color);
            }
        }
    }

    // -------------------------------------------------------------- prompt

    /// Add / Rename: a name box that validates as you type.
    fn name_prompt(&mut self, ctx: &egui::Context) {
        let Some((mut prompt, focus)) = self.prompt.take() else {
            return;
        };
        let editor = &mut self.session.editor;
        let mut done = false;
        let modal = egui::Modal::new(egui::Id::new("rig_name_prompt")).show(ctx, |ui| {
            ui.set_width(320.0);
            ui.heading(prompt.title(editor));
            let edit = ui.add(egui::TextEdit::singleline(&mut prompt.text).char_limit(MAX_NAME_LEN).desired_width(f32::INFINITY));
            if focus {
                edit.request_focus();
            }
            let error = prompt.error(editor);
            match &error {
                Some(why) => ui.colored_label(ui.visuals().warn_fg_color, why),
                None => ui.weak("lower-case letters, digits and _ ; _r / _l endings make a mirror pair"),
            };
            if let Some(offer) = prompt.twin_offer(editor) {
                ui.checkbox(&mut prompt.with_twin, offer);
            }
            ui.add_space(8.0);
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.horizontal(|ui| {
                let ok = ui.add_enabled(error.is_none(), egui::Button::new("OK")).clicked();
                if (ok || enter) && error.is_none() {
                    match prompt.apply(editor) {
                        Ok(()) => done = true,
                        Err(e) => self.session.status = e,
                    }
                }
                done |= ui.button("Cancel").clicked();
            });
        });
        if !done && !modal.should_close() {
            self.prompt = Some((prompt, false));
        }
    }

    /// The pose editor strip: which section to write, the pose file's own
    /// actions, the rows (bones) and the picked row's angles.
    fn pose_editor(&mut self, ui: &mut egui::Ui, family: &str, body_type: &str) {
        let warn = ui.visuals().warn_fg_color;
        if self.pose_doc.as_ref().is_none_or(|d| d.body_type() != body_type) {
            if let Some(other) = self.pose_doc.as_ref().filter(|d| d.is_dirty()).map(|d| d.body_type().to_owned()) {
                ui.colored_label(warn, format!("The {other} poses have unsaved edits."));
                ui.horizontal(|ui| {
                    if ui.button("Save them").clicked() {
                        self.save_poses();
                    }
                    if ui.button("Discard them").clicked() {
                        self.pose_doc = None;
                    }
                });
                return;
            }
            match PoseDoc::open(&self.session.data_root, body_type) {
                Ok(doc) => self.pose_doc = Some(doc),
                Err(e) => {
                    ui.colored_label(warn, e);
                    return;
                }
            }
        }
        if self.puppet_view.pose.is_empty() {
            self.puppet_view.pose = "stand".into();
        }
        let pose = self.puppet_view.pose.clone();
        let mut save = false;
        let mut revert = false;
        {
            let Some(doc) = self.pose_doc.as_mut() else { return };
            ui.horizontal_wrapped(|ui| {
                ui.strong(format!("Pose: {pose}"));
                ui.label("edits");
                ui.selectable_value(&mut self.pose_scope, Scope::Shared, format!("every {body_type}"))
                    .on_hover_text(format!("[pose.{pose}]: the pose every family rigged as {body_type} uses"));
                ui.selectable_value(&mut self.pose_scope, Scope::Family, format!("{family} only")).on_hover_text(format!(
                    "[family.{family}.{pose}]: only the angles {family} does differently; everything else follows the shared pose"
                ));
                if self.pose_scope == Scope::Family
                    && doc.family_has(family, &pose)
                    && ui.button("Clear correction").on_hover_text(format!("{family} follows the shared {pose} again")).clicked()
                {
                    doc.clear_family(family, &pose);
                }
            });
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(doc.can_undo(), egui::Button::new("Undo")).clicked() {
                    doc.undo();
                }
                if ui.add_enabled(doc.can_redo(), egui::Button::new("Redo")).clicked() {
                    doc.redo();
                }
                let dirty = doc.is_dirty();
                save = ui
                    .add_enabled(dirty, egui::Button::new(if dirty { "Save poses *" } else { "Save poses" }))
                    .on_hover_text(format!("Write {} and put it on the stage", doc.path().display()))
                    .clicked();
                revert = ui.add_enabled(dirty, egui::Button::new("Revert")).on_hover_text("Throw away unsaved pose edits").clicked();
                ui.separator();
                ui.add(egui::TextEdit::singleline(&mut self.new_pose_name).hint_text("new pose name").desired_width(110.0));
                if ui.button("New").on_hover_text(format!("A new shared pose, starting as a copy of {pose}")).clicked() {
                    let name = self.new_pose_name.trim().to_owned();
                    if doc.new_pose(&name, Some(&pose)) {
                        self.puppet_view.pose = name;
                        self.new_pose_name.clear();
                    } else {
                        self.session.status = format!("'{name}': taken, or not a name (letters, digits, _)");
                    }
                }
                if ui
                    .add_enabled(pose != "stand", egui::Button::new("Delete"))
                    .on_hover_text(format!("Remove {pose} (and every family's correction of it); Undo brings it back"))
                    .clicked()
                {
                    doc.delete_pose(&pose);
                    self.puppet_view.pose = "stand".into();
                }
            });
        }
        if save {
            self.save_poses();
        }
        if revert {
            self.pose_doc = None;
            return;
        }
        let pose = self.puppet_view.pose.clone();
        let scope = self.pose_scope;
        let Some(doc) = self.pose_doc.as_mut() else { return };
        let rows = match self.session.puppet() {
            Ok(puppet) => pose_edit::rows(puppet, &|stem| doc.pair_is_split(scope, family, &pose, stem)),
            Err(why) => {
                ui.weak(format!("No puppet yet: {why}"));
                return;
            }
        };
        // A row that split or joined: follow it to its new key.
        if let Some(key) = self.pose_row.clone().filter(|k| !rows.iter().any(|r| &r.key == k)) {
            let stem = key.strip_suffix("_l").or_else(|| key.strip_suffix("_r")).unwrap_or(&key).to_owned();
            self.pose_row = [stem.clone(), format!("{stem}_l")].into_iter().find(|k| rows.iter().any(|r| &r.key == k));
        }
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            for row in &rows {
                let (_, origin) = doc.value(scope, family, &pose, &row.key);
                let text = egui::RichText::new(&row.key);
                let text = match origin {
                    Origin::Here => text.strong(),
                    Origin::Shared => text.italics(),
                    Origin::Rest => text.weak(),
                };
                let hover = match origin {
                    Origin::Here => "Set in this pose",
                    Origin::Shared => "From the shared pose",
                    Origin::Rest => "At rest (not named)",
                };
                if ui.selectable_label(self.pose_row.as_ref() == Some(&row.key), text).on_hover_text(hover).clicked() {
                    self.pose_row = Some(row.key.clone());
                }
            }
        });
        let Some(row) = rows.iter().find(|r| Some(&r.key) == self.pose_row.as_ref()) else {
            ui.weak("Pick a bone: click it on the puppet, or its name above.");
            return;
        };
        let (mut entry, origin) = doc.value(scope, family, &pose, &row.key);
        // The sliders stop at the joint's limits (see `[limits]` in the file).
        let limit = if row.key == ROOT { None } else { doc.set_for(family).as_ref().ok().and_then(|s| s.limit(&row.key)) };
        let range = |r: Option<[f32; 2]>| r.map_or(-180.0..=180.0, |[lo, hi]| lo..=hi);
        if row.key != ROOT {
            let n = row.bends.max(entry.bend.len());
            entry.bend.resize(n, 0.0);
        }
        let before = entry.clone();
        let names = if row.key == ROOT {
            ["pitch (x)", "turn (y)", "roll (z)"]
        } else {
            ["swing (x)", "twist (y)", "spread (z)"]
        };
        ui.strong(match row.pair {
            Some(PairState::Joined) => format!("{} (both sides, mirrored)", row.key),
            _ => row.key.clone(),
        });
        let outside = limit.as_ref().is_some_and(|l| !l.allows(&gs_puppet::BonePose::euler(entry.rot, entry.bend.clone())));
        for (k, name) in names.iter().enumerate() {
            let r = range(limit.as_ref().and_then(|l| l.rot[k]));
            ui.add(egui::Slider::new(&mut entry.rot[k], r).step_by(0.5).text(*name));
        }
        for (i, b) in entry.bend.iter_mut().enumerate() {
            let r = range(limit.as_ref().and_then(|l| l.bend.get(i).copied().flatten()));
            ui.add(egui::Slider::new(b, r).step_by(0.5).text(format!("bend {}", i + 1)));
        }
        if outside {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "Outside this joint's limits: it is drawn held at the limit. Move a slider to bring it inside.",
            );
        }
        if entry != before {
            doc.set(scope, family, &pose, &row.key, Some(&entry));
        }
        ui.horizontal_wrapped(|ui| {
            match origin {
                Origin::Here => ui.weak("set in this section"),
                Origin::Shared => ui.weak(format!("the shared value; moving it makes a {family} correction")),
                Origin::Rest => ui.weak("at rest (not in the file)"),
            };
            if ui.add_enabled(origin == Origin::Here, egui::Button::new("Clear")).on_hover_text("Remove this key: back to the shared value or rest").clicked() {
                doc.set(scope, family, &pose, &row.key, None);
            }
            match row.pair {
                Some(PairState::Joined) => {
                    if ui.button("Split sides").on_hover_text("Write left and right separately, to pose them differently").clicked() {
                        doc.split_pair(scope, family, &pose, &row.key);
                        self.pose_row = Some(format!("{}_l", row.key));
                    }
                }
                Some(PairState::Split) => {
                    let stem = row.key.trim_end_matches("_l").trim_end_matches("_r").to_owned();
                    if ui.button("Join sides").on_hover_text("One mirrored entry again (the left side's angles win)").clicked() {
                        doc.join_pair(scope, family, &pose, &stem);
                        self.pose_row = Some(stem);
                    }
                }
                None => {}
            }
        });
        if row.key != ROOT {
            limits_editor(ui, doc, scope, family, row);
        }
        if row.key != ROOT {
            ui.weak("A hanging limb: +swing takes it back, -swing forward; +spread lifts a left limb out. Knees bend +, elbows -.");
        }
    }

    fn save_poses(&mut self) {
        let Some(doc) = self.pose_doc.as_mut() else { return };
        self.session.status = match doc.save() {
            Ok(path) => {
                let path = path.display().to_string();
                self.puppet_view.reload_poses();
                match self.session.install_poses() {
                    Ok(_) => format!("Poses saved to {path} and put on the stage (Publish ships them)"),
                    Err(e) => format!("Poses saved to {path}; {e}"),
                }
            }
            Err(e) => format!("Poses not saved: {e}"),
        };
    }

    fn pose_quit_prompt(&mut self, ctx: &egui::Context) {
        if self.pose_quit != PoseQuit::Asking {
            return;
        }
        let body_type = self.pose_doc.as_ref().map(|d| d.body_type().to_owned()).unwrap_or_default();
        let mut answer = None;
        let modal = egui::Modal::new(egui::Id::new("pose_discard")).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.heading("Unsaved poses");
            ui.label(format!("The {body_type} poses have unsaved edits. Save them before closing?"));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    answer = Some(true);
                }
                if ui.button("Discard").clicked() {
                    answer = Some(false);
                }
                if ui.button("Cancel").clicked() {
                    self.pose_quit = PoseQuit::No;
                }
            });
        });
        if modal.should_close() && answer.is_none() {
            self.pose_quit = PoseQuit::No;
        }
        if let Some(save) = answer {
            if save {
                self.save_poses();
            }
            if !save || !self.poses_dirty() {
                self.pose_quit = PoseQuit::Approved;
                // Carry on closing (the rig's own prompt, if it has one).
                self.approve_quit();
            } else {
                self.pose_quit = PoseQuit::No;
            }
        }
    }

    fn discard_prompt(&mut self, ctx: &egui::Context) {
        let Some(action) = self.session.pending.clone() else {
            return;
        };
        let modal = egui::Modal::new(egui::Id::new("rig_discard")).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.heading("Unsaved changes");
            ui.label(format!(
                "The {} rig has unsaved changes. Discard them and {}?",
                self.session.editor.rig().family,
                action.describe()
            ));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Save first").clicked() {
                    match self.session.save() {
                        Ok(()) => self.session.confirm_pending(),
                        Err(e) => {
                            self.session.status = format!("Save failed: {e}");
                            self.session.cancel_pending();
                        }
                    }
                }
                if ui.button("Discard").clicked() {
                    self.session.confirm_pending();
                }
                if ui.button("Cancel").clicked() {
                    self.session.cancel_pending();
                }
            });
        });
        if modal.should_close() {
            self.session.cancel_pending();
        }
    }
}

/// The joint list's placed / unplaced marker: a filled or a hollow dot,
/// PAINTED. (U+25CF / U+25CB are not in egui's default fonts; as text they
/// drew as empty boxes.)
/// The picked row's joint limits, collapsed until wanted: `[low, high]`
/// degrees per turn angle and per bend, written to `[limits]` (every
/// family) or `[family_limits.<family>]` as the pose scope says. A mirror
/// pair's limits are its stem's, for the left side.
fn limits_editor(ui: &mut egui::Ui, doc: &mut PoseDoc, scope: Scope, family: &str, row: &Row) {
    let key = match row.pair {
        Some(PairState::Split) => row.key.trim_end_matches("_l").trim_end_matches("_r").to_owned(),
        _ => row.key.clone(),
    };
    let (now, origin) = doc.limits(scope, family, &key);
    let title = if now.is_some() { "Joint limits" } else { "Joint limits (none: turns freely)" };
    egui::CollapsingHeader::new(title).id_salt("pose_limits").show(ui, |ui| {
        let mut l = now.clone().unwrap_or_default();
        let range = |ui: &mut egui::Ui, name: &str, r: &mut [f32; 2]| {
            ui.label(name);
            let [lo, hi] = *r;
            ui.add(egui::DragValue::new(&mut r[0]).speed(0.5).range(-180.0..=hi).prefix("from ").suffix(" deg"));
            ui.add(egui::DragValue::new(&mut r[1]).speed(0.5).range(lo..=180.0).prefix("to ").suffix(" deg"));
            ui.end_row();
        };
        let mut turn = l.rot.is_some();
        if ui.checkbox(&mut turn, "Limit its turn").on_hover_text("Bound swing, twist and spread (all three, or none)").changed() {
            l.rot = turn.then_some([FREE; 3]);
        }
        let mut bends = !l.bend.is_empty();
        if row.bends > 0 && ui.checkbox(&mut bends, "Limit its bends").on_hover_text("Bound how far each joint along the chain folds").changed() {
            l.bend = if bends { vec![FREE; row.bends] } else { Vec::new() };
        }
        egui::Grid::new("pose_limits_grid").num_columns(3).show(ui, |ui| {
            if let Some(rot) = l.rot.as_mut() {
                for (r, name) in rot.iter_mut().zip(["swing (x)", "twist (y)", "spread (z)"]) {
                    range(ui, name, r);
                }
            }
            for (i, r) in l.bend.iter_mut().enumerate() {
                range(ui, &format!("bend {}", i + 1), r);
            }
        });
        if !l.bend.is_empty() && l.bend.len() < row.bends && ui.small_button("Limit the next bend too").clicked() {
            l.bend.push(FREE);
        }
        if l != now.clone().unwrap_or_default() {
            let empty = l.rot.is_none() && l.bend.is_empty();
            doc.set_limits(scope, family, &key, (!empty).then_some(&l));
        }
        ui.horizontal_wrapped(|ui| {
            match (origin, scope) {
                (Origin::Here, Scope::Shared) => ui.weak("every family of this body type"),
                (Origin::Here, Scope::Family) => ui.weak(format!("{family}'s own limits")),
                (Origin::Shared, _) => ui.weak(format!("the shared limits; changing them gives {family} its own")),
                (Origin::Rest, _) => ui.weak("not in the file"),
            };
            if ui
                .add_enabled(origin == Origin::Here, egui::Button::new("Clear limits"))
                .on_hover_text("Remove this key's limits from this section")
                .clicked()
            {
                doc.set_limits(scope, family, &key, None);
            }
        });
        if row.pair.is_some() {
            ui.weak(format!("Limits are the left side's ({key}); the right side mirrors them."));
        }
    });
}

/// What a thickness row's checkbox says: "Auto" (ticked = measured) or
/// "Taper" (ticked = set by hand).
#[derive(Clone, Copy, PartialEq)]
enum Toggle {
    Auto,
    Taper,
}

/// Drags in progress / released across a panel's value boxes.
#[derive(Default)]
struct DragState {
    dragging: bool,
    released: bool,
}

/// One thickness override: its checkbox, then the value box when set or
/// the measured value, greyed, when not. Some(new value) on a change; a
/// value switched on starts from what was measured (or `fallback`).
fn thickness_row(ui: &mut egui::Ui, set: Option<f32>, measured: Option<f32>, toggle: Toggle, fallback: f32, drag: &mut DragState) -> Option<Option<f32>> {
    let mut out = None;
    ui.horizontal(|ui| {
        let (mut ticked, text) = match toggle {
            Toggle::Auto => (set.is_none(), "Auto"),
            Toggle::Taper => (set.is_some(), "Taper"),
        };
        if ui.checkbox(&mut ticked, text).changed() {
            let on = if toggle == Toggle::Auto { !ticked } else { ticked };
            out = Some(on.then(|| measured.unwrap_or(fallback)));
        }
        match set {
            Some(mut r) => {
                let d = ui.add(egui::DragValue::new(&mut r).speed(0.005).range(0.01..=30.0).fixed_decimals(2).suffix(" ft"));
                if d.changed() {
                    out = Some(Some(r));
                }
                drag.dragging |= d.dragged();
                drag.released |= d.drag_stopped() || d.lost_focus();
            }
            None => {
                match (toggle, measured) {
                    (Toggle::Taper, _) => ui.weak("off"),
                    (Toggle::Auto, Some(r)) => ui.weak(format!("{r:.2} ft")),
                    (Toggle::Auto, None) => ui.weak("--").on_hover_text("Not measured yet: needs a part, placed joints and a puppet"),
                };
            }
        }
    });
    out
}

fn placed_marker(ui: &mut egui::Ui, placed: bool) {
    let height = ui.text_style_height(&egui::TextStyle::Body);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(10.0, height), Sense::hover());
    let radius = 3.5;
    if placed {
        ui.painter().circle_filled(rect.center(), radius, ui.visuals().text_color());
    } else {
        ui.painter().circle_stroke(rect.center(), radius - 0.5, Stroke::new(1.0, ui.visuals().weak_text_color()));
    }
}

/// The joint list's depth column: the value, a marker saying whether it is a
/// guess or an override, and (for an override) a reset button. Returns the
/// joint whose "reset to guess" was clicked.
fn depth_cell(ui: &mut egui::Ui, editor: &Editor, model: &DepthModel, joint: &str) -> Option<String> {
    if !editor.is_placed(joint) {
        ui.weak("--").on_hover_text("Not placed yet, so it has no depth");
        return None;
    }
    let d = model.joint(editor.rig(), joint);
    let text = format!("{:+.2}", d.value);
    let hover = match (d.overridden, model.guess_source(joint)) {
        (true, Some(how)) => format!("Set by hand. The guess would be {:+.2} ft ({how}).", d.guess.unwrap_or(0.0)),
        (true, None) => "Set by hand. There is nothing to guess from.".to_owned(),
        (false, Some(how)) => format!("A guess: {how}. Drag it in DEPTH view to correct it."),
        (false, None) => "No override and nothing to guess from: it falls back to 0.".to_owned(),
    };
    if d.overridden {
        ui.strong(text).on_hover_text(hover);
        let reset = ui
            .add(egui::Button::new("x").small())
            .on_hover_text(format!("Reset '{joint}' to the guess, so a better guess later still applies"));
        return reset.clicked().then(|| joint.to_owned());
    }
    ui.weak(text).on_hover_text(hover);
    None
}

fn guide_text(dark: bool) -> Color32 {
    if dark {
        Color32::from_gray(170)
    } else {
        Color32::from_gray(90)
    }
}
