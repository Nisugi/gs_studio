//! puppet_demo: the station puppet in a bare eframe window.
//!
//!   cargo run -p gs_puppet --example puppet_demo
//!
//! `PUPPET_DEMO_EXIT_AFTER_SECS=<n>` closes the window by itself (smoke runs).
//! `PUPPET_DEMO_SHOTS=<dir>` opens NO window: it renders a fixed set of views
//! with the crate's software rasteriser, writes them as PNGs (plus the baked
//! puppets), and prints fit and cost numbers.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use gs_puppet::math::V3;
use gs_puppet::{
    guess_depths_mask, ground_grid, rest_iou, rest_prims, Camera, DepthGuess, Look, Pose, PoseSet, Prim, Puppet,
    MASK_PX,
};
use rig::{OutlineEntry, Outlines, Rig, View};
use rig_bake::Mask;

mod shots;

const BACKGROUND: [u8; 3] = [0x2b, 0x30, 0x36];
const GRID: [u8; 4] = [0x4a, 0x52, 0x5c, 255];
const SHADOW: [u8; 4] = [0x1d, 0x21, 0x26, 255];
const WOUND_PARTS: [&str; 11] = [
    "head", "neck", "chest", "abdomen", "back", "leftArm", "rightArm", "leftHand", "rightHand", "leftLeg",
    "rightLeg",
];
const FAMILIES: [&str; 2] = ["humanoid", "canine"];

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

pub struct Figure {
    pub label: String,
    pub rig: Rig,
    pub front: Option<(OutlineEntry, Mask)>,
    pub side: Option<(OutlineEntry, Mask)>,
    pub guesses: BTreeMap<String, DepthGuess>,
    pub puppet: Puppet,
    pub poses: PoseSet,
}

impl Figure {
    fn load(outlines: &Outlines, family: &str) -> Result<Figure, String> {
        let dir = data_dir();
        let rig = Rig::load(&dir.join("rigs").join(Rig::file_name(family))).map_err(|e| e.to_string())?;
        let view = |v: View| outlines.get(family, v).map(|e| (e.clone(), rig_bake::rasterize(&e.outline(), MASK_PX)));
        let (front, side) = (view(View::Front), view(View::Side));
        let poses = PoseSet::load(&dir.join("poses"), &rig.body_type)?;
        let mut fig = Figure {
            label: family.to_owned(),
            puppet: Puppet::build_masks(&rig, front.as_ref().map(|f| &f.1), side.as_ref().map(|s| &s.1))
                .map_err(|e| e.to_string())?,
            guesses: BTreeMap::new(),
            rig,
            front,
            side,
            poses,
        };
        fig.guesses = guess_depths_mask(&fig.rig, fig.mask(other(fig.rig.view)));
        Ok(fig)
    }

    pub fn mask(&self, view: View) -> Option<&Mask> {
        match view {
            View::Front => self.front.as_ref().map(|f| &f.1),
            View::Side => self.side.as_ref().map(|s| &s.1),
        }
    }

    /// After `rig.depth` changed.
    fn rebuild(&mut self) {
        if let Ok(p) = Puppet::build_masks(&self.rig, self.mask(View::Front), self.mask(View::Side)) {
            self.puppet = p;
        }
    }
}

fn other(v: View) -> View {
    if v == View::Front {
        View::Side
    } else {
        View::Front
    }
}

pub fn load_figures() -> Vec<Figure> {
    let outlines = Outlines::load(&data_dir().join("outlines.json")).expect("data/outlines.json");
    FAMILIES
        .iter()
        .filter_map(|f| Figure::load(&outlines, f).map_err(|e| eprintln!("puppet_demo: {f}: {e}")).ok())
        .collect()
}

/// Everything for one view, back to front: shadow, grid, puppet.
pub fn scene(puppet: &Puppet, pose: &Pose, look: &Look, cam: &Camera, grid: bool) -> Vec<Prim> {
    let posed = puppet.pose(pose);
    let project = |p: V3| cam.project(p);
    let mut prims = posed.ground_shadow(&project, look, SHADOW);
    if grid {
        prims.extend(ground_grid(&project, 5, GRID, 1.0));
    }
    prims.extend(posed.draw_list(&project, look));
    prims
}

