//! The Frames tab: a COMPOSER of frame recipes over the parts library
//! (`frame_recipe`; the design agreed with Hydra's side on 2026-10-11).
//! A frame is a recipe TOML naming parts: an edge, a sticker for each kind
//! of junction, ornament rules. The tab lists the library's parts as
//! thumbnails, each click puts one in the chosen slot, the recipe's
//! numbers sit beside, and the preview assembles a window, a row of two
//! cells and a 2x2 grid at real point sizes (1x or 2x, rotate or mirror),
//! or tiles the edge three times for the seam. Save validates and writes
//! `frames/<name>.toml`; Export pack saves, then asks the host to pack the
//! recipe for Hydra (`export_frame` in the outcome, `frame_recipe::write_pack_files`).
//! The parts come from the pod
//! (`image-gen/flux2_frames.py`) through `tools/frames_ingest.py --painted`.
//!
//! The earlier per-folder calibrator (five pieces cut by hand) is gone;
//! `frame_template` keeps the turning, the placement types and the keying
//! this composer still uses.

use std::collections::HashMap;

use crate::color::color_field;
use crate::frame_recipe::{self as fr, Effects, Emissive, Orient, OrnamentRule, PartSizes, Place, Recipe};
use crate::frame_template::{EdgeMode, Plan, Turn};
use crate::skin_paint::parse_hex_rgb;
use crate::CalibrationOutcome;

/// Which slot a library click fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Edge,
    Corner,
    Tee,
    Cross,
    Ornament(usize),
}

impl Slot {
    fn kind(self) -> &'static str {
        match self {
            Slot::Edge => "edges",
            Slot::Corner | Slot::Tee | Slot::Cross => "stickers",
            Slot::Ornament(_) => "ornaments",
        }
    }
}

/// Which assembly the preview shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Window,
    Row,
    Grid,
    Seam,
}

const VIEWS: [(View, &str, &str); 4] = [
    (View::Window, "Window", "One window"),
    (View::Row, "Row", "Two cells across: tees top and bottom"),
    (View::Grid, "Grid", "2x2: tees all round and a cross"),
    (View::Seam, "Seam", "The edge tiled three times: the joins"),
];

/// A library part on the GPU: its full picture (for the preview) and its
/// pixel size.
struct PartTex {
    tex: egui::TextureHandle,
    size: [f32; 2],
}

/// A part being trimmed: its cut box on its raw picture.
struct Trim {
    kind: &'static str,
    name: String,
    raw: Option<PartTex>,
    cut: [f32; 4],
    /// Some(true): sizing the box; Some(false): moving it; None: panning or nothing.
    drag: Option<bool>,
    zoom: f32,
    pan: egui::Vec2,
}

