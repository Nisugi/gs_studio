//! The rig editor's egui layer. It draws `editor::Session` and forwards
//! input to it; every rule lives in `crate::editor`.

use std::path::Path;

use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};
use rig::{templates, View, AXES, PARTS};

use crate::editor::palette::{label_rgba, LABEL_RGB};
use crate::editor::prompt::{NamePrompt, PromptKind};
use crate::editor::structure::{AnchorSlot, MAX_NAME_LEN};
use crate::editor::warnings::MIN_SHARE;
use crate::editor::{Canvas, DocAction, Session, ViewTransform};

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
    texture: Option<(u64, egui::TextureHandle)>,
    /// A radius drag-value is being dragged: bake small, like a joint drag.
    radius_drag: bool,
    /// Likewise the eye-radius drag-value.
    eye_drag: bool,
    /// The open Add / Rename prompt, and whether its text box still needs focus.
    prompt: Option<(NamePrompt, bool)>,
}

fn label_color(label: usize) -> Color32 {
    let [r, g, b] = LABEL_RGB[label.min(PARTS.len())];
    Color32::from_rgb(r, g, b)
}

fn pos(p: [f32; 2]) -> Pos2 {
    Pos2::new(p[0], p[1])
}

impl RigTool {
    pub fn open(data_root: &Path) -> Result<RigTool, String> {
        Ok(RigTool {
            session: Session::open(data_root)?,
            view: ViewTransform::default(),
            texture: None,
            radius_drag: false,
            eye_drag: false,
            prompt: None,
        })
    }

    pub fn is_dirty(&self) -> bool {
        self.session.editor.is_dirty()
    }

    /// The window wants to close. True = go ahead; false = a discard prompt
    /// is now showing.
    pub fn approve_quit(&mut self) -> bool {
        if !self.session.quit_approved {
            self.session.request(DocAction::Quit);
        }
        self.session.quit_approved
    }