// ------------------------------------------------------------------ app

struct Demo {
    figures: Vec<Figure>,
    figure: usize,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    auto_rotate: bool,
    pose: Pose,
    from: Pose,
    to: Pose,
    blend_ms: f32,
    blend_started: Option<Instant>,
    blend_t: f32,
    bone: String,
    bone_euler: [f32; 3],
    wounds: BTreeMap<&'static str, u8>,
    dead: bool,
    boss: bool,
    body: [u8; 3],
    outline_px: f32,
    silhouettes: bool,
    started: Instant,
    last_frame: Instant,
    frames: u64,
    exit_after: Option<f32>,
    last_cost: std::time::Duration,
    last_prims: usize,
}

impl Demo {
    fn new() -> Self {
        let figures = load_figures();
        assert!(!figures.is_empty(), "no figure could be loaded");
        let bone = figures[0].puppet.segments[0].bone.clone();
        Demo {
            figures,
            figure: 0,
            yaw: 35.0,
            pitch: 12.0,
            zoom: 1.0,
            auto_rotate: false,
            pose: Pose::default(),
            from: Pose::default(),
            to: Pose::default(),
            blend_ms: 300.0,
            blend_started: None,
            blend_t: 1.0,
            bone,
            bone_euler: [0.0; 3],
            wounds: BTreeMap::new(),
            dead: false,
            boss: false,
            body: gs_puppet::BODY_DEFAULT,
            outline_px: 1.5,
            silhouettes: true,
            started: Instant::now(),
            last_frame: Instant::now(),
            frames: 0,
            exit_after: std::env::var("PUPPET_DEMO_EXIT_AFTER_SECS").ok().and_then(|s| s.parse().ok()),
            last_cost: Default::default(),
            last_prims: 0,
        }
    }

    fn go_to(&mut self, target: Pose) {
        self.from = self.pose.clone();
        self.to = target;
        self.blend_started = Some(Instant::now());
        self.blend_t = 0.0;
    }

