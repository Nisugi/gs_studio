//! torchlight: the v2 lighting pass ([`gs_field_egui::field_gpu`]) on Niffy's
//! "Swamp Torchlight" prototype scene -- the swamp (or desert) backdrop, the
//! sidewinder and the lurk, a torch and a mote of light -- to compare the
//! port with the prototype itself.
//!
//!   cargo run --release -p gs_field_egui --example torchlight [-- <dir>]
//!
//! `<dir>` holds the prototype's files (unzip "Swamp Torchlight.zip";
//! default `target/torchlight`). The night look is v1's own
//! (`vellum_light`), made on the CPU with the prototype's night keyframe;
//! the GPU pass adds the lights.
//!
//! Env: `TORCHLIGHT_BG=desert`; `TORCHLIGHT_T=<s>` freezes time there;
//! `TORCHLIGHT_VIEW=0..4` (lit, light only, normals, depth, albedo);
//! `TORCHLIGHT_WEATHER=storm,mist,rain,sand,snow` (any) turns on the
//! prototype's weather at its defaults;
//! `TORCHLIGHT_PIECES=1` draws the backdrop and each layer as separate
//! callbacks, the stage's way, instead of in one;
//! `TORCHLIGHT_SHOT=<png>` saves the scene at the prototype's 1264x848 and
//! quits (with `TORCHLIGHT_T`, comparable with the prototype frozen at that
//! time, weather off);
//! `TORCHLIGHT_BENCH=1` opens no window: it times the pass on every GPU the
//! machine has (`=all`: on every backend of each), feature by feature, at
//! 1x, 1.5x and 2x the prototype's size, shading every screen pixel and
//! shaded no finer than the art (`FieldFrame::detail`), and prints
//! milliseconds a frame; `TORCHLIGHT_BENCH_PNG=<prefix>` also saves a
//! crowd at 2x both ways (`<prefix>_every.png`, `<prefix>_art.png`);
//! `TORCHLIGHT_DETAIL=<d>` sets the window's detail (0 = every pixel);
//! `TORCHLIGHT_SCALE=<k>` makes the window (and a shot) k times the
//! prototype's size, to see the scene shown bigger than its art.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui;
use gs_field_egui::field_gpu::{self, FieldCamera, FieldFrame, FieldLight, FieldView, LayerDraw, LitImage};

// ---- the prototype's scene (its `P` block)
const SIZE: [f32; 2] = [1264.0, 848.0];
const BG_Z: [f32; 2] = [0.5, 130.0];
const Z_RANGE: [f32; 2] = [-0.5, 2.0];
const NIGHT_KEY: [f32; 3] = [0.7165974271310463, 0.9813295906830981, 2.0194524063073094];
const NIGHT_AMB: [f32; 3] = [0.6880028155437992, 0.9724834788242755, 2.1912800188402555];
const NIGHT_STRENGTH: f32 = 0.9;
const NIGHT_EXPOSURE: f32 = 0.07;
const NIGHT_NIGHT: f32 = 0.85;
const NIGHT_CHROMA: f32 = 1.0;

struct Creature {
    id: &'static str,
    height_m: f32,
    tex: [u32; 2],
    content: [f32; 4],
}
const CREATURES: [Creature; 2] = [
    Creature { id: "sn", height_m: 1.8035, tex: [424, 628], content: [15.0, 112.0, 404.0, 605.5] },
    Creature { id: "lk", height_m: 1.7517, tex: [360, 744], content: [31.0, 34.0, 317.0, 720.0] },
];