    pub fn quit_approved(&self) -> bool {
        self.session.quit_approved
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
        ui.label(format!("{} / {} / {}", rig.family, rig.view.as_str(), rig.body_type));
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
        let interactive = self.session.editor.is_dragging() || self.radius_drag || self.eye_drag;
        self.session.baked(interactive);

        egui::Panel::left("rig_left").default_size(330.0).min_size(260.0).show(ui, |ui| self.left_panel(ui));
        egui::Panel::right("rig_right").default_size(270.0).min_size(200.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.right_panel(ui));
        });
        egui::CentralPanel::default().show(ui, |ui| self.centre(ui));
        self.discard_prompt(ui.ctx());
        self.name_prompt(ui.ctx());
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
        if pressed(ui, Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) || pressed(ui, Modifiers::COMMAND, Key::Y) {
            self.session.editor.redo();
        } else if pressed(ui, Modifiers::COMMAND, Key::Z) {
            self.session.editor.undo();
        }
        if pressed(ui, Modifiers::COMMAND, Key::S) {
            self.save();
        }
        let typing = ui.ctx().egui_wants_keyboard_input();
        if !typing && ui.input(|i| i.key_pressed(Key::Delete)) {
            self.session.editor.unplace_selected();
        }
        if !typing && ui.input(|i| i.key_pressed(Key::Escape)) {
            self.session.editor.set_chain_pick(None);
        }
    }

    fn save(&mut self) {
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
        self.document_controls(ui);
        ui.separator();
        let spare = ui.available_height();
        let most = (spare - MIN_LIST_HEIGHT).max(MIN_LIST_HEIGHT);
        egui::Panel::bottom("rig_bones_split")
            .resizable(true)
            .frame(egui::Frame::NONE.inner_margin(egui::Margin { top: 6, ..Default::default() }))
            .default_size((spare * BONES_SHARE).clamp(MIN_LIST_HEIGHT, most))
            .size_range(MIN_LIST_HEIGHT..=most)
            .show(ui, |ui| self.bone_list(ui));
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.joint_list(ui));
    }

    fn document_controls(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.session;
        let (family, view, body_type, axis, ref_ft) = {
            let r = s.editor.rig();
            (r.family.clone(), r.view, r.body_type.clone(), r.axis.clone(), r.ref_ft)
        };

        egui::Grid::new("rig_doc").num_columns(2).show(ui, |ui| {
            ui.label("Family");
            let mut open = None;
            egui::ComboBox::from_id_salt("family").selected_text(&family).width(170.0).show_ui(ui, |ui| {
                for f in s.outlines.families() {
                    let text = if s.has_saved_rig(&f) { format!("{f}  (saved)") } else { f.clone() };
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
                if let Err(e) = s.save() {
                    s.status = format!("Save failed: {e}");
                }
            }
            if ui.button("New from template").on_hover_text("Un-place everything and reset the bones").clicked() {
                s.request(DocAction::NewFromTemplate { template: body_type.clone(), view });
            }
        });
        ui.horizontal(|ui| {
            let target = s.publish_target();
            let saved = s.has_saved_bake(&family);
            let button = ui.add_enabled(target.is_ok() && saved, egui::Button::new("Publish"));
            let button = match &target {
                Ok(dir) if saved => button.on_hover_text(format!(
                    "Copy the SAVED {family}.png + {family}.toml to {} for game clients to embed.\n\
                     Unsaved edits are not included.",
                    dir.display()
                )),
                Ok(_) => button.on_disabled_hover_text("Nothing to publish yet: save this rig first"),
                Err(why) => button.on_disabled_hover_text(why),
            };
            if button.clicked() {
                if let Err(e) = s.publish() {
                    s.status = format!("Publish failed: {e}");
                }
            }
            ui.add_enabled(target.is_ok(), egui::Checkbox::new(&mut s.publish_on_save, "publish on save"))
                .on_hover_text("Save also copies the baked map + metadata to the publish directory")
                .on_disabled_hover_text(target.err().unwrap_or_default());
        });
        ui.horizontal(|ui| {
            if ui.add_enabled(s.editor.can_undo(), egui::Button::new("Undo")).clicked() {
                s.editor.undo();
            }
            if ui.add_enabled(s.editor.can_redo(), egui::Button::new("Redo")).clicked() {
                s.editor.redo();
            }
        });
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
        });
        // Above the list, so the list can take every pixel that is left.
        match &current {
            Some(j) => ui.weak(format!("Next click places: {j}")),
            None => ui.weak("All joints placed. Click a joint in the list to re-place it."),
        };
        egui::ScrollArea::vertical().id_salt("joints").auto_shrink([false, false]).show(ui, |ui| {
            for joint in &order {
                let is_current = current.as_deref() == Some(joint);
                let placed = editor.is_placed(joint);
                let mut text = egui::RichText::new(joint);
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
                        ui.selectable_label(is_current, text)
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
            if let Some(bone) = self.session.editor.selected_bone() {
                ui.separator();
                self.bone_details(ui, bone);
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

    /// The selected bone: parent, pivot, and its joint chain.
    fn bone_details(&mut self, ui: &mut egui::Ui, index: usize) {
        let pixels = self.session.last_bake().and_then(|b| b.baked.bone_pixels.get(index).copied());
        let editor = &mut self.session.editor;
        let Some(bone) = editor.rig().bones.get(index).cloned() else {
            return;
        };
        ui.horizontal(|ui| {
            ui.strong(&bone.name);
            if let Some(n) = pixels {
                ui.weak(format!("{n} px"));
            }
        });
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

        ui.label("Chain");
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
    }

    // --------------------------------------------------------------- right

    fn right_panel(&mut self, ui: &mut egui::Ui) {
        let Some(bake) = self.session.last_bake() else {
            ui.label("No silhouette for this family / view.");
            return;
        };
        let baked = &bake.baked;
        ui.strong("Parts");
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
                    ui.colored_label(ui.visuals().warn_fg_color, text);
                } else {
                    ui.monospace(text);
                }
                ui.end_row();
            }
        });

        ui.separator();
        let eye_used = baked.eye_radius_ft;
        self.anchors_and_eyes(ui, eye_used);
        let Some(bake) = self.session.last_bake() else {
            return;
        };
        let baked = &bake.baked;

        ui.separator();
        ui.strong("Map");
        let grid = baked.map.grid;
        ui.label(format!("{} x {} px at {:.1} px/ft", grid.w, grid.h, baked.px_per_ft));
        ui.label(format!("aspect {:.3}   span {:.3}", baked.metrics.aspect, baked.metrics.span));
        ui.label(format!("ground at {:.1}% down", baked.ground_frac * 100.0));
        ui.label(format!("orphan pixels: {}", baked.map.orphans));

        ui.separator();
        ui.strong(format!("Warnings ({})", bake.warnings.len()));
        if bake.warnings.is_empty() {
            ui.weak("none");
        }
        for w in &bake.warnings {
            ui.horizontal_wrapped(|ui| ui.colored_label(ui.visuals().warn_fg_color, format!("\u{26A0} {w}")));
        }
    }

    fn anchors_and_eyes(&mut self, ui: &mut egui::Ui, eye_used: Option<f32>) {
        let editor = &mut self.session.editor;
        let joints = editor.joint_order();
        let side = editor.rig().view == View::Side;
        ui.strong("Anchors and eyes");
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
        let mut fit = std::mem::take(&mut self.session.fit_requested);
        ui.horizontal(|ui| {
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

fn guide_text(dark: bool) -> Color32 {
    if dark {
        Color32::from_gray(170)
    } else {
        Color32::from_gray(90)
    }
}
