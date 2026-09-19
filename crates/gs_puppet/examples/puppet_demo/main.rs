//! puppet_demo: the capsule puppet in a bare eframe window.
//!
//!   cargo run -p gs_puppet --example puppet_demo
//!
//! `PUPPET_DEMO_EXIT_AFTER_SECS=<n>` closes the window by itself (smoke runs).
//! `PUPPET_DEMO_SHOTS=<dir>` opens NO window: it renders a fixed set of views
//! with the crate's software rasteriser and writes them as PNGs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use gs_puppet::draw::{BODY_ALIVE, BODY_DEAD};
use gs_puppet::raster::Canvas;
use gs_puppet::{build_draw_list, ground_grid, ground_shadow, solve, Camera, Pose, Prim, Skeleton, Style, PRESETS};
use rig::{OutlineEntry, Outlines, Rig, View};

mod canine;

const BACKGROUND: [u8; 3] = [0x2b, 0x30, 0x36];
const GRID: [u8; 4] = [0x4a, 0x52, 0x5c, 255];
const SHADOW: [u8; 4] = [0x1d, 0x21, 0x26, 255];
const WOUND_PARTS: [&str; 11] = [
    "head", "neck", "chest", "abdomen", "leftArm", "rightArm", "leftHand", "rightHand", "leftLeg", "rightLeg",
    "back",
];

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

pub struct Figure {
    pub label: &'static str,
    pub skel: Skeleton,
    pub presets: Vec<(&'static str, Pose)>,
    pub front: OutlineEntry,
    pub side: OutlineEntry,
}

fn load_figures() -> Vec<Figure> {
    let dir = data_dir();
    let outlines = Outlines::load(&dir.join("outlines.json")).expect("data/outlines.json");
    let rig = Rig::load(&dir.join("rigs/humanoid.rig.toml")).expect("humanoid rig");
    let front = outlines.get("humanoid", View::Front).expect("humanoid-front").clone();
    let side = outlines.get("humanoid", View::Side).expect("humanoid").clone();
    let skel = Skeleton::from_rig(&rig, &front.outline(), &side.outline());
    let presets = PRESETS.iter().map(|n| (*n, Pose::preset(n).unwrap())).collect();
    let mut out = vec![Figure { label: "humanoid", skel, presets, front, side }];
    if let Some(dog) = canine::figure(&outlines) {
        out.push(dog);
    }
    out
}

/// Everything for one view, back to front: shadow, grid, puppet.
fn scene(skel: &Skeleton, pose: &Pose, style: &Style, cam: &Camera) -> Vec<Prim> {
    let posed = solve(skel, pose);
    let project = cam.projector();
    let mut prims = ground_shadow(skel, &posed, style, &project, SHADOW);
    prims.extend(ground_grid(&project, 5, GRID, 1.0));
    prims.extend(build_draw_list(skel, &posed, style, &project));
    prims
}

// ---------------------------------------------------------------- shots

fn write_png(path: &Path, w: usize, h: usize, rgba: &[u8]) {
    let file = std::fs::File::create(path).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut w| w.write_image_data(rgba)).expect("write png");
}

