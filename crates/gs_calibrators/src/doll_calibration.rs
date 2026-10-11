//! The injury-doll picture calibrator (REBUILT 2026-10-10, after the crate
//! split deleted VellumFE's): click a doll picture to place each body
//! part's wound dot, style the dots, and save the result **into the
//! picture**, in the `vellum-meta` text chunk Hydra reads
//! (`hydra-gui/src/calibration.rs`). A doll travels as one PNG; nothing is
//! written beside it.
//!
//! ```toml
//! kind = "doll"
//! [anchors]
//! head = [0.5, 0.09]        # fractions of the picture, the game's part ids
//! leftArm = [0.31, 0.36]
//! [dots]
//! opacity = 0.9
//! diameter = 0.07           # of the picture's height
//! wound_color = "#e02020"   # the preview's; Hydra keeps what it does not read
//! scar_color = "#b8b8b8"
//! ```
//!
//! A picture with no chunk may have a working copy beside it,
//! `<picture>.toml`, read in its place (what Hydra does too). Keys are read
//! without regard to case and written in the game's spelling. A part with
//! no anchor sits at the doll's default place (`skins::DOLL_PARTS`).
//!
//! The pictures are the pool's `global/images/dolls/*.png`; an overlay,
//! `<picture>_<part>_<level>.png` (Hydra's "Doll plus"), is not a doll.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::color::color_field;
use crate::skin_paint::{self as gui_skin, ResolvedDotStyle, SkinTexture};
use crate::CalibrationOutcome;
use creature_field::png_meta;
use creature_field::skins::{self, DOLL_PARTS};

/// The levels an overlay file names (Hydra's `doll_art::LEVELS`).
const LEVELS: [&str; 7] = ["healthy", "injury1", "injury2", "injury3", "scar1", "scar2", "scar3"];

/// Whether the file named `stem` is one of a doll's overlays rather than a
/// doll: `<picture>_<part>_<level>`.
pub fn is_overlay(stem: &str) -> bool {
    let mut pieces = stem.rsplitn(3, '_');
    let (Some(level), Some(part), Some(_)) = (pieces.next(), pieces.next(), pieces.next()) else { return false };
    LEVELS.iter().any(|l| l.eq_ignore_ascii_case(level)) && DOLL_PARTS.iter().any(|(id, _, _)| id.eq_ignore_ascii_case(part))
}

/// The pool's dolls: `global/images/dolls/*.png`, overlays left out,
/// sorted. Each as its pool-relative path (`dolls/<file>`).
pub fn pool_dolls() -> Vec<String> {
    let Ok(root) = creature_field::pool_dirs::global_images_dir() else { return Vec::new() };
    let mut out: Vec<String> = std::fs::read_dir(root.join("dolls"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("png")))
        .filter(|p| !p.file_stem().and_then(|s| s.to_str()).is_some_and(is_overlay))
        .filter_map(|p| Some(format!("dolls/{}", p.file_name()?.to_str()?)))
        .collect();
    out.sort();
    out
}

/// Which pool doll to calibrate.
#[derive(Debug, Clone)]
pub struct DollCalibrationTarget {
    /// Pool-relative path (`dolls/x.png`); the title and messages use its stem.
    pub pool_path: String,
    /// The picture itself: read, and written back with its calibration in it.
    pub image_abs: PathBuf,
}

impl DollCalibrationTarget {
    /// A pool-relative path against the shared image pool.
    pub fn from_pool_path(pool_path: &str) -> Self {
        let root = creature_field::pool_dirs::global_images_dir().unwrap_or_default();
        Self { pool_path: pool_path.to_owned(), image_abs: root.join(pool_path) }
    }
}

/// What the dots look like.
#[derive(Debug, Clone, PartialEq)]
pub struct DollDotSpec {
    pub wound_color: String,
    pub scar_color: String,
    pub opacity: f32,
    /// Diameter as a fraction of the drawn doll height.
    pub diameter: f32,
}

impl Default for DollDotSpec {
    fn default() -> Self {
        Self { wound_color: "#e02020".to_owned(), scar_color: "#b8b8b8".to_owned(), opacity: 0.9, diameter: 0.07 }
    }
}