struct Backdrop {
    name: &'static str,
    f: f32,
    pitch: f32,
    height: f32,
    bg_frame: [f32; 2],
    creature_frames: [[f32; 2]; 2],
    water: f32,
    feet: [[f32; 2]; 2],
    torch: ([f32; 2], f32),
    mote: ([f32; 2], f32),
    /// Blowing sand's colour, sRGB 0..1.
    dust: [f32; 3],
}
const BACKDROPS: [Backdrop; 2] = [
    Backdrop {
        name: "swamp",
        f: 1164.9304258487598,
        pitch: 0.1381779137443424,
        height: 2.2,
        bg_frame: [0.15608161228259188, 0.8553464393468567],
        creature_frames: [[0.3069610688797599, 0.9079682306597833], [0.23850061269999914, 0.8377149256133828]],
        water: 1.0,
        feet: [[690.0, 730.0], [420.0, 700.0]],
        torch: ([105.0, 690.0], 0.8),
        mote: ([930.0, 357.0], 1.7),
        dust: [0.6622558082974083, 0.55654482237131, 0.4078805974784152],
    },
    Backdrop {
        name: "desert",
        f: 1164.9304258487598,
        pitch: 0.12298864011886135,
        height: 2.2,
        bg_frame: [0.44973804506278103, 0.826015000467486],
        creature_frames: [[0.2953101925768179, 0.9144409837825009], [0.22902167872357349, 0.844524831380564]],
        water: 0.0,
        feet: [[720.0, 735.0], [430.0, 700.0]],
        torch: ([105.0, 700.0], 0.8),
        mote: ([940.0, 385.0], 1.7),
        dust: [0.7946740494322824, 0.6277302186864314, 0.4294282767155073],
    },
];

fn load_rgba(path: &Path) -> (u32, u32, Vec<u8>) {
    let img = image::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())).to_rgba8();
    (img.width(), img.height(), img.into_raw())
}

/// v1's night keyframe (the prototype's `nightPass`: v1's per-pixel
/// function with bounce 0 and exposure under 1) over straight sRGB RGBA.
fn night(rgba: &[u8], w: u32, frame: [f32; 2]) -> Vec<u8> {
    let mut env = vellum_light::LightEnv::from_key("#ffffff");
    env.strength = NIGHT_STRENGTH;
    env.bounce_strength = 0.0;
    env.haze = 0.0;
    env.night = NIGHT_NIGHT;
    let mut d = vellum_light::Derived::new(&env);
    d.key_tint = NIGHT_KEY;
    d.ambient_tint = NIGHT_AMB;
    d.exposure = NIGHT_EXPOSURE;
    d.white = 1.0;
    d.chroma = NIGHT_CHROMA;
    let h = (rgba.len() / 4) as u32 / w;
    let fr = vellum_light::LightFrame { l_lo: frame[0], l_hi: frame[1], y_top: 0.0, y_bot: h as f32 };
    let mut out = rgba.to_vec();
    vellum_light::apply_threads(&mut out, w as usize, &env, &d, &fr, vellum_light::YMap::IDENTITY, &vellum_light::Srgb8Lut::new(), 8);
    out
}

fn lit_image(key: u64, (w, h, albedo): (u32, u32, Vec<u8>), depth: &(u32, u32, Vec<u8>), normal: &(u32, u32, Vec<u8>), z: [f32; 2], frame: [f32; 2]) -> LitImage {
    assert_eq!((depth.0, depth.1, normal.0, normal.1), (w, h, w, h), "maps match the image");
    let night = night(&albedo, w, frame);
    let n = (w * h) as usize;
    let mut lit = vec![0u8; n * 4];
    let mut geo = vec![[0.0f32; 4]; n];
    for i in 0..n {
        let d = [depth.2[i * 4], depth.2[i * 4 + 1], depth.2[i * 4 + 2]];
        let (zz, glow) = field_gpu::unpack_depth(d, z[0], z[1]);
        let nn = field_gpu::unpack_normal([normal.2[i * 4], normal.2[i * 4 + 1], normal.2[i * 4 + 2]]);
        lit[i * 4..i * 4 + 3].copy_from_slice(&night[i * 4..i * 4 + 3]);
        lit[i * 4 + 3] = (glow * 255.0).round() as u8;
        geo[i] = if albedo[i * 4 + 3] > 0 { [nn[0], nn[1], nn[2], zz] } else { [0.0, 0.0, 0.0, zz] };
    }
    LitImage { key, w, h, albedo: Arc::new(albedo), lit: Arc::new(lit), geo: Arc::new(geo) }
}