    fn panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("gs_puppet");
        ui.horizontal(|ui| {
            for i in 0..self.figures.len() {
                if ui.selectable_label(self.figure == i, self.figures[i].label.clone()).clicked() && self.figure != i {
                    self.figure = i;
                    self.pose = Pose::default();
                    self.to = Pose::default();
                    self.blend_started = None;
                    self.bone = self.figures[i].puppet.segments[0].bone.clone();
                }
            }
        });
        ui.separator();
        ui.add(egui::Slider::new(&mut self.yaw, -180.0..=180.0).text("yaw (or drag the view)"));
        ui.checkbox(&mut self.auto_rotate, "auto-rotate");
        ui.add(egui::Slider::new(&mut self.pitch, -10.0..=60.0).text("camera pitch"));
        ui.add(egui::Slider::new(&mut self.zoom, 0.1..=2.5).text("zoom"));
        ui.separator();
        ui.label("Pose (from data/poses)");
        let poses: Vec<(String, Pose)> =
            self.figures[self.figure].poses.poses.iter().map(|(n, p)| (n.clone(), p.clone())).collect();
        ui.horizontal_wrapped(|ui| {
            for (name, pose) in poses {
                if ui.button(name).clicked() {
                    self.go_to(pose);
                }
            }
        });
        ui.add(egui::Slider::new(&mut self.blend_ms, 0.0..=1500.0).text("blend ms"));
        if ui.add(egui::Slider::new(&mut self.blend_t, 0.0..=1.0).text("blend")).changed() {
            self.blend_started = None;
            self.pose = self.from.blend(&self.to, self.blend_t);
        }
        ui.separator();
        let bones: Vec<String> = self.figures[self.figure].puppet.bones().iter().map(|b| (*b).to_owned()).collect();
        egui::ComboBox::from_label("bone").selected_text(self.bone.clone()).show_ui(ui, |ui| {
            for b in bones {
                if ui.selectable_value(&mut self.bone, b.clone(), b).clicked() {
                    self.bone_euler = [0.0; 3];
                }
            }
        });
        let mut changed = false;
        for (k, label) in ["x (fore/aft)", "y (twist)", "z (sideways)"].into_iter().enumerate() {
            changed |= ui.add(egui::Slider::new(&mut self.bone_euler[k], -180.0..=180.0).text(label)).changed();
        }
        if changed {
            self.blend_started = None;
            self.pose.set_euler(&self.bone.clone(), self.bone_euler);
            self.to = self.pose.clone();
        }
        ui.separator();
        ui.label("Wounds (3 on an arm / leg severs it)");
        egui::Grid::new("wounds").show(ui, |ui| {
            for part in WOUND_PARTS {
                ui.label(part);
                let rank = self.wounds.entry(part).or_insert(0);
                for r in 0..=3u8 {
                    ui.radio_value(rank, r, r.to_string());
                }
                ui.end_row();
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.dead, "dead");
            ui.checkbox(&mut self.boss, "boss");
            ui.color_edit_button_srgb(&mut self.body);
            ui.label("body");
        });
        ui.add(egui::Slider::new(&mut self.outline_px, 0.0..=3.0).text("outline px"));
        ui.checkbox(&mut self.silhouettes, "silhouette inset");
        ui.separator();
        ui.collapsing("Joint depth: guess vs override", |ui| {
            let fig = &mut self.figures[self.figure];
            let axis = if fig.rig.view == View::Front { "z" } else { "x" };
            let mut dirty = false;
            let names: Vec<String> = fig.rig.joints.keys().cloned().collect();
            for name in names {
                let guess = fig.guesses.get(&name).map(|g| g.value).unwrap_or(0.0);
                let mut on = fig.rig.depth.contains_key(&name);
                let mut value = fig.rig.depth.get(&name).copied().unwrap_or(guess);
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut on, "").changed() {
                        dirty = true;
                    }
                    if ui.add_enabled(on, egui::Slider::new(&mut value, -2.0..=2.0).text(format!("{name} {axis} (guess {guess:+.2})"))).changed() {
                        dirty = true;
                    }
                });
                if on {
                    fig.rig.depth.insert(name, value);
                } else {
                    fig.rig.depth.remove(&name);
                }
            }
            if dirty {
                fig.rebuild();
            }
        });
        ui.separator();
        ui.label(format!("{} prims, pose + list {:?}", self.last_prims, self.last_cost));
    }

    fn silhouette_inset(&self, painter: &egui::Painter, rect: egui::Rect) {
        let fig = &self.figures[self.figure];
        let mut left = rect.left() + 12.0;
        for (entry, label) in [(&fig.front, "front"), (&fig.side, "side")] {
            let Some((entry, _)) = entry else { continue };
            let Some(b) = entry.bounds() else { continue };
            let scale = 150.0 / (b[3] - b[1]).max(0.1);
            let width = (b[2] - b[0]) * scale;
            let base = egui::pos2(left - b[0] * scale, rect.bottom() - 24.0);
            for sub in &entry.subpaths {
                let pts = sub.iter().map(|p| egui::pos2(base.x + p[0] * scale, base.y - p[1] * scale)).collect();
                painter.add(egui::Shape::closed_line(pts, egui::Stroke::new(1.0, egui::Color32::from_gray(190))));
            }
            painter.text(
                egui::pos2(left, rect.bottom() - 20.0),
                egui::Align2::LEFT_TOP,
                label,
                egui::FontId::proportional(12.0),
                egui::Color32::from_gray(190),
            );
            left += width + 18.0;
        }
    }
}

impl eframe::App for Demo {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let dt = self.last_frame.elapsed().as_secs_f32();
        self.last_frame = Instant::now();
        self.frames += 1;
        if self.auto_rotate {
            self.yaw = (self.yaw + 40.0 * dt + 180.0).rem_euclid(360.0) - 180.0;
            ctx.request_repaint();
        }
        if let Some(t0) = self.blend_started {
            let t = if self.blend_ms <= 1.0 { 1.0 } else { (t0.elapsed().as_secs_f32() * 1000.0 / self.blend_ms).min(1.0) };
            // Smoothstep, so the motion eases in and out.
            self.blend_t = t;
            self.pose = self.from.blend(&self.to, t * t * (3.0 - 2.0 * t));
            if t >= 1.0 {
                self.blend_started = None;
            }
            ctx.request_repaint();
        }