/// A picture's calibration, as read from and written to its chunk.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DollCalibration {
    /// Anchors by the game's part id (as `DOLL_PARTS` spells it).
    pub anchors: HashMap<String, [f32; 2]>,
    pub dots: DollDotSpec,
}

/// A TOML number, written with a point or without.
fn number(value: &toml::Value) -> Option<f64> {
    value.as_float().or_else(|| value.as_integer().map(|i| i as f64))
}

impl DollCalibration {
    /// From the chunk's TOML. What it does not know it leaves at the
    /// default: a part it has no id for, a value out of range.
    pub fn of_toml(text: &str) -> Self {
        let mut read = Self::default();
        let Ok(table) = text.parse::<toml::Table>() else { return read };
        if let Some(anchors) = table.get("anchors").and_then(toml::Value::as_table) {
            for (key, value) in anchors {
                let Some((id, _, _)) = DOLL_PARTS.iter().find(|(id, _, _)| id.eq_ignore_ascii_case(key)) else { continue };
                let pair: Option<Vec<f64>> = value.as_array().map(|a| a.iter().filter_map(number).collect());
                if let Some([x, y]) = pair.as_deref() {
                    if (0.0..=1.0).contains(x) && (0.0..=1.0).contains(y) {
                        read.anchors.insert((*id).to_owned(), [*x as f32, *y as f32]);
                    }
                }
            }
        }
        if let Some(dots) = table.get("dots").and_then(toml::Value::as_table) {
            if let Some(v) = dots.get("opacity").and_then(number) {
                read.dots.opacity = v.clamp(0.0, 1.0) as f32;
            }
            if let Some(v) = dots.get("diameter").and_then(number) {
                read.dots.diameter = v.clamp(0.01, 0.5) as f32;
            }
            if let Some(v) = dots.get("wound_color").and_then(toml::Value::as_str) {
                read.dots.wound_color = v.to_owned();
            }
            if let Some(v) = dots.get("scar_color").and_then(toml::Value::as_str) {
                read.dots.scar_color = v.to_owned();
            }
        }
        read
    }

    /// The calibration text the picture at `path` carries: its own chunk,
    /// else the `.toml` beside it.
    fn text_of(path: &Path) -> Option<String> {
        png_meta::read_embedded(path).or_else(|| std::fs::read_to_string(path.with_extension("toml")).ok())
    }

    /// The picture's calibration, or the defaults when it has none.
    pub fn of_picture(path: &Path) -> Self {
        Self::text_of(path).map_or_else(Self::default, |t| Self::of_toml(&t))
    }

    /// `old` with this calibration's anchors and dots in it: whatever else
    /// the picture's calibration held is carried over.
    fn over_table(&self, mut old: toml::Table) -> toml::Table {
        let round = |v: f32, places: f64| (f64::from(v) * places).round() / places;
        old.insert("kind".to_owned(), toml::Value::from("doll"));
        let mut anchors = toml::Table::new();
        for (id, _, _) in DOLL_PARTS {
            if let Some([x, y]) = self.anchors.get(*id) {
                anchors.insert((*id).to_owned(), toml::Value::Array(vec![round(*x, 10_000.0).into(), round(*y, 10_000.0).into()]));
            }
        }
        old.insert("anchors".to_owned(), toml::Value::Table(anchors));
        let mut dots = old.remove("dots").and_then(|d| d.as_table().cloned()).unwrap_or_default();
        dots.insert("opacity".to_owned(), round(self.dots.opacity, 100.0).into());
        dots.insert("diameter".to_owned(), round(self.dots.diameter, 1_000.0).into());
        dots.insert("wound_color".to_owned(), self.dots.wound_color.trim().into());
        dots.insert("scar_color".to_owned(), self.dots.scar_color.trim().into());
        old.insert("dots".to_owned(), toml::Value::Table(dots));
        old
    }