struct Scene {
    bd: &'static Backdrop,
    camera: FieldCamera,
    backdrop: Arc<LitImage>,
    creatures: Vec<Arc<LitImage>>,
}

fn load(dir: &Path, bd: &'static Backdrop) -> Scene {
    let camera = FieldCamera { size: SIZE, f: bd.f, cx: SIZE[0] / 2.0, cy: SIZE[1] / 2.0, pitch: bd.pitch, height: bd.height };
    let f = |name: String| dir.join(name);
    let bg_rgba = load_rgba(&f(format!("bg_{}_albedo.jpg", bd.name)));
    let bg_depth = load_rgba(&f(format!("bg_{}_depth.png", bd.name)));
    let bg_normal = load_rgba(&f(format!("bg_{}_normal.png", bd.name)));
    let mut backdrop = lit_image(1, bg_rgba, &bg_depth, &bg_normal, BG_Z, bd.bg_frame);
    // a backdrop has no glowing pixels: its depth map's blue is not a mask
    for p in Arc::make_mut(&mut backdrop.lit).chunks_exact_mut(4) {
        p[3] = 0;
    }
    let creatures = CREATURES
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let albedo = load_rgba(&f(format!("{}_albedo_{}.png", c.id, bd.name)));
            let depth = load_rgba(&f(format!("{}_depth.png", c.id)));
            let normal = load_rgba(&f(format!("{}_normal.png", c.id)));
            Arc::new(lit_image(10 + i as u64, albedo, &depth, &normal, Z_RANGE, bd.creature_frames[i]))
        })
        .collect();
    Scene { bd, camera, backdrop: Arc::new(backdrop), creatures }
}

impl Scene {
    /// `TORCHLIGHT_WEATHER=storm,mist,rain,sand,snow` (any of them): the
    /// prototype's weather at its default settings; none by default.
    fn weather(&self, t: f32) -> field_gpu::FieldWeather {
        self.weather_of(t, &std::env::var("TORCHLIGHT_WEATHER").unwrap_or_default())
    }

    /// The prototype's weather: `want` lists the kinds, as the env var does.
    fn weather_of(&self, t: f32, want: &str) -> field_gpu::FieldWeather {
        let on = |w: &str| want.split(',').any(|x| x.trim() == w);
        let s2l = |c: f32| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
        let mut w = field_gpu::FieldWeather { dust: self.bd.dust.map(s2l), ..Default::default() };
        if on("storm") {
            let (l, x) = field_gpu::lightning(t, 7.0, 0.7);
            (w.flash, w.flash_x) = (l * 0.6, x);
        }
        // the night air: the prototype's uFogAmb, with the flash's lift
        let expo = NIGHT_EXPOSURE * (1.0 + 2.2 * w.flash);
        w.fog = [0, 1, 2].map(|k| (NIGHT_AMB[k] + (NIGHT_KEY[k] - NIGHT_AMB[k]) * 0.4) * expo * 0.3);
        if on("mist") {
            (w.mist, w.mist_h, w.mist_drift) = (0.4 * 0.35, 0.8, 0.4);
        }
        if on("rain") {
            (w.rain, w.wind) = (0.5, 0.15);
        }
        if on("sand") {
            (w.sand, w.sand_wind) = (0.35, (3.0 + 15.0 * 0.6) * (0.5 + 0.5 * 0.35 + 0.3));
        }
        if on("snow") {
            (w.snow, w.snow_wind, w.snow_cover) = (0.3, 0.2 * (0.6 + 11.0 * 0.3 * 0.3), 0.45);
        }
        w
    }

