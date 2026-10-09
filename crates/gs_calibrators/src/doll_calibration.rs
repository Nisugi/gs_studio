//! Injury doll calibrator: click the assigned pool doll image to place
//! each body part's dot anchor, style the generated wound/scar dots, and
//! save the result into the doll's sidecar toml (calibration travels with
//! the artwork). Saved coordinates are fractions of the base image, so
//! they hold at any window size or zoom.

//!
//! Ported from VellumFE `src/frontend/gui/app/editors/doll_calibration.rs`
//! @60c84a79 (GPL-3.0-or-later). The original was two `impl VellumGuiApp`
//! methods reading `self.skin_state`; here it has the same
//! `open(..) -> (Option<Self>, CalibrationOutcome)` / `ui(..) ->
//! CalibrationOutcome` shape as its siblings, with what it took from the
//! skin state turned into explicit inputs. The window body and the sidecar
//! write are unchanged. See PORTING.md.

use std::collections::HashMap;

use crate::color::color_field;
use crate::skin_paint::{self as gui_skin, ResolvedDotStyle, SkinTexture};
use crate::CalibrationOutcome;
use gs_field::skins::{self, DollDotSpec, DOLL_PARTS};

/// Which pool doll to calibrate: what VellumFE read from
/// `SkinState::doll_override()` / `doll_override_abs_path()`.
#[derive(Debug, Clone)]
pub struct DollCalibrationTarget {
    /// Pool-relative path ("dolls/x.png"); the window title and messages
    /// use its file stem.
    pub pool_path: String,
    /// Absolute path of that image; the sidecar is read and written beside it.
    pub image_abs: std::path::PathBuf,
}

impl DollCalibrationTarget {
    /// Resolve a pool-relative path against the shared image pool, exactly
    /// as `SkinState::doll_override_abs_path` did (its root was
    /// `global_images_dir().unwrap_or_default()`).
    pub fn from_pool_path(pool_path: &str) -> Self {
        let root = gs_field::pool_dirs::global_images_dir().unwrap_or_default();
        Self {
            pool_path: pool_path.to_owned(),
            image_abs: skins::resolve_image_path(&root, pool_path),
        }
    }
}

/// NEW (not in VellumFE, where the target came from the injuries window's
/// Appearance menu): pool-relative paths of every calibratable doll, layer
/// files excluded, for a host-side picker. Same filter as the Doll Sets
/// workbench's scan.
pub fn pool_dolls() -> Vec<String> {
    gs_field::pool::list_category("dolls")
        .iter()
        .filter(|image| !gs_field::pool::is_doll_layer_file(&image.pool_path))
        .map(|image| image.pool_path.clone())
        .collect()
}

/// Where the calibration saves: a pool doll's sidecar toml.
struct CalibrationTarget {
    image_abs: std::path::PathBuf,
    /// Pool-relative path (texture debug name for a self-loaded base).
    pool_path: String,
    /// Display name (image stem) for the window title and messages.
    label: String,
}

impl CalibrationTarget {
    fn label(&self) -> &str {
        &self.label
    }
}

pub struct DollCalibrationState {
    /// Where Save writes.
    target: CalibrationTarget,
    /// Working anchors keyed by lowercase protocol part name. Only parts
    /// the sidecar (or this session) has placed appear here; the rest
    /// render at built-in defaults and stay implicit in the saved file.
    anchors: HashMap<String, [f32; 2]>,
    /// Index into DOLL_PARTS of the part the next click places.
    selected: usize,
    /// Jump to the next part after each click.
    auto_advance: bool,
    wound_color: String,
    scar_color: String,
    opacity: f32,
    diameter: f32,
    /// Preview dots as scars (rings) instead of wounds (solid).
    preview_scars: bool,
    /// Preview severity numeral, 1-3.
    preview_level: u8,
    error: Option<String>,
    /// Base image loaded by [`Self::ui`] when the host supplies none.
    own_base: Option<egui::TextureHandle>,
    /// A self-load was tried (a failure is not retried every frame).
    own_base_tried: bool,
}