pub struct FrameCalibrationState {
    recipes: Vec<String>,
    /// The library's part names, by kind.
    library: HashMap<&'static str, Vec<String>>,
    recipe: Recipe,
    /// The recipe's name (the file's stem).
    name: String,
    slot: Slot,
    /// The library kind shown in the browser.
    browse: &'static str,
    filter: String,
    /// Parts loaded: by (kind, name). Thumbnails are the same textures,
    /// drawn small.
    parts: HashMap<(&'static str, String), Option<PartTex>>,
    /// Emissive blooms: by (kind, name, colour, tolerance in hundredths, radius in part pixels).
    blooms: HashMap<(&'static str, String, [u8; 3], u32, u32), Option<PartTex>>,
    /// The part being trimmed, with its cut box on its raw picture, and
    /// the raw's texture.
    trim: Option<Trim>,
    view: View,
    zoom: f32,
    /// The preview's pan, screen points.
    pan: egui::Vec2,
    /// Draw the window's bound, the rails' centrelines and the stickers' squares over the preview.
    guides: bool,
    problems: Vec<fr::Problem>,
    error: Option<String>,
    note: Option<String>,
    dirty: bool,
}

impl FrameCalibrationState {
    /// Open the composer: `initial` names a recipe to start on. `None` when
    /// the pool has no `frames/` folder at all.
    pub fn open(initial: Option<String>) -> (Option<Self>, CalibrationOutcome) {
        let mut outcome = CalibrationOutcome::default();
        if !fr::frames_dir().is_some_and(|d| d.is_dir()) {
            outcome.messages.push("No frames in the pool (global/images/frames/): ingest parts there (tools/frames_ingest.py --painted) then Rescan pool.".to_owned());
            return (None, outcome);
        }
        let mut state = FrameCalibrationState {
            recipes: fr::list_recipes(),
            library: fr::PART_KINDS.iter().map(|k| (*k, fr::list_parts(k))).collect(),
            recipe: Recipe::default(),
            name: "new_frame".to_owned(),
            slot: Slot::Edge,
            browse: "edges",
            filter: String::new(),
            parts: HashMap::new(),
            blooms: HashMap::new(),
            trim: None,
            view: View::Window,
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
            guides: false,
            problems: Vec::new(),
            error: None,
            note: None,
            dirty: false,
        };
        let want = initial.as_deref().map(str::trim).filter(|w| !w.is_empty());
        let first = want.and_then(|w| state.recipes.iter().find(|r| r.eq_ignore_ascii_case(w)).cloned()).or_else(|| state.recipes.first().cloned());
        match first {
            Some(name) => state.load(&name),
            None => {
                // A fresh recipe on the first parts there are.
                state.recipe.edge = state.library["edges"].first().cloned().unwrap_or_default();
                let sticker = state.library["stickers"].first().cloned();
                state.recipe.corner = sticker.clone();
                state.recipe.tee = sticker.clone();
                state.recipe.cross = sticker;
                if let Some(o) = state.library["ornaments"].first().cloned() {
                    state.recipe.ornaments.push(OrnamentRule { image: o, ..Default::default() });
                }
                state.revalidate();
            }
        }
        if state.library.values().all(|v| v.is_empty()) {
            outcome.messages.push("The parts library is empty: ingest parts with tools/frames_ingest.py --painted <hires_test/frames>.".to_owned());
        }
        (Some(state), outcome)
    }

    pub fn has_unsaved_changes(&self) -> bool {
        self.dirty
    }

    fn load(&mut self, name: &str) {
        match fr::recipe_path(name).ok_or_else(|| "no pool".to_owned()).and_then(|p| Recipe::read(&p)) {
            Ok(r) => {
                self.recipe = r;
                self.name = name.to_owned();
                self.dirty = false;
                self.error = None;
                self.note = None;
            }
            Err(e) => self.error = Some(e),
        }
        self.revalidate();
    }

    fn revalidate(&mut self) {
        self.problems = fr::validate(&self.recipe, |kind, name| fr::load_part(kind, name));
    }

    /// The part's texture, loaded on first use (None when the file is missing).
    fn part(&mut self, ctx: &egui::Context, kind: &'static str, name: &str) -> Option<(egui::TextureId, [f32; 2])> {
        let key = (kind, name.to_owned());
        if !self.parts.contains_key(&key) {
            let loaded = fr::load_part(kind, name).map(|img| {
                let color = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                PartTex { tex: ctx.load_texture(format!("frame-part:{kind}/{name}"), color, egui::TextureOptions::LINEAR), size: [img.width() as f32, img.height() as f32] }
            });
            self.parts.insert(key.clone(), loaded);
        }
        self.parts.get(&key).and_then(|p| p.as_ref()).map(|p| (p.tex.id(), p.size))
    }

    /// The emissive bloom of a part at a setting, uploaded premultiplied so it adds.
    fn bloom(&mut self, ctx: &egui::Context, kind: &'static str, name: &str, e: &Emissive, radius_px: f32) -> Option<egui::TextureId> {
        let color = fr::hex_rgb(&e.color).unwrap_or([255, 106, 32]);
        let key = (kind, name.to_owned(), color, (e.tolerance * 100.0).round() as u32, radius_px.round().max(1.0) as u32);
        if !self.blooms.contains_key(&key) {
            let loaded = fr::load_part(kind, name).map(|img| {
                let b = fr::emission_bloom(&img, color, e.tolerance, key.4 as f32);
                let color_img = egui::ColorImage::from_rgba_premultiplied([b.width() as usize, b.height() as usize], b.as_raw());
                PartTex { tex: ctx.load_texture(format!("frame-bloom:{kind}/{name}@{:?}", key), color_img, egui::TextureOptions::LINEAR), size: [img.width() as f32, img.height() as f32] }
            });
            self.blooms.insert(key.clone(), loaded);
        }
        self.blooms.get(&key).and_then(|p| p.as_ref()).map(|p| p.tex.id())
    }

    fn sizes(&mut self, ctx: &egui::Context) -> PartSizes {
        let r = self.recipe.clone();
        let mut of = |kind: &'static str, name: &Option<String>| name.as_deref().and_then(|n| self.part(ctx, kind, n)).map(|(_, s)| s);
        PartSizes {
            edge: of("edges", &Some(r.edge.clone()).filter(|e| !e.is_empty())),
            corner: of("stickers", &r.corner),
            tee: of("stickers", &r.tee),
            cross: of("stickers", &r.cross),
            ornaments: r.ornaments.iter().map(|o| of("ornaments", &Some(o.image.clone()))).collect(),
        }
    }

    /// Floating pop-out (the calibrators demo).
    pub fn ui(&mut self, ctx: &egui::Context) -> CalibrationOutcome {
        let mut outcome = CalibrationOutcome::default();
        let mut open = true;
        egui::Window::new("Frame Composer")
            .id(egui::Id::new("gui_frame_calibration"))
            .order(egui::Order::Foreground)
            .open(&mut open)
            .default_width(1000.0)
            .default_height(680.0)
            .resizable(true)
            .show(ctx, |ui| outcome = self.ui_contents(ui));
        outcome.closed = !open;
        outcome
    }

    /// The whole tab. Never sets `closed`.
    pub fn ui_contents(&mut self, ui: &mut egui::Ui) -> CalibrationOutcome {
        let mut outcome = CalibrationOutcome::default();
        let mut load: Option<String> = None;
        let mut save = false;
        let mut export = false;
        let before = self.recipe.clone();

        // The recipe strip.
        ui.horizontal_wrapped(|ui| {
            ui.label("Recipe:");
            egui::ComboBox::from_id_salt("frame_recipe").selected_text(self.name.clone()).width(200.0).show_ui(ui, |ui| {
                for r in &self.recipes {
                    if ui.selectable_label(*r == self.name, r).clicked() {
                        load = Some(r.clone());
                    }
                }
            });
            if ui.button("New").on_hover_text("Start a recipe from the current one under a new name").clicked() {
                self.name = format!("{}_copy", self.name);
                self.dirty = true;
            }
            ui.label("Save as:");
            ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(160.0));
            if ui.add_enabled(fr::exportable(&self.problems), egui::Button::new("Save recipe")).on_hover_text("Writes frames/<name>.toml, which Hydra reads; an error blocks, a warning does not").clicked() {
                save = true;
            }
            if ui
                .add_enabled(fr::exportable(&self.problems), egui::Button::new("Export pack"))
                .on_hover_text(format!(
                    "Saves the recipe, then packs it with the parts it names for Hydra: packs/{}.zip and its line in packs/packs.toml, the version raised when the pack changed. Parts are scaled down to {} px a point at {} pt thick; the library keeps them as painted",
                    fr::pack_id(&self.name),
                    fr::PACK_PX_PER_PT,
                    fr::PACK_MAX_THICKNESS
                ))
                .clicked()
            {
                save = true;
                export = true;
            }
            if self.dirty {
                ui.weak("unsaved");
            }
        });

        // The recipe's numbers and slots.
        ui.horizontal_wrapped(|ui| {
            ui.label("thickness");
            ui.add(egui::DragValue::new(&mut self.recipe.thickness).range(1.0..=48.0).suffix(" pt"));
            ui.label("turn");
            ui.selectable_value(&mut self.recipe.turn, Turn::Rotate, "rotate").on_hover_text("90, 180, 270 clockwise, stickers and ornaments alike");
            ui.selectable_value(&mut self.recipe.turn, Turn::Mirror, "mirror").on_hover_text("left, right and bottom are reflections");
            ui.label("edge run");
            ui.selectable_value(&mut self.recipe.run, EdgeMode::Tile, "tile");
            ui.selectable_value(&mut self.recipe.run, EdgeMode::Fit, "fit").on_hover_text("A whole number of tiles between the corner stickers, each stretched a little");
            ui.selectable_value(&mut self.recipe.run, EdgeMode::Mirror, "mirror").on_hover_text("Every other tile reflected, so each join meets its own mirror image: no seam on any edge");
            ui.selectable_value(&mut self.recipe.run, EdgeMode::Stretch, "stretch");
            ui.label("edge inset");
            ui.add(egui::DragValue::new(&mut self.recipe.edge_inset).range(0.0..=96.0).speed(0.25).suffix(" pt")).on_hover_text("How far the rails sit in from the window's edge");
            ui.label("sticker size");
            ui.add(egui::DragValue::new(&mut self.recipe.size).range(1.0..=96.0).suffix(" pt"));
            ui.label("sticker inset");
            ui.add(egui::DragValue::new(&mut self.recipe.inset).range(-96.0..=96.0).speed(0.25).suffix(" pt")).on_hover_text("How far each sticker slides from its joint toward the window's inside: corners diagonally, tees along the stem, crosses stay");
            if ui.small_button("centre on rail").on_hover_text("Rails set in by half the sticker's overhang, stickers on their joints: each sticker centred on the rail, nothing cut").clicked() {
                self.recipe.edge_inset = self.recipe.centred_edge_inset();
                self.recipe.inset = 0.0;
            }
            if ui.small_button("flush").on_hover_text("Keep the rails where they are and slide the stickers in until their square sits against the window's edge").clicked() {
                self.recipe.inset = self.recipe.flush_inset();
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Slots (pick one, then click a part below):");
            let edge = self.recipe.edge.clone();
            if ui.selectable_label(self.slot == Slot::Edge, format!("edge: {}", if edge.is_empty() { "-" } else { &edge })).clicked() {
                self.slot = Slot::Edge;
                self.browse = "edges";
            }
            for (slot, label, value) in [(Slot::Corner, "corner", self.recipe.corner.clone()), (Slot::Tee, "tee", self.recipe.tee.clone()), (Slot::Cross, "cross", self.recipe.cross.clone())] {
                if ui.selectable_label(self.slot == slot, format!("{label}: {}", value.as_deref().unwrap_or("-"))).clicked() {
                    self.slot = slot;
                    self.browse = "stickers";
                }
                if value.is_some() && ui.small_button("x").on_hover_text("No sticker there: a bare seam").clicked() {
                    match slot {
                        Slot::Corner => self.recipe.corner = None,
                        Slot::Tee => self.recipe.tee = None,
                        _ => self.recipe.cross = None,
                    }
                }
            }
        });
        // How the stickers turn: the default, and each corner and tee side.
        ui.horizontal_wrapped(|ui| {
            ui.weak("sticker turning");
            egui::ComboBox::from_id_salt("orient_default").selected_text(self.recipe.orient.word()).width(80.0).show_ui(ui, |ui| {
                for (o, w) in Orient::ALL {
                    ui.selectable_value(&mut self.recipe.orient, o, w);
                }
            });
            ui.label("corners");
            for (i, label) in ["TL", "TR", "BR", "BL"].iter().enumerate() {
                let cur = self.recipe.corners[i];
                egui::ComboBox::from_id_salt(("orient_corner", i)).selected_text(format!("{label} {}", cur.map(Orient::word).unwrap_or("-"))).width(84.0).show_ui(ui, |ui| {
                    if ui.selectable_label(cur.is_none(), "- (default)").clicked() {
                        self.recipe.corners[i] = None;
                    }
                    for (o, w) in Orient::ALL {
                        if ui.selectable_label(cur == Some(o), w).clicked() {
                            self.recipe.corners[i] = Some(o);
                        }
                    }
                });
            }
            ui.label("tees");
            for (i, label) in ["top", "right", "bottom", "left"].iter().enumerate() {
                let cur = self.recipe.tees[i];
                egui::ComboBox::from_id_salt(("orient_tee", i)).selected_text(format!("{label} {}", cur.map(Orient::word).unwrap_or("-"))).width(96.0).show_ui(ui, |ui| {
                    if ui.selectable_label(cur.is_none(), "- (default)").clicked() {
                        self.recipe.tees[i] = None;
                    }
                    for (o, w) in Orient::ALL {
                        if ui.selectable_label(cur == Some(o), w).clicked() {
                            self.recipe.tees[i] = Some(o);
                        }
                    }
                });
            }
        });
        effects_row(ui, "edge effects", &mut self.recipe.edge_effects);
        effects_row(ui, "sticker effects", &mut self.recipe.sticker_effects);
        // Ornament rules.
        let mut remove: Option<usize> = None;
        for i in 0..self.recipe.ornaments.len() {
            let mut pick = false;
            ui.horizontal_wrapped(|ui| {
                let o = &mut self.recipe.ornaments[i];
                if ui.selectable_label(self.slot == Slot::Ornament(i), format!("ornament {}: {}", i + 1, if o.image.is_empty() { "-" } else { &o.image })).clicked() {
                    pick = true;
                }
                ui.label("on");
                for (j, side) in ["top", "right", "bottom", "left"].iter().enumerate() {
                    ui.toggle_value(&mut o.sides[j], *side);
                }
                ui.selectable_value(&mut o.place, Place::Middle, "middle");
                ui.selectable_value(&mut o.place, Place::Ends, "ends").on_hover_text("One just inside each end's sticker");
                ui.selectable_value(&mut o.place, Place::Every, "every");
                if o.place == Place::Every {
                    ui.add(egui::DragValue::new(&mut o.spacing).range(1.0..=2000.0).suffix(" pt apart"));
                }
                ui.label("height");
                ui.add(egui::DragValue::new(&mut o.height).range(1.0..=200.0).suffix(" pt"));
                ui.checkbox(&mut o.outer, "outer only").on_hover_text("Only the outline's runs, not the lines between cells");
                if ui.small_button("remove").clicked() {
                    remove = Some(i);
                }
            });
            effects_row(ui, "   effects", &mut self.recipe.ornaments[i].effects);
            if pick {
                self.slot = Slot::Ornament(i);
                self.browse = "ornaments";
            }
        }
        if let Some(i) = remove {
            self.recipe.ornaments.remove(i);
            if matches!(self.slot, Slot::Ornament(j) if j >= self.recipe.ornaments.len()) {
                self.slot = Slot::Edge;
                self.browse = "edges";
            }
        }
        if ui.small_button("+ ornament rule").clicked() {
            self.recipe.ornaments.push(OrnamentRule { image: self.library["ornaments"].first().cloned().unwrap_or_default(), height: self.recipe.size.min(16.0), ..Default::default() });
            self.slot = Slot::Ornament(self.recipe.ornaments.len() - 1);
            self.browse = "ornaments";
        }
        if self.recipe != before {
            self.dirty = true;
            self.revalidate();
        }
        ui.separator();

        // Trim the chosen slot's part: cut its raw picture again.
        ui.horizontal_wrapped(|ui| {
            let (kind, name) = match self.slot {
                Slot::Edge => ("edges", Some(self.recipe.edge.clone()).filter(|e| !e.is_empty())),
                Slot::Corner => ("stickers", self.recipe.corner.clone()),
                Slot::Tee => ("stickers", self.recipe.tee.clone()),
                Slot::Cross => ("stickers", self.recipe.cross.clone()),
                Slot::Ornament(i) => ("ornaments", self.recipe.ornaments.get(i).map(|o| o.image.clone())),
            };
            if let Some(name) = name {
                let open = self.trim.as_ref().is_some_and(|t| t.name == name);
                if ui.selectable_label(open, format!("Trim {name}")).on_hover_text("Cut this part again from its raw picture: drag the box, its corner to size it, Apply rewrites the part in the library").clicked() {
                    self.trim = if open {
                        None
                    } else {
                        let raw_img = fr::load_raw(&name);
                        let cut = fr::read_cut(kind, &name).or_else(|| raw_img.as_ref().and_then(|r| crate::frame_template::measure_cut(kind.trim_end_matches('s'), r, &crate::frame_template::FrameToml::default()).ok()));
                        let raw = raw_img.map(|img| {
                            let color = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                            PartTex { tex: ui.ctx().load_texture(format!("frame-raw:{name}"), color, egui::TextureOptions::LINEAR), size: [img.width() as f32, img.height() as f32] }
                        });
                        Some(Trim { kind, name: name.clone(), cut: cut.unwrap_or([0.0, 0.0, 10.0, 10.0]), raw, drag: None, zoom: 1.0, pan: egui::Vec2::ZERO })
                    };
                }
            }
        });

        // Body: the library browser (or the trim editor) on the left, the preview on the right.
        const FOOT: f32 = 90.0;
        let avail = ui.available_size();
        let body_h = (avail.y - FOOT).max(240.0);
        let left_w = (avail.x * 0.42).max(220.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(left_w);
                if self.trim.is_some() {
                    self.trim_panel(ui, egui::vec2(left_w, body_h));
                } else {
                    self.browser(ui, egui::vec2(left_w, body_h));
                }
            });
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    for (view, label, hint) in VIEWS {
                        if ui.selectable_label(self.view == view, label).on_hover_text(hint).clicked() {
                            self.view = view;
                        }
                    }
                    ui.separator();
                    for z in [1.0, 2.0, 4.0, 6.0, 8.0, 10.0] {
                        if ui.selectable_value(&mut self.zoom, z, format!("{z}x")).on_hover_text("Device pixels per point; drag the preview to pan").clicked() {
                            self.pan = egui::Vec2::ZERO;
                        }
                    }
                    if ui.small_button("centre").on_hover_text("Pan back to the middle").clicked() {
                        self.pan = egui::Vec2::ZERO;
                    }
                    ui.separator();
                    ui.toggle_value(&mut self.guides, "guides").on_hover_text("The window's bound, each rail's centreline, each sticker's square and each ornament's box");
                });
                let size = egui::vec2((avail.x - left_w - 16.0).max(200.0), body_h - 24.0);
                self.preview(ui, size);
            });
        });

        ui.separator();
        for p in &self.problems {
            let color = if p.warning { ui.visuals().warn_fg_color } else { ui.visuals().error_fg_color };
            ui.colored_label(color, format!("{} {}", if p.warning { "warning:" } else { "error:" }, p.text));
        }
        if let Some(e) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        if let Some(n) = &self.note {
            ui.label(n);
        }

        if let Some(name) = load {
            self.load(&name);
        }
        if save && self.save(&mut outcome) && export {
            outcome.export_frame = Some(self.name.clone());
        }
        outcome
    }

    /// Write the recipe to `frames/<name>.toml`. True when it was written.
    fn save(&mut self, outcome: &mut CalibrationOutcome) -> bool {
        let name = self.name.trim().to_owned();
        if name.is_empty() || name.contains(['/', '\\']) {
            self.error = Some("Save as: a recipe name is needed".to_owned());
            return false;
        }
        let Some(path) = fr::recipe_path(&name) else { return false };
        match std::fs::write(&path, self.recipe.to_toml_string()) {
            Ok(()) => {
                self.dirty = false;
                self.error = None;
                self.note = Some(format!("Recipe written to {}", path.display()));
                outcome.reload_art = true;
                outcome.messages.push(format!("Frame recipe '{name}' saved."));
                if !self.recipes.iter().any(|r| *r == name) {
                    self.recipes.push(name.clone());
                    self.recipes.sort();
                }
                self.name = name;
                true
            }
            Err(e) => {
                self.error = Some(format!("Save failed: {e}"));
                false
            }
        }
    }

    /// What the host made of *Export pack*, shown under the recipe.
    pub fn exported(&mut self, result: Result<String, String>) {
        match result {
            Ok(note) => {
                self.note = Some(note);
                self.error = None;
            }
            Err(e) => self.error = Some(format!("Export failed: {e}")),
        }
    }

    /// The trim editor: the part's raw picture with its cut box; drag the
    /// body to move it, the bottom-right handle to size it; Apply recuts.
    fn trim_panel(&mut self, ui: &mut egui::Ui, size: egui::Vec2) {
        let mut apply = false;
        let mut close = false;
        ui.horizontal_wrapped(|ui| {
            let t = self.trim.as_mut().unwrap();
            ui.strong(format!("trim {}/{}", t.kind, t.name));
            let square = t.kind == "stickers";
            ui.add(egui::DragValue::new(&mut t.cut[0]).suffix(" x"));
            ui.add(egui::DragValue::new(&mut t.cut[1]).suffix(" y"));
            if ui.add(egui::DragValue::new(&mut t.cut[2]).range(1.0..=16384.0).suffix(" w")).changed() && square {
                t.cut[3] = t.cut[2];
            }
            ui.add_enabled(!square, egui::DragValue::new(&mut t.cut[3]).range(1.0..=16384.0).suffix(" h"));
            if ui.button("Apply").on_hover_text("Rewrite the part in the library from this cut").clicked() {
                apply = true;
            }
            if ui.button("Close").clicked() {
                close = true;
            }
            ui.separator();
            for z in [1.0, 2.0, 3.0, 4.0, 5.0, 6.0] {
                if ui.selectable_value(&mut t.zoom, z, format!("{z}x")).on_hover_text("Zoom on the raw picture; drag outside the box to pan").clicked() {
                    t.pan = egui::Vec2::ZERO;
                }
            }
        });
        let (rect, response) = ui.allocate_exact_size(egui::vec2(size.x, size.y - 30.0), egui::Sense::click_and_drag());
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
        let t = self.trim.as_mut().unwrap();
        let Some(raw) = &t.raw else {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, "no raw picture for this part (parts/raw/)", egui::FontId::proportional(14.0), ui.visuals().weak_text_color());
            if close {
                self.trim = None;
            }
            return;
        };
        let [rw, rh] = raw.size;
        let s = ((rect.width() - 8.0) / rw).min((rect.height() - 8.0) / rh) * t.zoom;
        let dest = egui::Rect::from_center_size(rect.center() + t.pan, egui::vec2(rw * s, rh * s));
        painter.image(raw.tex.id(), dest, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        let to_screen = |[x, y, w, h]: [f32; 4]| egui::Rect::from_min_size(dest.min + egui::vec2(x * s, y * s), egui::vec2(w * s, h * s));
        let r = to_screen(t.cut);
        let square = t.kind == "stickers";
        if response.drag_started() {
            if let Some(pos) = response.interact_pointer_pos() {
                t.drag = if (pos - r.right_bottom()).length() <= 12.0 { Some(true) } else if r.contains(pos) { Some(false) } else { None };
            }
        }
        if response.dragged() && t.drag.is_none() {
            // Outside the box: pan the picture.
            t.pan += response.drag_delta();
        }
        if response.dragged() {
            if let Some(sizing) = t.drag {
                let d = response.drag_delta() / s;
                if sizing {
                    if square {
                        let g = ((d.x + d.y) / 2.0).max(1.0 - t.cut[2]);
                        t.cut[2] += g;
                        t.cut[3] += g;
                    } else {
                        t.cut[2] = (t.cut[2] + d.x).max(1.0);
                        t.cut[3] = (t.cut[3] + d.y).max(1.0);
                    }
                } else {
                    t.cut[0] += d.x;
                    t.cut[1] += d.y;
                }
            }
        }
        if response.drag_stopped() {
            t.drag = None;
            t.cut = t.cut.map(f32::round);
        }
        let color = ui.visuals().hyperlink_color;
        painter.rect_stroke(r, 0.0, egui::Stroke::new(1.5, color), egui::StrokeKind::Outside);
        painter.rect_filled(egui::Rect::from_center_size(r.right_bottom(), egui::vec2(10.0, 10.0)), 0.0, color);
        painter.text(rect.min + egui::vec2(8.0, 8.0), egui::Align2::LEFT_TOP, "drag the box to move it, its corner to size it, outside it to pan; Apply rewrites the part", egui::FontId::proportional(11.0), ui.visuals().weak_text_color());
        if apply {
            let (kind, name, cut) = (t.kind, t.name.clone(), t.cut);
            match fr::recut_part(kind, &name, cut) {
                Ok(()) => {
                    self.parts.remove(&(kind, name.clone()));
                    self.blooms.retain(|k, _| !(k.0 == kind && k.1 == name));
                    self.note = Some(format!("{kind}/{name}.png cut again from its raw."));
                    self.revalidate();
                }
                Err(e) => self.error = Some(format!("Trim: {e}")),
            }
        }
        if close {
            self.trim = None;
        }
    }

    /// The library: a kind, a filter, thumbnails; a click fills the slot.
    fn browser(&mut self, ui: &mut egui::Ui, size: egui::Vec2) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            for kind in fr::PART_KINDS {
                if ui.selectable_label(self.browse == kind, format!("{kind} ({})", self.library[kind].len())).clicked() {
                    self.browse = kind;
                }
            }
            ui.add(egui::TextEdit::singleline(&mut self.filter).desired_width(120.0).hint_text("filter"));
            if ui.small_button("rescan").on_hover_text("Read the library again").clicked() {
                self.library = fr::PART_KINDS.iter().map(|k| (*k, fr::list_parts(k))).collect();
                self.recipes = fr::list_recipes();
                self.parts.clear();
            }
        });
        if self.browse != self.slot.kind() {
            ui.weak(format!("the chosen slot takes {}; showing {}", self.slot.kind(), self.browse));
        }
        let kind = self.browse;
        let names: Vec<String> = self.library[kind].iter().filter(|n| self.filter.is_empty() || n.to_ascii_lowercase().contains(&self.filter.to_ascii_lowercase())).cloned().collect();
        let chosen: Vec<String> = self.recipe.parts().into_iter().filter(|(k, _)| *k == kind).map(|(_, n)| n).collect();
        let cell = if kind == "edges" { egui::vec2(size.x - 24.0, 44.0) } else { egui::vec2(96.0, 96.0) };
        let per_row = ((size.x - 16.0) / (cell.x + 8.0)).floor().max(1.0) as usize;
        let mut picked: Option<String> = None;
        egui::ScrollArea::vertical().id_salt(("frame_library", kind)).max_height(size.y - 30.0).show(ui, |ui| {
            for row in names.chunks(per_row) {
                ui.horizontal(|ui| {
                    for name in row {
                        let (r, resp) = ui.allocate_exact_size(cell, egui::Sense::click());
                        let painter = ui.painter().with_clip_rect(r);
                        painter.rect_filled(r, 3.0, egui::Color32::from_gray(50));
                        if let Some((tex, [w, h])) = self.part(&ctx, kind, name) {
                            let s = ((r.width() - 6.0) / w).min((r.height() - 6.0) / h);
                            let dest = egui::Rect::from_center_size(r.center(), egui::vec2(w * s, h * s));
                            painter.image(tex, dest, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                        }
                        if chosen.iter().any(|c| c == name) {
                            painter.rect_stroke(r, 3.0, egui::Stroke::new(2.0, ui.visuals().hyperlink_color), egui::StrokeKind::Inside);
                        }
                        if resp.on_hover_text(name).clicked() {
                            picked = Some(name.clone());
                        }
                    }
                });
            }
        });
        if let Some(name) = picked {
            if self.slot.kind() == kind {
                match self.slot {
                    Slot::Edge => self.recipe.edge = name,
                    Slot::Corner => self.recipe.corner = Some(name),
                    Slot::Tee => self.recipe.tee = Some(name),
                    Slot::Cross => self.recipe.cross = Some(name),
                    Slot::Ornament(i) => {
                        if let Some(o) = self.recipe.ornaments.get_mut(i) {
                            o.image = name;
                        }
                    }
                }
                self.dirty = true;
                self.revalidate();
            } else {
                self.note = Some(format!("Pick a {} slot first, then click a {kind} part.", kind.trim_end_matches('s')));
            }
        }
    }

    /// The assembly, or the seam check.
    fn preview(&mut self, ui: &mut egui::Ui, size: egui::Vec2) {
        let ctx = ui.ctx().clone();
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::drag());
        if response.dragged() {
            self.pan += response.drag_delta();
        }
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, 4.0, egui::Color32::from_gray(28));
        let sizes = self.sizes(&ctx);
        let Some([ew, eh]) = sizes.edge else {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, "pick an edge", egui::FontId::proportional(14.0), ui.visuals().weak_text_color());
            return;
        };
        let zoom = self.zoom;
        let t = self.recipe.thickness;
        if self.view == View::Seam {
            let edge = self.recipe.edge.clone();
            let Some((tex, _)) = self.part(&ctx, "edges", &edge) else { return };
            let (w, h) = (ew / eh.max(1.0) * t * zoom, t * zoom);
            let origin = rect.center() - egui::vec2(1.5 * w, h / 2.0);
            for i in 0..3 {
                let dest = egui::Rect::from_min_size(origin + egui::vec2(i as f32 * w, 0.0), egui::vec2(w, h));
                painter.image(tex, dest, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
            }
            for i in 1..3 {
                let x = origin.x + i as f32 * w;
                painter.line_segment([egui::pos2(x, origin.y - 12.0), egui::pos2(x, origin.y - 2.0)], egui::Stroke::new(1.0, egui::Color32::YELLOW));
                painter.line_segment([egui::pos2(x, origin.y + h + 2.0), egui::pos2(x, origin.y + h + 12.0)], egui::Stroke::new(1.0, egui::Color32::YELLOW));
            }
            painter.text(egui::pos2(rect.min.x + 8.0, rect.min.y + 8.0), egui::Align2::LEFT_TOP, format!("edge {:.0} x {:.0} pt, tiled three times; the joins are marked", ew / eh.max(1.0) * t, t), egui::FontId::proportional(12.0), ui.visuals().text_color());
            return;
        }
        let (cols, rows) = match self.view {
            View::Window => (1, 1),
            View::Row => (2, 1),
            _ => (2, 2),
        };
        // The frame keeps its point size as it is zoomed: at 1x it fits the panel, past that it
        // runs beyond it and the pan brings the rest into view.
        let inner = rect.shrink(16.0);
        let plan_w = inner.width().min(420.0);
        let plan_h = inner.height().min(300.0);
        let plan = Plan { rect: [0.0, 0.0, plan_w, plan_h], cols, rows };
        let origin = inner.center() - egui::vec2(plan_w, plan_h) * zoom / 2.0 + self.pan;
        let placed = fr::layout(&plan, &self.recipe, &sizes);
        let bound = egui::Rect::from_min_size(origin, egui::vec2(plan_w, plan_h) * zoom);
        painter.rect_filled(bound, 0.0, egui::Color32::from_gray(44));
        let painter = painter.with_clip_rect(bound.intersect(rect));
        let now = ctx.input(|i| i.time) as f32;
        let mut pulsing = false;
        for p in &placed {
            let Some((tex, [pw_px, ph_px])) = self.part(&ctx, p.kind, &p.name) else { continue };
            let [x, y, w, h] = p.placed.rect;
            let dest = egui::Rect::from_min_size(origin + egui::vec2(x, y) * zoom, egui::vec2(w, h) * zoom);
            // Part pixels per point as this slot draws it: the blur radius in points becomes part pixels.
            let ppp = if matches!(p.placed.kind, crate::frame_template::PieceKind::Edge) { ph_px / h.max(0.5) } else { (pw_px / w.max(0.5)).max(ph_px / h.max(0.5)) };
            let tint = p.effects.tint.as_deref().and_then(parse_hex_rgb).unwrap_or(egui::Color32::WHITE);
            paint_turned(&painter, tex, dest, p.placed.uvs, tint);
            for e in &p.effects.emissive {
                if let Some(btex) = self.bloom(&ctx, p.kind, &p.name, e, e.radius * ppp) {
                    let breathe = match e.pulse {
                        Some(period) if period > 0.0 => {
                            pulsing = true;
                            0.7 + 0.3 * (now * std::f32::consts::TAU / period).sin()
                        }
                        _ => 1.0,
                    };
                    // Strength as a premultiplied tint with no alpha: the bloom adds.
                    let k = (e.strength * breathe).clamp(0.0, 1.0);
                    let c = egui::Color32::from_rgba_premultiplied((255.0 * k) as u8, (255.0 * k) as u8, (255.0 * k) as u8, 0);
                    // The bloom's picture is the part padded by the radius, drawn on the part's box
                    // grown by the same radius (one to one); what shows is clipped to the box grown
                    // by `cast` alone: cast 0 = no light leaves the part. The radius in points on
                    // screen matches the padding in part pixels through ppp.
                    let grow = egui::vec2(e.radius, e.radius) * zoom;
                    let clip = painter.clip_rect().intersect(dest.expand2(egui::vec2(e.cast, e.cast) * zoom));
                    let bloom_painter = painter.with_clip_rect(clip);
                    paint_turned(&bloom_painter, btex, dest.expand2(grow), p.placed.uvs, c);
                    if e.strength * breathe > 1.0 {
                        let k2 = (e.strength * breathe - 1.0).min(1.0);
                        let c2 = egui::Color32::from_rgba_premultiplied((255.0 * k2) as u8, (255.0 * k2) as u8, (255.0 * k2) as u8, 0);
                        paint_turned(&bloom_painter, btex, dest.expand2(grow), p.placed.uvs, c2);
                    }
                }
            }
        }
        if pulsing {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        if self.guides {
            let g = ui.painter().with_clip_rect(rect);
            let yellow = egui::Color32::from_rgba_unmultiplied(255, 220, 60, 220);
            let cyan = egui::Color32::from_rgba_unmultiplied(80, 220, 255, 220);
            let green = egui::Color32::from_rgba_unmultiplied(120, 255, 120, 200);
            g.rect_stroke(bound, 0.0, egui::Stroke::new(1.0, yellow), egui::StrokeKind::Outside);
            for p in &placed {
                let [x, y, w, h] = p.placed.rect;
                let r = egui::Rect::from_min_size(origin + egui::vec2(x, y) * zoom, egui::vec2(w, h) * zoom);
                match p.kind {
                    "edges" => {
                        // The rail's centreline, along its long axis.
                        let (a, b) = if w >= h { (egui::pos2(r.min.x, r.center().y), egui::pos2(r.max.x, r.center().y)) } else { (egui::pos2(r.center().x, r.min.y), egui::pos2(r.center().x, r.max.y)) };
                        g.line_segment([a, b], egui::Stroke::new(1.0, cyan));
                    }
                    "stickers" => {
                        g.rect_stroke(r, 0.0, egui::Stroke::new(1.0, green), egui::StrokeKind::Outside);
                        g.line_segment([egui::pos2(r.center().x - 4.0, r.center().y), egui::pos2(r.center().x + 4.0, r.center().y)], egui::Stroke::new(1.0, green));
                        g.line_segment([egui::pos2(r.center().x, r.center().y - 4.0), egui::pos2(r.center().x, r.center().y + 4.0)], egui::Stroke::new(1.0, green));
                    }
                    _ => {
                        g.rect_stroke(r, 0.0, egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(255, 160, 255, 200)), egui::StrokeKind::Outside);
                    }
                }
            }
            let e = self.recipe.edge_inset;
            if e > 0.0 {
                g.rect_stroke(bound.shrink(e * zoom), 0.0, egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(255, 220, 60, 120)), egui::StrokeKind::Outside);
            }
        }
        let label = format!("{}x{} at {:.0}x{:.0} pt, {}x: edge {:.0} pt, stickers {:.0} pt, {:?}", cols, rows, plan_w, plan_h, zoom, t, self.recipe.size, self.recipe.turn);
        ui.painter().with_clip_rect(rect).text(egui::pos2(rect.min.x + 8.0, rect.min.y + 8.0), egui::Align2::LEFT_TOP, label, egui::FontId::proportional(12.0), ui.visuals().text_color());
    }
}