    fn frame(&self, t: f32, view: FieldView, shadows: bool, torch: bool, mote: bool) -> FieldFrame {
        let cam = self.camera;
        let up = cam.up();
        let add = |a: [f32; 3], b: [f32; 3], k: f32| [a[0] + b[0] * k, a[1] + b[1] * k, a[2] + b[2] * k];
        // creatures stand at their feet's ground depth; far first
        let mut layers: Vec<LayerDraw> = CREATURES
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let feet = self.bd.feet[i];
                let zf = cam.ground_depth(feet[1]);
                let hpx = c.height_m * cam.f / zf;
                let s = hpx / (c.content[3] - c.content[1]);
                let cx = (c.content[0] + c.content[2]) / 2.0;
                LayerDraw {
                    image: self.creatures[i].clone(),
                    rect: [feet[0] - s * cx, feet[1] - s * c.content[3], c.tex[0] as f32 * s, c.tex[1] as f32 * s],
                    feet_z: zf,
                    tint: 1.0,
                }
            })
            .collect();
        layers.sort_by(|a, b| b.feet_z.total_cmp(&a.feet_z));
        let mut lights = Vec::new();
        if torch {
            let (uv, h) = self.bd.torch;
            let base = cam.point_at_height(uv[0], uv[1], h);
            let (f, sway) = field_gpu::torch_flicker(t, 1.0);
            lights.push(FieldLight::torch(add(base, sway, 1.0), base, h, f, 1.0));
        }
        if mote {
            let (uv, h) = self.bd.mote;
            let m0 = cam.point_at_height(uv[0], uv[1], h);
            let bob = 0.04 * (t * 1.4).sin() + 0.015 * (t * 3.1 + 1.0).sin();
            let drift = 0.02 * (t * 0.7 + 2.0).sin();
            let m = add(add(m0, up, bob), [drift, 0.0, 0.0], 1.0);
            let (f, k) = field_gpu::mote_pulse(t, 0.18, 1.0);
            let colour = field_gpu::mote_colour("#ffd27a", "#fff0c0", "#ffab40", k, 0.6);
            lights.push(FieldLight::mote(m, colour, f, 1.0, 10.0));
        }
        FieldFrame {
            id: FieldFrame::next_id(),
            camera: cam,
            backdrop: self.backdrop.clone(),
            backdrop_rect: [0.0, 0.0, SIZE[0], SIZE[1]],
            layers,
            lights,
            time: t,
            water: self.bd.water,
            shadows,
            view,
            weather: self.weather(t),
            detail: std::env::var("TORCHLIGHT_DETAIL").ok().and_then(|v| v.parse().ok()).unwrap_or(field_gpu::DETAIL),
        }
    }
}