fn shots(dir: &Path) {
    const SIZE: usize = 440;
    std::fs::create_dir_all(dir).expect("shots dir");
    let figures = load_figures();
    let human = &figures[0];
    let wounded = Style {
        injuries: vec![("leftArm".into(), 1), ("chest".into(), 2), ("rightLeg".into(), 3)],
        ..Style::default()
    };
    let mut list: Vec<(String, &Figure, &str, f32, Style)> = vec![
        ("stand_front".into(), human, "stand", 0.0, Style::default()),
        ("stand_34".into(), human, "stand", 35.0, Style::default()),
        ("stand_side".into(), human, "stand", 90.0, Style::default()),
        ("kneel_34".into(), human, "kneel", 35.0, Style::default()),
        ("sit_34".into(), human, "sit", 35.0, Style::default()),
        ("prone_34".into(), human, "prone", 35.0, Style::default()),
        ("fallen_side_34".into(), human, "fallen_side", 35.0, Style::default()),
        ("arms_up_front".into(), human, "arms_up", 0.0, Style::default()),
        ("wounded_34".into(), human, "stand", 35.0, wounded),
        ("supine_34".into(), human, "supine", 35.0, Style::default()),
        ("kneel_side".into(), human, "kneel", 90.0, Style::default()),
        ("sit_side".into(), human, "sit", 90.0, Style::default()),
    ];
    if let Some(dog) = figures.get(1) {
        for (name, _) in &dog.presets {
            list.push((format!("canine_{name}_34"), dog, name, 50.0, Style::default()));
        }
        list.push(("canine_stand_side".into(), dog, "stand", 90.0, Style::default()));
    }

    let cols = 4;
    let rows = list.len().div_ceil(cols);
    let mut sheet = vec![0u8; cols * SIZE * rows * SIZE * 4];
    for (i, (name, fig, preset, yaw, style)) in list.iter().enumerate() {
        let pose = &fig.presets.iter().find(|(n, _)| n == preset).expect("preset").1;
        let mut cam = Camera::framing(fig.skel.height.max(5.0), [SIZE as f32, SIZE as f32]);
        cam.yaw_deg = *yaw;
        let started = Instant::now();
        let prims = scene(&fig.skel, pose, style, &cam);
        let built = started.elapsed();
        let mut canvas = Canvas::new(SIZE, SIZE, 2, BACKGROUND);
        canvas.draw(&prims);
        let rgba = canvas.to_rgba8();
        write_png(&dir.join(format!("{name}.png")), SIZE, SIZE, &rgba);
        let (cx, cy) = (i % cols * SIZE, i / cols * SIZE);
        for y in 0..SIZE {
            let dst = ((cy + y) * cols * SIZE + cx) * 4;
            sheet[dst..dst + SIZE * 4].copy_from_slice(&rgba[y * SIZE * 4..(y + 1) * SIZE * 4]);
        }
        eprintln!("puppet_demo: {name}.png  {} prims, scene built in {built:?}", prims.len());
    }
    write_png(&dir.join("sheet.png"), cols * SIZE, rows * SIZE, &sheet);

    // Per-frame cost, puppet only (FK + grounding + draw list).
    let pose = &human.presets.iter().find(|(n, _)| *n == "kneel").unwrap().1;
    let cam = Camera::framing(human.skel.height, [SIZE as f32, SIZE as f32]);
    let project = cam.projector();
    let n = 2000;
    let started = Instant::now();
    let mut prims = 0;
    for _ in 0..n {
        let posed = solve(&human.skel, pose);
        prims = build_draw_list(&human.skel, &posed, &Style::default(), &project).len();
    }
    eprintln!(
        "puppet_demo: {} segments -> {prims} prims; solve + draw list = {:?} per frame",
        human.skel.segments.len(),
        started.elapsed() / n
    );
    report_depths();
}

/// Print the automatic z guesses so the report can judge them.
fn report_depths() {
    let dir = data_dir();
    let outlines = Outlines::load(&dir.join("outlines.json")).unwrap();
    let rig = Rig::load(&dir.join("rigs/humanoid.rig.toml")).unwrap();
    let side = rig_bake::rasterize(&outlines.get("humanoid", View::Side).unwrap().outline(), gs_puppet::skeleton::MASK_PX);
    for (name, g) in gs_puppet::guess_depths(&rig, &side, gs_puppet::BIPED) {
        eprintln!("  z {name:<12} {:+.3}  {:?}", g.value, g.source);
    }
    let figs = load_figures();
    for s in &figs[0].skel.segments {
        eprintln!("  seg {:<14} r0 {:.2}/{:.2}  r1 {:.2}/{:.2}  parent {:?}", s.name, s.r0[0], s.r0[1], s.r1[0], s.r1[1], s.parent);
    }
}

// ------------------------------------------------------------------ app