    /// Write into the picture at `path`: its chunk replaced (or put after
    /// the header), the pixels and every other chunk byte for byte. Written
    /// to a temp file beside it and renamed into place.
    pub fn write(&self, path: &Path) -> anyhow::Result<()> {
        let bytes = std::fs::read(path)?;
        let old = Self::text_of(path).and_then(|t| t.parse::<toml::Table>().ok()).unwrap_or_default();
        let text = toml::to_string(&self.over_table(old))?;
        let out = png_meta::write_embedded_bytes(&bytes, &text).ok_or_else(|| anyhow::anyhow!("{} is not a PNG", path.display()))?;
        let tmp = path.with_extension("png.tmp");
        std::fs::write(&tmp, out)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

pub struct DollCalibrationState {
    target: DollCalibrationTarget,
    label: String,
    /// Working anchors by part id. Only placed parts appear; the rest draw
    /// at the built-in defaults and stay out of the saved file.
    anchors: HashMap<String, [f32; 2]>,
    /// Index into DOLL_PARTS of the part the next click places.
    selected: usize,
    auto_advance: bool,
    dots: DollDotSpec,
    /// Preview dots as scars (rings) instead of wounds (solid).
    preview_scars: bool,
    /// Preview severity numeral, 1-3.
    preview_level: u8,
    error: Option<String>,
    /// The picture, loaded once.
    own_base: Option<egui::TextureHandle>,
    own_base_tried: bool,
    /// Edits since the last save.
    dirty: bool,
}

impl DollCalibrationState {
    pub fn open(target: Option<DollCalibrationTarget>) -> (Option<Self>, CalibrationOutcome) {
        let mut outcome = CalibrationOutcome::default();
        let Some(target) = target else {
            outcome.messages.push("No doll picture selected.".to_owned());
            return (None, outcome);
        };
        let label = Path::new(&target.pool_path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| target.pool_path.clone());
        let read = DollCalibration::of_picture(&target.image_abs);
        let state = DollCalibrationState {
            target,
            label,
            anchors: read.anchors,
            selected: 0,
            auto_advance: true,
            dots: read.dots,
            preview_scars: false,
            preview_level: 2,
            error: None,
            own_base: None,
            own_base_tried: false,
            dirty: false,
        };
        (Some(state), outcome)
    }

    pub fn has_unsaved_changes(&self) -> bool {
        self.dirty
    }

    fn calibration(&self) -> DollCalibration {
        DollCalibration { anchors: self.anchors.clone(), dots: self.dots.clone() }
    }

    fn anchor_for(&self, id: &str) -> egui::Vec2 {
        self.anchors.get(id).copied().or_else(|| skins::default_doll_anchor(id)).map(|[x, y]| egui::vec2(x, y)).unwrap_or(egui::vec2(0.5, 0.5))
    }

    fn own_base(&mut self, ctx: &egui::Context) -> Option<SkinTexture> {
        if !self.own_base_tried {
            self.own_base_tried = true;
            self.own_base = creature_field_egui::image_store::load_texture_file(ctx, &self.target.image_abs, &format!("doll-cal:{}", self.target.pool_path), "doll calibration");
        }
        self.own_base.as_ref().map(|h| SkinTexture { texture: h.id(), size: h.size_vec2() })
    }

    /// Floating pop-out (the calibrators demo).
    pub fn ui(&mut self, ctx: &egui::Context) -> CalibrationOutcome {
        let mut outcome = CalibrationOutcome::default();
        let mut open = true;
        egui::Window::new(format!("Injury Doll Calibration - {}", self.label))
            .id(egui::Id::new("gui_doll_calibration"))
            .order(egui::Order::Foreground)
            .open(&mut open)
            .default_width(560.0)
            .default_height(520.0)
            .resizable(true)
            .show(ctx, |ui| outcome = self.ui_contents(ui));
        outcome.closed |= !open;
        outcome
    }

    /// What the host made of *Export pack*: a failure shown under Save
    /// (a success is in the status bar).
    pub fn exported(&mut self, result: Result<String, String>) {
        self.error = result.err().map(|e| format!("Export failed: {e}"));
    }

    fn base_vanished(&self) -> CalibrationOutcome {
        let mut outcome = CalibrationOutcome::default();
        outcome.messages.push(format!("Injury doll calibration closed: '{}' cannot be read as a picture.", self.target.image_abs.display()));
        outcome.closed = true;
        outcome
    }

    /// The whole calibrator in a caller's region (a tab). `closed` is set
    /// in one case: the picture cannot be loaded.
    pub fn ui_contents(&mut self, ui: &mut egui::Ui) -> CalibrationOutcome {
        let Some(base) = self.own_base(&ui.ctx().clone()) else { return self.base_vanished() };
        let mut outcome = CalibrationOutcome::default();
        let state = self;
        let mut save_request = false;
        let mut export_request = false;

        ui.label("Click the doll to place the highlighted part's dot. Positions are fractions of the picture, so they hold at any size. Save writes them into the PNG itself.");
        ui.separator();
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(150.0);
                for (index, (id, display, _)) in DOLL_PARTS.iter().enumerate() {
                    let label = if state.anchors.contains_key(*id) { format!("{display} \u{2022}") } else { (*display).to_string() };
                    if ui.selectable_label(state.selected == index, label).clicked() {
                        state.selected = index;
                    }
                }
                ui.add_space(6.0);
                ui.checkbox(&mut state.auto_advance, "Auto-advance").on_hover_text("Jump to the next part after each click");
                if ui.button("Use default").on_hover_text("Drop the selected part's calibration and fall back to the built-in position").clicked() {
                    if state.anchors.remove(DOLL_PARTS[state.selected].0).is_some() {
                        state.dirty = true;
                    }
                }
            });

            const CONTROLS_HEIGHT: f32 = 130.0;
            let avail = ui.available_size();
            let canvas = egui::Vec2::new(avail.x.max(160.0), (avail.y - CONTROLS_HEIGHT).max(200.0));
            let (rect, response) = ui.allocate_exact_size(canvas, egui::Sense::click());
            let painter = ui.painter().with_clip_rect(rect);
            painter.rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
            let dest = gui_skin::sprite_dest(&base, rect);
            gui_skin::paint_sprite(&painter, dest, &base, egui::Color32::WHITE);

            let style = ResolvedDotStyle::from_spec(&state.dots);
            let level = state.preview_level.clamp(1, 3) + if state.preview_scars { 3 } else { 0 };
            let radius = (style.diameter * dest.height() / 2.0).max(4.0);
            let at = |anchor: egui::Vec2| dest.min + egui::Vec2::new(anchor.x * dest.width(), anchor.y * dest.height());
            for (id, _, _) in DOLL_PARTS {
                gui_skin::paint_severity_dot(&painter, at(state.anchor_for(id)), radius, level, &style);
            }

            let highlight = ui.visuals().hyperlink_color;
            let center = at(state.anchor_for(DOLL_PARTS[state.selected].0));
            let stroke = egui::Stroke::new(1.0, highlight);
            painter.line_segment([egui::pos2(dest.min.x, center.y), egui::pos2(dest.max.x, center.y)], stroke);
            painter.line_segment([egui::pos2(center.x, dest.min.y), egui::pos2(center.x, dest.max.y)], stroke);
            painter.circle_stroke(center, radius + 3.0, egui::Stroke::new(2.0, highlight));

            if response.clicked() {
                if let Some(pos) = response.interact_pointer_pos() {
                    if dest.contains(pos) && dest.width() > 0.0 && dest.height() > 0.0 {
                        let normalized = [((pos.x - dest.min.x) / dest.width()).clamp(0.0, 1.0), ((pos.y - dest.min.y) / dest.height()).clamp(0.0, 1.0)];
                        state.anchors.insert(DOLL_PARTS[state.selected].0.to_owned(), normalized);
                        state.dirty = true;
                        if state.auto_advance {
                            state.selected = (state.selected + 1) % DOLL_PARTS.len();
                        }
                    }
                }
            }
        });

        ui.separator();
        let before = state.dots.clone();
        ui.horizontal(|ui| {
            ui.label("Wound");
            color_field(ui, &mut state.dots.wound_color);
            ui.label("Scar");
            color_field(ui, &mut state.dots.scar_color);
            ui.label("Preview:");
            ui.selectable_value(&mut state.preview_scars, false, "wounds");
            ui.selectable_value(&mut state.preview_scars, true, "scars");
            ui.add(egui::Slider::new(&mut state.preview_level, 1..=3).text("rank"));
        });
        ui.horizontal(|ui| {
            ui.add(egui::Slider::new(&mut state.dots.diameter, 0.02..=0.20).text("dot size").custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
            ui.add(egui::Slider::new(&mut state.dots.opacity, 0.2..=1.0).text("opacity").custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
        });
        if state.dots != before {
            state.dirty = true;
        }

        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Save").on_hover_text("Writes the anchors and dot style into the picture's PNG (its vellum-meta chunk), the one file Hydra reads").clicked() {
                save_request = true;
            }
            if ui
                .button("Export pack")
                .on_hover_text(format!(
                    "Saves, then packs the picture with its own overlays for Hydra: packs/doll_{}.zip and its line in packs/packs.toml, the version raised when the pack changed",
                    state.label.to_ascii_lowercase()
                ))
                .clicked()
            {
                save_request = true;
                export_request = true;
            }
            if ui.button("Reset all to defaults").clicked() {
                state.anchors.clear();
                state.dots = DollDotSpec::default();
                state.dirty = true;
            }
            if state.dirty {
                ui.weak("unsaved");
            }
        });
        if let Some(error) = &state.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }

        if save_request {
            match state.calibration().write(&state.target.image_abs) {
                Ok(()) => {
                    state.error = None;
                    state.dirty = false;
                    outcome.reload_art = true;
                    outcome.messages.push(format!("Injury doll calibration saved into '{}'.", state.label));
                    if export_request {
                        outcome.export_pack = Some(state.label.clone());
                    }
                }
                Err(err) => state.error = Some(format!("Failed to save: {err}")),
            }
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255])).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    #[test]
    fn lower_case_keys_are_read_and_the_rest_defaults() {
        let read = DollCalibration::of_toml("kind = \"doll\"\n[anchors]\nleftarm = [0.7763, 0.2857]\nwing = [0.1, 0.1]\nhead = [2.0, 0.5]\n[dots]\nopacity = 0.5\ndiameter = 9.0\n");
        assert_eq!(read.anchors.get("leftArm"), Some(&[0.7763, 0.2857]));
        assert_eq!(read.anchors.len(), 1, "no wing, and no head off the picture");
        assert_eq!(read.dots.opacity, 0.5);
        assert_eq!(read.dots.diameter, 0.5, "clamped");
        assert_eq!(read.dots.wound_color, "#e02020");
    }

    /// Written into the picture in Hydra's format: read back the same, the
    /// chunk never doubled, every other chunk as it was, what the `.toml`
    /// beside it held carried in, and the picture still a picture.
    #[test]
    fn a_calibration_is_written_into_the_picture_and_nothing_else_changes() {
        let dir = std::env::temp_dir().join(format!("gs_doll_calibration_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("doll.png");
        std::fs::write(&path, png(3, 2)).unwrap();
        std::fs::write(dir.join("doll.toml"), "priority = 4\n[dots]\nextra = 1\n").unwrap();
        let before = std::fs::read(&path).unwrap();

        let mut cal = DollCalibration::default();
        cal.anchors.insert("leftArm".to_owned(), [0.25, 0.5]);
        cal.dots.opacity = 0.6;
        cal.write(&path).unwrap();
        cal.anchors.insert("head".to_owned(), [0.5, 0.125]);
        cal.write(&path).unwrap();

        let after = std::fs::read(&path).unwrap();
        assert_eq!(DollCalibration::of_picture(&path), cal);
        let text = png_meta::read_embedded_bytes(&after).unwrap();
        assert!(text.contains("kind = \"doll\"") && text.contains("leftArm = [0.25, 0.5]"), "{text}");
        assert!(text.contains("priority = 4") && text.contains("extra = 1"), "carried over: {text}");
        assert_eq!(text.matches("leftArm").count(), 1, "replaced, not doubled");
        assert!(image::load_from_memory(&after).is_ok(), "still a picture");
        assert_eq!(image::load_from_memory(&after).unwrap().to_rgba8().as_raw(), image::load_from_memory(&before).unwrap().to_rgba8().as_raw(), "pixels byte for byte");
        assert!(!dir.join("doll.png.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_overlay_is_not_a_doll() {
        assert!(is_overlay("nisugi_leftArm_injury2"));
        assert!(is_overlay("stealth_male_head_Scar1"));
        assert!(!is_overlay("stealth_male"));
        assert!(!is_overlay("nisugi_bow"));
        assert!(!is_overlay("a_wing_injury1"));
    }
}