struct App {
    scene: Scene,
    t: f32,
    frozen: Option<f32>,
    view: FieldView,
    shadows: bool,
    /// Backdrop and layers as separate callbacks, the stage's way.
    pieces: bool,
    torch: bool,
    mote: bool,
    shot: Option<PathBuf>,
    frames: u32,
    field_rect: egui::Rect,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        self.t = self.frozen.unwrap_or(self.t + dt);
        if self.shot.is_none() {
            egui::Panel::right("controls").show(ui, |ui| {
                ui.heading("torchlight");
                ui.label(format!("{} · t {:.2} s", self.scene.bd.name, self.t));
                ui.checkbox(&mut self.torch, "Torch");
                ui.checkbox(&mut self.mote, "Mote");
                ui.checkbox(&mut self.shadows, "Shadows");
                ui.checkbox(&mut self.pieces, "Draw in pieces (as the stage)");
                let mut frozen = self.frozen.is_some();
                if ui.checkbox(&mut frozen, "Pause").changed() {
                    self.frozen = frozen.then_some(self.t);
                }
                for (v, label) in [(FieldView::Lit, "Lit"), (FieldView::LightOnly, "Light only"), (FieldView::Normals, "Normals"), (FieldView::Depth, "Depth"), (FieldView::Albedo, "Albedo")] {
                    ui.radio_value(&mut self.view, v, label);
                }
            });
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            let avail = ui.available_rect_before_wrap();
            let scale = (avail.width() / SIZE[0]).min(avail.height() / SIZE[1]);
            let rect = egui::Rect::from_center_size(avail.center(), egui::vec2(SIZE[0], SIZE[1]) * scale);
            self.field_rect = rect;
            if field_gpu::available(&ctx) {
                let frame: field_gpu::SharedFrame = Arc::new(std::sync::Mutex::new(self.scene.frame(self.t, self.view, self.shadows, self.torch, self.mote)));
                let place = field_gpu::Placement { origin: rect.min, scale };
                if self.pieces {
                    // as the stage draws it: the backdrop, then each layer
                    field_gpu::paint(ui.painter(), &frame, place, field_gpu::Piece::Backdrop);
                    let n = frame.lock().map_or(0, |f| f.layers.len().min(field_gpu::MAX_LAYERS));
                    for i in 0..n {
                        field_gpu::paint(ui.painter(), &frame, place, field_gpu::Piece::Layer(i));
                    }
                } else {
                    field_gpu::paint(ui.painter(), &frame, place, field_gpu::Piece::Everything);
                }
            } else {
                ui.label("The field pass needs the wgpu renderer.");
            }
        });
        if let Some(path) = &self.shot {
            self.frames += 1;
            if self.frames == 4 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            let shot = ctx.input(|i| i.events.iter().find_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None }));
            if let Some(image) = shot {
                let ppp = ctx.pixels_per_point();
                let r = self.field_rect;
                let (x0, y0) = ((r.min.x * ppp).round() as u32, (r.min.y * ppp).round() as u32);
                let (w, h) = ((r.width() * ppp).round() as u32, (r.height() * ppp).round() as u32);
                let full = image::RgbaImage::from_raw(image.size[0] as u32, image.size[1] as u32, image.as_raw().to_vec()).expect("screenshot");
                let crop = image::imageops::crop_imm(&full, x0, y0, w.min(full.width() - x0), h.min(full.height() - y0)).to_image();
                let k = scale();
                let out = image::imageops::resize(&crop, (SIZE[0] * k) as u32, (SIZE[1] * k) as u32, image::imageops::FilterType::Triangle);
                out.save(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                println!("wrote {}", path.display());
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ctx.request_repaint();
        } else if self.frozen.is_none() {
            ctx.request_repaint();
        }
    }
}

/// A moon for the bench: at `azimuth` / `elevation` degrees, as the stage
/// places one (`field_gpu_stage::moon_light`), with some cloud.
fn bench_moon(cam: &FieldCamera, azimuth: f32, elevation: f32, colour: [f32; 3], power: f32) -> FieldLight {
    let up = cam.up();
    let deeper = [0.0, up[2], -up[1]];
    let (sa, ca) = azimuth.to_radians().sin_cos();
    let (se, ce) = elevation.to_radians().sin_cos();
    let toward = [0, 1, 2].map(|i| ce * (sa * [1.0, 0.0, 0.0][i] - ca * deeper[i]) + se * up[i]);
    FieldLight::moon(toward, colour, power, 0.3, [3.0, 0.6])
}

/// Run a future to the end on this thread (wgpu's native ones are ready at
/// once or nearly).
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    let mut f = std::pin::pin!(f);
    loop {
        if let std::task::Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}