impl DollCalibrationState {
    fn dot_spec(&self) -> DollDotSpec {
        DollDotSpec {
            wound_color: self.wound_color.trim().to_string(),
            scar_color: self.scar_color.trim().to_string(),
            opacity: self.opacity,
            diameter: self.diameter,
        }
    }

    fn anchor_for(&self, part_key: &str) -> egui::Vec2 {
        let key = part_key.to_ascii_lowercase();
        self.anchors
            .get(&key)
            .copied()
            .or_else(|| skins::default_doll_anchor(&key))
            .map(|[x, y]| egui::vec2(x, y))
            .unwrap_or(egui::vec2(0.5, 0.5))
    }
}

impl DollCalibrationState {
    /// `target` is the doll to calibrate (VellumFE: the injuries window's
    /// assigned doll). `None` yields the original "No doll image selected"
    /// message and no state.
    pub fn open(target: Option<DollCalibrationTarget>) -> (Option<Self>, CalibrationOutcome) {
        let mut outcome = CalibrationOutcome::default();
        // Calibration targets the assigned pool doll's sidecar.
        let Some(DollCalibrationTarget {
            pool_path,
            image_abs: abs,
        }) = target
        else {
            outcome.messages.push(
                "No doll image selected. Pick one in the injuries window's Appearance menu \
                 (install some with .jinx)."
                    .to_owned(),
            );
            return (None, outcome);
        };
        let sidecar: gs_field::pool::DollSidecar =
            gs_field::pool::read_sidecar(&abs).unwrap_or_default();
        let label = pool_path
            .rsplit_once('/')
            .map(|(_, file)| file)
            .unwrap_or(&pool_path)
            .rsplit_once('.')
            .map(|(stem, _)| stem.to_owned())
            .unwrap_or(pool_path.clone());
        let target = CalibrationTarget {
            image_abs: abs,
            pool_path: pool_path.clone(),
            label,
        };
        let (anchors, dots) = (sidecar.anchors, sidecar.dots);
        let anchors = anchors
            .iter()
            .map(|(part, anchor)| (part.to_ascii_lowercase(), *anchor))
            .collect();
        let state = DollCalibrationState {
            target,
            anchors,
            selected: 0,
            auto_advance: true,
            wound_color: dots.wound_color,
            scar_color: dots.scar_color,
            opacity: dots.opacity,
            diameter: dots.diameter,
            preview_scars: false,
            preview_level: 2,
            error: None,
            own_base: None,
            own_base_tried: false,
        };
        (Some(state), outcome)
    }

    /// The base image, loaded once by this state from the target's file
    /// (hosts without a skin system: gs-studio).
    fn own_base(&mut self, ctx: &egui::Context) -> Option<SkinTexture> {
        if !self.own_base_tried {
            self.own_base_tried = true;
            self.own_base = gs_field_egui::image_store::load_texture_file(
                ctx,
                &self.target.image_abs,
                &format!("doll-cal:{}", self.target.pool_path),
                "doll calibration",
            );
        }
        self.own_base.as_ref().map(|handle| SkinTexture {
            texture: handle.id(),
            size: handle.size_vec2(),
        })
    }

    /// Floating pop-out over a self-loaded base. See [`Self::ui_with_base`].
    pub fn ui(&mut self, ctx: &egui::Context) -> CalibrationOutcome {
        let base = self.own_base(ctx);
        self.ui_with_base(ctx, base)
    }

    /// Tab/panel body over a self-loaded base. See
    /// [`Self::ui_contents_with_base`].
    pub fn ui_contents(&mut self, ui: &mut egui::Ui) -> CalibrationOutcome {
        let base = self.own_base(&ui.ctx().clone());
        self.ui_contents_with_base(ui, base)
    }