/// One slot's effects as controls: glow, shadow, tint, each on or off.
fn effects_row(ui: &mut egui::Ui, label: &str, fx: &mut Effects) {
    ui.horizontal_wrapped(|ui| {
        ui.weak(label);
        let mut tint_on = fx.tint.is_some();
        if ui.checkbox(&mut tint_on, "tint").changed() {
            fx.tint = tint_on.then(|| "#c8b89a".to_owned());
        }
        if let Some(t) = &mut fx.tint {
            color_field(ui, t);
        }
        if ui.small_button("+ emissive").on_hover_text("A bloom from inside the art: the part's own pixels near a colour, blurred and added over it; one per glowing colour").clicked() {
            fx.emissive.push(Emissive::default());
        }
    });
    let mut remove = None;
    for (i, e) in fx.emissive.iter_mut().enumerate() {
        ui.horizontal_wrapped(|ui| {
            ui.weak(format!("      emissive {}", i + 1));
            color_field(ui, &mut e.color);
            ui.add(egui::DragValue::new(&mut e.tolerance).range(0.05..=0.8).speed(0.01).suffix(" tol")).on_hover_text("How far from the colour a pixel may be and still glow");
            egui::ComboBox::from_id_salt(("em_radius", label, i)).selected_text(format!("{} pt", e.radius)).width(70.0).show_ui(ui, |ui| {
                for r in fr::EMISSIVE_RADII {
                    ui.selectable_value(&mut e.radius, r, format!("{r} pt"));
                }
            });
            ui.add(egui::DragValue::new(&mut e.strength).range(0.0..=3.0).speed(0.02).suffix(" x"));
            let mut pulse_on = e.pulse.is_some();
            if ui.checkbox(&mut pulse_on, "pulse").changed() {
                e.pulse = pulse_on.then_some(3.0);
            }
            if let Some(p) = &mut e.pulse {
                ui.add(egui::DragValue::new(p).range(0.2..=20.0).speed(0.1).suffix(" s"));
            }
            ui.add(egui::DragValue::new(&mut e.cast).range(0.0..=64.0).speed(0.25).fixed_decimals(0).suffix(" pt cast")).on_hover_text("How far the bloom may leave the part's box, over the content or the rail beside a sticker: 0 keeps every bit of light inside the part");
            if ui.small_button("remove").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        fx.emissive.remove(i);
    }
}

/// Draw `tex` into `dest` with the piece's (u, v) at the destination's four
/// corners as `uvs` gives them (top-left, top-right, bottom-right,
/// bottom-left): how a top piece becomes a side.
pub fn paint_turned(painter: &egui::Painter, tex: egui::TextureId, dest: egui::Rect, uvs: [[f32; 2]; 4], tint: egui::Color32) {
    let mut mesh = egui::Mesh::with_texture(tex);
    let corners = [dest.left_top(), dest.right_top(), dest.right_bottom(), dest.left_bottom()];
    for (pos, [u, v]) in corners.into_iter().zip(uvs) {
        mesh.vertices.push(egui::epaint::Vertex { pos, uv: egui::pos2(u, v), color: tint });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}