/// `TORCHLIGHT_BENCH`: the pass timed offscreen, case by case.
fn bench(scene: &Scene, every_backend: bool) {
    use egui_wgpu::wgpu;
    const T: f32 = 5.0;
    let cam = scene.camera;
    // lamps + both moons, the stage's night at its fullest
    let moons = || [bench_moon(&cam, -30.0, 40.0, [0.456, 0.552, 0.834], 0.32), bench_moon(&cam, 50.0, 25.0, [0.42, 0.43, 0.47], 0.1)];
    let base = |torch: bool, mote: bool| scene.frame(T, FieldView::Lit, true, torch, mote);
    let with = |mut f: FieldFrame, moons_n: usize, weather: &str| {
        f.lights.extend(moons().into_iter().take(moons_n));
        f.weather = scene.weather_of(T, weather);
        f
    };
    let mut cases: Vec<(String, FieldFrame)> = vec![
        ("bare (no lights)".into(), with(base(false, false), 0, "")),
        ("torch".into(), with(base(true, false), 0, "")),
        ("torch + mote".into(), with(base(true, true), 0, "")),
        ("+ moon".into(), with(base(true, true), 1, "")),
        ("+ 2 moons".into(), with(base(true, true), 2, "")),
    ];
    let mut no_shadows = with(base(true, true), 2, "");
    no_shadows.shadows = false;
    cases.push(("lamps + 2 moons, shadows off".into(), no_shadows));
    for w in ["rain", "snow", "storm", "sand", "mist"] {
        cases.push((format!("lamps + 2 moons + {w}"), with(base(true, true), 2, w)));
    }
    cases.push(("lamps + 2 moons + all weather".into(), with(base(true, true), 2, "storm,mist,rain,sand,snow")));
    // eight torches across the ground: the cost of each more lamp
    let mut many = with(base(false, false), 2, "");
    for i in 0..8 {
        let u = 120.0 + i as f32 * 140.0;
        let b = cam.point_at_height(u, 640.0 + 20.0 * (i % 3) as f32, 0.8);
        let (f, sway) = field_gpu::torch_flicker(T + i as f32, 1.0);
        many.lights.push(FieldLight::torch([b[0] + sway[0], b[1] + sway[1], b[2] + sway[2]], b, 0.8, f, 1.0));
    }
    cases.push(("8 torches + 2 moons".into(), many));
    // a crowd: the two creatures again at six more spots, as a busy stage
    let crowd = |mut f: FieldFrame| {
        for (i, feet) in [[230.0, 650.0], [360.0, 790.0], [560.0, 640.0], [850.0, 690.0], [1000.0, 610.0], [1120.0, 780.0]].into_iter().enumerate() {
            let c = &CREATURES[i % 2];
            let zf = cam.ground_depth(feet[1]);
            let k = c.height_m * cam.f / zf / (c.content[3] - c.content[1]);
            let cx = (c.content[0] + c.content[2]) / 2.0;
            f.layers.push(LayerDraw {
                image: scene.creatures[i % 2].clone(),
                rect: [feet[0] - k * cx, feet[1] - k * c.content[3], c.tex[0] as f32 * k, c.tex[1] as f32 * k],
                feet_z: zf,
                tint: 1.0,
            });
        }
        f.layers.sort_by(|a, b| b.feet_z.total_cmp(&a.feet_z));
        f
    };
    cases.push(("8 creatures, lamps + 2 moons".into(), crowd(with(base(true, true), 2, ""))));
    cases.push(("8 creatures, lamps + 2 moons + snow".into(), crowd(with(base(true, true), 2, "snow"))));
    let mut crowd_off = crowd(with(base(true, true), 2, ""));
    crowd_off.shadows = false;
    cases.push(("8 creatures, shadows off".into(), crowd_off));

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapters = block_on(instance.enumerate_adapters(wgpu::Backends::all()));
    let mut seen = std::collections::HashSet::new();
    for a in &adapters {
        let i = a.get_info();
        println!("adapter: {} ({:?}, {:?})", i.name, i.device_type, i.backend);
    }
    let sizes = [1.0f32, 1.5, 2.0].map(|k| [(SIZE[0] * k) as u32, (SIZE[1] * k) as u32]);
    // pictures of the crowd both ways, from the first GPU
    let mut pictures = std::env::var("TORCHLIGHT_BENCH_PNG").ok();
    for adapter in adapters {
        let info = adapter.get_info();
        // software rasterisers take seconds a frame: not a GPU to plan for
        if info.device_type == wgpu::DeviceType::Cpu || (!every_backend && !seen.insert(info.name.clone())) {
            continue;
        }
        let Ok((device, queue)) = block_on(adapter.request_device(&wgpu::DeviceDescriptor { label: Some("torchlight bench"), ..Default::default() })) else {
            println!("\n{}: no device", info.name);
            continue;
        };
        println!("\n== {} ({:?}, {:?}) -- ms a frame", info.name, info.device_type, info.backend);
        for (detail, how) in [(0.0, "every screen pixel shaded"), (field_gpu::DETAIL, "shaded no finer than the art")] {
            println!("\n-- {how}");
            println!("{:<38}{}", "case", sizes.map(|s| format!("{:>12}", format!("{}x{}", s[0], s[1]))).join(""));
            for (label, frame) in &cases {
                let frame = FieldFrame { detail, ..frame.clone() };
                let ms = sizes.map(|s| {
                    let repeats = (40.0 * (SIZE[0] * SIZE[1]) / (s[0] * s[1]) as f32).max(10.0) as u32;
                    field_gpu::time_offscreen(&device, &queue, &frame, s, repeats).as_secs_f64() * 1000.0
                });
                println!("{:<38}{}", label, ms.map(|m| format!("{m:>12.2}")).join(""));
            }
        }
        if let Some(prefix) = pictures.take() {
            let (_, crowd) = cases.iter().find(|(l, _)| l.starts_with("8 creatures, lamps + 2 moons")).expect("the crowd");
            let s = sizes[2];
            for (detail, name) in [(0.0, "every"), (field_gpu::DETAIL, "art")] {
                let frame = FieldFrame { detail, ..crowd.clone() };
                let px = field_gpu::render_offscreen(&device, &queue, &frame, s);
                let path = format!("{prefix}_{name}.png");
                image::RgbaImage::from_raw(s[0], s[1], px).expect("pixels").save(&path).expect("save");
                println!("wrote {path}");
            }
        }
    }
}