struct Demo {
    figures: Vec<Figure>,
    figure: usize,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    distance: f32,
    auto_rotate: bool,
    pose: Pose,
    from: Pose,
    to: Pose,
    blend_ms: f32,
    blend_started: Option<Instant>,
    blend_t: f32,
    selected: String,
    wounds: BTreeMap<&'static str, u8>,
    dead: bool,
    outline: bool,
    highlight: bool,
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
        let selected = figures[0].skel.segments[0].name.clone();
        Demo {
            figures,
            figure: 0,
            yaw: 35.0,
            pitch: 12.0,
            zoom: 1.0,
            distance: 16.0,
            auto_rotate: false,
            pose: Pose::default(),
            from: Pose::default(),
            to: Pose::default(),
            blend_ms: 300.0,
            blend_started: None,
            blend_t: 1.0,
            selected,
            wounds: BTreeMap::new(),
            dead: false,
            outline: true,
            highlight: true,
            silhouettes: false,
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

    fn style(&self) -> Style {
        let base = Style::default();
        Style {
            body: if self.dead { BODY_DEAD } else { BODY_ALIVE },
            outline: self.outline.then_some(base.outline).flatten(),
            highlight: self.highlight,
            injuries: self.wounds.iter().filter(|(_, r)| **r > 0).map(|(p, r)| ((*p).to_owned(), *r)).collect(),
            ..base
        }
    }

    fn panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("gs_puppet");
        if self.figures.len() > 1 {
            ui.horizontal(|ui| {
                for i in 0..self.figures.len() {
                    if ui.selectable_label(self.figure == i, self.figures[i].label).clicked() && self.figure != i {
                        self.figure = i;
                        self.pose = Pose::default();
                        self.to = Pose::default();
                        self.blend_started = None;
                        self.selected = self.figures[i].skel.segments[0].name.clone();
                    }
                }
            });
        }
        ui.separator();
        ui.add(egui::Slider::new(&mut self.yaw, -180.0..=180.0).text("yaw"));
        ui.checkbox(&mut self.auto_rotate, "auto-rotate");
        ui.add(egui::Slider::new(&mut self.pitch, -10.0..=60.0).text("camera pitch"));
        ui.add(egui::Slider::new(&mut self.zoom, 0.4..=2.5).text("zoom"));
        ui.add(egui::Slider::new(&mut self.distance, 6.0..=40.0).text("distance (ft)"));
        ui.separator();
        ui.label("Pose");
        let presets: Vec<(&'static str, Pose)> = self.figures[self.figure].presets.clone();
        ui.horizontal_wrapped(|ui| {
            for (name, pose) in presets {
                if ui.button(name).clicked() {
                    self.go_to(pose);
                }
            }
        });
        ui.add(egui::Slider::new(&mut self.blend_ms, 0.0..=1500.0).text("blend ms"));
        if ui.add(egui::Slider::new(&mut self.blend_t, 0.0..=1.0).text("blend")).changed() {
            self.blend_started = None;
            self.pose = self.from.lerp(&self.to, self.blend_t);
        }
        ui.separator();
        let names: Vec<String> = self.figures[self.figure].skel.segments.iter().map(|s| s.name.clone()).collect();
        egui::ComboBox::from_label("segment").selected_text(self.selected.clone()).show_ui(ui, |ui| {
            for n in names {
                ui.selectable_value(&mut self.selected, n.clone(), n);
            }
        });
        let mut rot = self.pose.get(&self.selected);
        let mut changed = false;
        for (k, label) in ["x (fore/aft)", "y (twist)", "z (sideways)"].into_iter().enumerate() {
            changed |= ui.add(egui::Slider::new(&mut rot[k], -180.0..=180.0).text(label)).changed();
        }
        if changed {
            self.blend_started = None;
            self.pose.rot.insert(self.selected.clone(), rot);
            self.to = self.pose.clone();
        }
        ui.separator();
        ui.label("Wounds (3 on a limb severs it)");
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
        ui.checkbox(&mut self.dead, "dead (grey body)");
        ui.checkbox(&mut self.outline, "capsules + outline");
        ui.checkbox(&mut self.highlight, "highlight");
        ui.checkbox(&mut self.silhouettes, "show front/side silhouettes");
        ui.separator();
        ui.label(format!("{} prims, solve + list {:?}", self.last_prims, self.last_cost));
    }

    fn silhouette_inset(&self, painter: &egui::Painter, rect: egui::Rect) {
        let fig = &self.figures[self.figure];
        let mut left = rect.left() + 12.0;
        for (entry, label) in [(&fig.front, "front"), (&fig.side, "side")] {
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
            self.pose = self.from.lerp(&self.to, t * t * (3.0 - 2.0 * t));
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
            let mut cam = Camera::framing(fig.skel.height.max(5.0), [rect.width(), rect.height()]);
            cam.center = [rect.center().x, rect.center().y];
            cam.focal *= self.zoom * self.distance / cam.distance;
            cam.distance = self.distance;
            cam.yaw_deg = self.yaw;
            cam.pitch_deg = self.pitch;
            let style = self.style();
            let started = Instant::now();
            let posed = solve(&fig.skel, &self.pose);
            let project = cam.projector();
            let puppet = build_draw_list(&fig.skel, &posed, &style, &project);
            self.last_cost = started.elapsed();
            self.last_prims = puppet.len();
            let painter = painter.with_clip_rect(rect);
            gs_puppet::paint::paint(&painter, egui::Vec2::ZERO, &ground_shadow(&fig.skel, &posed, &style, &project, SHADOW));
            gs_puppet::paint::paint(&painter, egui::Vec2::ZERO, &ground_grid(&project, 5, GRID, 1.0));
            gs_puppet::paint::paint(&painter, egui::Vec2::ZERO, &puppet);
            if self.silhouettes {
                self.silhouette_inset(&painter, rect);
            }
        });

        if self.frames == 1 || self.frames == 60 {
            eprintln!("puppet_demo: frame {} ok, {} prims, {:?}", self.frames, self.last_prims, self.last_cost);
        }
        if let Some(secs) = self.exit_after {
            // Exercise the blend while unattended.
            if self.blend_started.is_none() && self.frames.is_multiple_of(45) {
                let presets = &self.figures[self.figure].presets;
                let next = presets[(self.frames / 45) as usize % presets.len()].1.clone();
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
        shots(Path::new(&dir));
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("gs_puppet demo").with_inner_size([1200.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native("gs_puppet_demo", options, Box::new(|_cc| Ok(Box::new(Demo::new()))))
}