    /// VellumFE's floating pop-out: the ORIGINAL `egui::Window` (same
    /// title, id, order, sizes, X button) around
    /// [`Self::ui_contents_with_base`]. `base` is the texture the HOST loaded
    /// (VellumFE: `skin_state.widget_art().and_then(|art|
    /// art.doll_set(None).base)`); `None` closes the calibrator with the
    /// original message, without showing a window (as the original did).
    pub fn ui_with_base(
        &mut self,
        ctx: &egui::Context,
        base: Option<SkinTexture>,
    ) -> CalibrationOutcome {
        if base.is_none() {
            return self.base_vanished();
        }
        let mut outcome = CalibrationOutcome::default();
        let mut open = true;
        egui::Window::new(format!("Injury Doll Calibration - {}", self.target.label()))
            .id(egui::Id::new("gui_doll_calibration"))
            .order(egui::Order::Foreground)
            .open(&mut open)
            .default_width(560.0)
            .default_height(520.0)
            .resizable(true)
            .show(ctx, |ui| {
                outcome = self.ui_contents_with_base(ui, base);
            });
        outcome.closed = !open;
        outcome
    }

    fn base_vanished(&self) -> CalibrationOutcome {
        let mut outcome = CalibrationOutcome::default();
        outcome.messages.push(
            "Injury doll calibration closed: the selected doll set has no base image loaded."
                .to_owned(),
        );
        outcome.closed = true;
        outcome
    }