        egui::Panel::left("puppet_controls").show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.panel(ui));
        });
        egui::CentralPanel::default().show(ui, |ui| {
            let (response, painter) = ui.allocate_painter(ui.available_size(), egui::Sense::drag());
            let rect = response.rect;
            if response.dragged() {
                let d = response.drag_delta();
                self.yaw = (self.yaw + d.x * 0.5 + 180.0).rem_euclid(360.0) - 180.0;
                self.pitch = (self.pitch + d.y * 0.3).clamp(-10.0, 60.0);
            }
            painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(BACKGROUND[0], BACKGROUND[1], BACKGROUND[2]));
            let fig = &self.figures[self.figure];
            let mut cam = Camera::framing(fig.puppet.height.max(5.0), [rect.width(), rect.height()]);
            cam.center = [rect.center().x, rect.center().y];
            cam.focal *= self.zoom;
            cam.pitch_deg = self.pitch;
            let wounds: Vec<(&str, u8)> = self.wounds.iter().filter(|(_, r)| **r > 0).map(|(p, r)| (*p, *r)).collect();
            let look = Look {
                body: self.body,
                wounds: &wounds,
                dead: self.dead,
                boss_outline: self.boss,
                yaw: self.yaw,
                outline_px: self.outline_px,
                ..Look::default()
            };
            let started = Instant::now();
            let posed = fig.puppet.pose(&self.pose);
            let project = |p: V3| cam.project(p);
            let puppet = posed.draw_list(&project, &look);
            self.last_cost = started.elapsed();
            self.last_prims = puppet.len();
            let painter = painter.with_clip_rect(rect);
            gs_puppet::paint(&painter, egui::Vec2::ZERO, &posed.ground_shadow(&project, &look, SHADOW));
            gs_puppet::paint(&painter, egui::Vec2::ZERO, &ground_grid(&project, 5, GRID, 1.0));
            gs_puppet::paint(&painter, egui::Vec2::ZERO, &puppet);
            if self.silhouettes {
                self.silhouette_inset(&painter, rect);
            }
        });

        if self.frames == 1 || self.frames == 60 {
            eprintln!("puppet_demo: frame {} ok, {} prims, {:?}", self.frames, self.last_prims, self.last_cost);
        }
        if let Some(secs) = self.exit_after {
            // Exercise the blend, both families and the looks while unattended.
            if self.blend_started.is_none() && self.frames.is_multiple_of(40) {
                let step = (self.frames / 40) as usize;
                if step.is_multiple_of(4) {
                    self.figure = (self.figure + 1) % self.figures.len();
                    self.pose = Pose::default();
                }
                self.boss = step % 3 == 0;
                self.dead = step % 5 == 0;
                let poses = &self.figures[self.figure].poses.poses;
                let next = poses.values().nth(step % poses.len()).cloned().unwrap_or_default();
                self.go_to(next);
            }
            if self.started.elapsed().as_secs_f32() >= secs {
                eprintln!("puppet_demo: {} frames, exiting cleanly", self.frames);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ctx.request_repaint();
        }
    }
}

fn main() -> eframe::Result {
    if let Ok(dir) = std::env::var("PUPPET_DEMO_SHOTS") {
        shots::run(Path::new(&dir));
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("gs_puppet demo").with_inner_size([1200.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native("gs_puppet_demo", options, Box::new(|_cc| Ok(Box::new(Demo::new()))))
}

// Shared with `shots`.
pub fn iou_report(fig: &Figure) -> Vec<(View, f32)> {
    [View::Front, View::Side]
        .into_iter()
        .filter_map(|v| Some((v, rest_iou(&fig.puppet, v, fig.mask(v)?))))
        .collect()
}

pub fn overlay_prims(fig: &Figure, view: View) -> Option<Vec<Prim>> {
    Some(rest_prims(&fig.puppet, view, fig.mask(view)?, &Look::default()))
}