/// `TORCHLIGHT_SCALE`: how many times the prototype's size the window is.
fn scale() -> f32 {
    std::env::var("TORCHLIGHT_SCALE").ok().and_then(|v| v.parse::<f32>().ok()).filter(|k| *k > 0.0).unwrap_or(1.0)
}

fn main() -> eframe::Result {
    let dir = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/torchlight"));
    let bd = match std::env::var("TORCHLIGHT_BG").as_deref() {
        Ok("desert") => &BACKDROPS[1],
        _ => &BACKDROPS[0],
    };
    let frozen = std::env::var("TORCHLIGHT_T").ok().and_then(|v| v.parse().ok());
    let view = match std::env::var("TORCHLIGHT_VIEW").ok().and_then(|v| v.parse::<u32>().ok()) {
        Some(1) => FieldView::LightOnly,
        Some(2) => FieldView::Normals,
        Some(3) => FieldView::Depth,
        Some(4) => FieldView::Albedo,
        _ => FieldView::Lit,
    };
    let shot = std::env::var_os("TORCHLIGHT_SHOT").map(PathBuf::from);
    let scene = load(&dir, bd);
    if let Ok(v) = std::env::var("TORCHLIGHT_BENCH") {
        bench(&scene, v.trim() == "all");
        return Ok(());
    }
    let k = scale();
    let size = if shot.is_some() { [SIZE[0] * k, SIZE[1] * k] } else { [SIZE[0] * k + 200.0, SIZE[1] * k] };
    let options = eframe::NativeOptions { viewport: egui::ViewportBuilder::default().with_title("torchlight").with_inner_size(size), ..Default::default() };
    eframe::run_native(
        "torchlight",
        options,
        Box::new(move |cc| {
            if let Some(rs) = cc.wgpu_render_state.as_ref() {
                field_gpu::init(&cc.egui_ctx, rs);
            }
            Ok(Box::new(App {
                scene,
                t: frozen.unwrap_or(0.0),
                frozen,
                view,
                shadows: true,
                pieces: std::env::var_os("TORCHLIGHT_PIECES").is_some(),
                torch: true,
                mote: true,
                shot,
                frames: 0,
                field_rect: egui::Rect::NOTHING,
            }))
        }),
    )
}