    /// The calibrator's whole UI drawn into a caller-supplied region (a tab,
    /// a panel): no window of its own, the canvas takes the room that is
    /// left. Sets `closed` in ONE case, inherited from the original: `base`
    /// is `None` (the doll's image is gone), where the host should drop the
    /// state.
    pub fn ui_contents_with_base(
        &mut self,
        ui: &mut egui::Ui,
        base: Option<SkinTexture>,
    ) -> CalibrationOutcome {
        let mut outcome = CalibrationOutcome::default();
        let state = self;
        // The base can vanish mid-session (doll override cleared); close
        // rather than calibrating against nothing.
        let Some(base) = base else {
            return state.base_vanished();
        };

        let mut save_request = false;

        (|ui: &mut egui::Ui| {
                ui.label("Click the doll to place the highlighted part's dot. Coordinates are stored as fractions of the image, so they hold at any size.");
                ui.separator();
                ui.horizontal_top(|ui| {
                    // Part list: click-through order, dot marker for parts
                    // with a calibrated (non-default) anchor.
                    ui.vertical(|ui| {
                        ui.set_width(150.0);
                        for (index, (key, display, _)) in DOLL_PARTS.iter().enumerate() {
                            let calibrated =
                                state.anchors.contains_key(&key.to_ascii_lowercase());
                            let label = if calibrated {
                                format!("{display} \u{2022}")
                            } else {
                                display.to_string()
                            };
                            if ui
                                .selectable_label(state.selected == index, label)
                                .clicked()
                            {
                                state.selected = index;
                            }
                        }
                        ui.add_space(6.0);
                        ui.checkbox(&mut state.auto_advance, "Auto-advance")
                            .on_hover_text("Jump to the next part after each click");
                        if ui
                            .button("Use default")
                            .on_hover_text(
                                "Drop the selected part's calibration and fall back to the built-in position",
                            )
                            .clicked()
                        {
                            let key = DOLL_PARTS[state.selected].0.to_ascii_lowercase();
                            state.anchors.remove(&key);
                        }
                    });

                    // Doll canvas: base image aspect-fit into whatever space
                    // is left, every part's dot drawn live at preview
                    // severity, the selected part cross-haired. Reserve room
                    // below for the style rows and the Save button so the
                    // canvas can't push them past the window's bottom edge.
                    const CONTROLS_HEIGHT: f32 = 130.0;
                    let avail = ui.available_size();
                    let canvas = egui::Vec2::new(
                        avail.x.max(160.0),
                        (avail.y - CONTROLS_HEIGHT).max(200.0),
                    );
                    let (rect, response) =
                        ui.allocate_exact_size(canvas, egui::Sense::click());
                    let painter = ui.painter().with_clip_rect(rect);
                    painter.rect_filled(
                        rect,
                        4.0,
                        ui.visuals().extreme_bg_color,
                    );
                    let dest = gui_skin::sprite_dest(&base, rect);
                    gui_skin::paint_sprite(&painter, dest, &base, egui::Color32::WHITE);

                    let style = ResolvedDotStyle::from_spec(&state.dot_spec());
                    let level = state.preview_level.clamp(1, 3)
                        + if state.preview_scars { 3 } else { 0 };
                    let radius = (style.diameter * dest.height() / 2.0).max(4.0);
                    let at = |anchor: egui::Vec2| {
                        dest.min
                            + egui::Vec2::new(anchor.x * dest.width(), anchor.y * dest.height())
                    };
                    for (key, _, _) in DOLL_PARTS {
                        gui_skin::paint_severity_dot(
                            &painter,
                            at(state.anchor_for(key)),
                            radius,
                            level,
                            &style,
                        );
                    }

                    // Selected-part crosshair on top of its dot.
                    let highlight = ui.visuals().hyperlink_color;
                    let center = at(state.anchor_for(DOLL_PARTS[state.selected].0));
                    let stroke = egui::Stroke::new(1.0, highlight);
                    painter.line_segment(
                        [
                            egui::pos2(dest.min.x, center.y),
                            egui::pos2(dest.max.x, center.y),
                        ],
                        stroke,
                    );
                    painter.line_segment(
                        [
                            egui::pos2(center.x, dest.min.y),
                            egui::pos2(center.x, dest.max.y),
                        ],
                        stroke,
                    );
                    painter.circle_stroke(
                        center,
                        radius + 3.0,
                        egui::Stroke::new(2.0, highlight),
                    );

                    if response.clicked() {
                        if let Some(pos) = response.interact_pointer_pos() {
                            if dest.contains(pos) && dest.width() > 0.0 && dest.height() > 0.0 {
                                let normalized = [
                                    ((pos.x - dest.min.x) / dest.width()).clamp(0.0, 1.0),
                                    ((pos.y - dest.min.y) / dest.height()).clamp(0.0, 1.0),
                                ];
                                let key = DOLL_PARTS[state.selected].0.to_ascii_lowercase();
                                state.anchors.insert(key, normalized);
                                if state.auto_advance {
                                    state.selected = (state.selected + 1) % DOLL_PARTS.len();
                                }
                            }
                        }
                    }
                });

                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Wound");
                    color_field(ui, &mut state.wound_color);
                    ui.label("Scar");
                    color_field(ui, &mut state.scar_color);
                    ui.label("Preview:");
                    ui.selectable_value(&mut state.preview_scars, false, "wounds");
                    ui.selectable_value(&mut state.preview_scars, true, "scars");
                    ui.add(
                        egui::Slider::new(&mut state.preview_level, 1..=3).text("rank"),
                    );
                });
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Slider::new(&mut state.diameter, 0.02..=0.20)
                            .text("dot size")
                            .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                    );
                    ui.add(
                        egui::Slider::new(&mut state.opacity, 0.2..=1.0)
                            .text("opacity")
                            .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                    );
                });

                ui.separator();
                ui.horizontal(|ui| {
                    let (save_label, save_hover) = (
                        "Save",
                        "Writes anchors and dot styling into the doll's sidecar toml, next to the image — the calibration travels with the artwork",
                    );
                    if ui.button(save_label).on_hover_text(save_hover).clicked() {
                        save_request = true;
                    }
                    if ui
                        .button("Reset all to defaults")
                        .clicked()
                    {
                        state.anchors.clear();
                        let defaults = DollDotSpec::default();
                        state.wound_color = defaults.wound_color;
                        state.scar_color = defaults.scar_color;
                        state.opacity = defaults.opacity;
                        state.diameter = defaults.diameter;
                    }
                });
                if let Some(error) = &state.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
        })(ui);

        if save_request {
            let result = gs_field::pool::write_doll_sidecar(
                &state.target.image_abs,
                &state.anchors,
                &state.dot_spec(),
            );
            match result {
                Ok(()) => {
                    state.error = None;
                    // Sidecars aren't mtime-polled (and the skin poll takes a
                    // second); force a reload so the live doll updates on the
                    // very next frame.
                    outcome.reload_art = true;
                    outcome.messages.push(format!(
                        "Injury doll calibration saved to '{}'.",
                        state.target.label()
                    ));
                }
                Err(err) => {
                    state.error = Some(format!("Failed to save: {}", err));
                }
            }
        }

        outcome
    }
}
