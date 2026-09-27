//! ADDITION (not in VellumFE): the v2 lighting pass ([`crate::field_gpu`])
//! on the stage.
//!
//! `render_creature_field_content` asks [`StageGpu::begin`] for a frame at
//! the top of each draw (None = draw the old way: the pass is off, has no
//! wgpu, or the scene has no backdrop to light). With one, the backdrop is
//! a GPU piece instead of an image, and each creature body -- sprite art or
//! puppet skin -- is pushed as a layer and drawn as its own piece exactly
//! where the old path drew its image, so every egui shape between them
//! (occluders, card overlays, labels) keeps its place. The frame fills as
//! the cards draw; egui prepares the callbacks only after the UI pass, so
//! the backdrop, drawn first, is still shadowed by every creature.
//!
//! Geometry. The stage camera is a true pinhole in stage pixels (the
//! solver's `screen_y(z) = horizon + cam_h * f / z`), unpitched; the pass
//! wants metres. Card sizes are world units at 5 ft each
//! ([`METRES_PER_CARD_UNIT`]), but the solver draws cards `mscale` times
//! the camera's own units (a crowded field shrinks them), so the camera's
//! units are `1.524 / mscale` metres: that keeps a creature's true size and
//! the lights' distances in one scale. A backdrop with `<stem>.depth.png`
//! beside it (a monocular model's relative depth, `tools/depth_maps.py`)
//! is fitted to metres against the flat ground this camera sees
//! ([`backdrop_geo`]); one without is flat ground below the horizon and
//! sky above. A sprite's `<art>_depth.png` / `<art>_normal.png` beside it
//! are made for a figure 1.8 m tall and scaled to the creature's height on
//! the stage; one without is rounded from its outline (a tube profile); a
//! puppet brings its real surface.
//!
//! Lamps. A scene's `lights` become the pass's lights, each kind with its
//! own physics ([`lamp_physics`]: the notes' torch and mote, and campfire,
//! lantern and glow by likeness), its beam if it has one, and a wisp's
//! colour swelling and fading toward its preset's shades
//! ([`field_gpu::MOTE_PRESETS`]). Torches and wisps are drawn by the pass
//! itself (flame, stick, glowing mote); campfires, lanterns and glows light
//! by it but keep their drawn look from `field_lights`.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use gs_field::lighting::{LampKind, PointLight};

use crate::field_gpu::{self, FieldCamera, FieldFrame, FieldLight, FieldView, FieldWeather, LayerDraw, LitImage, Piece, Placement, SharedFrame};

/// Metres in a card (world) unit: `gs_field::cards::UNITS_PER_FOOT` is 0.2.
pub const METRES_PER_CARD_UNIT: f32 = 1.524;
/// A dead sprite's grey (the old path tints dead art sRGB 110/255), linear.
/// A puppet's skin is a corpse's colour already (`gs_puppet`), so it takes
/// none.
const DEAD_TINT: f32 = 0.155;
/// A creature depth map's range, metres from its feet (the prototype's
/// `z_range`; lights-and-weather.md "Geometry from depth maps").
const CREATURE_Z: [f32; 2] = [-0.5, 2.0];
/// How far a sprite without a depth map bulges toward the viewer at its
/// middle, metres (for a figure [`NOMINAL_FIGURE_M`] tall, scaled to the
/// creature's own), and how wide its rounded rim is (share of its height).
const PROFILE_RELIEF_M: f32 = 0.3;
/// The height a sprite's depth map (and the bulge of one without) is made
/// for, metres: the prototype's figures, `tools/depth_maps.py`'s.
const NOMINAL_FIGURE_M: f32 = 1.8;
/// A backdrop's relative depth is fitted to the flat ground this near and
/// no nearer than [`FIT_NEAREST_M`], metres; past [`SKY_M`] is sky (the
/// shader's own test).
const FIT_FARTHEST_M: f32 = 25.0;
const FIT_NEAREST_M: f32 = 0.5;
const SKY_M: f32 = 60.0;
/// Neighbouring backdrop pixels this far apart in depth (a share of it) are
/// across an edge, not one surface: at the painting's own size, far ground
/// changes a few percent a row.
const EDGE_JUMP: f32 = 0.15;
const PROFILE_RIM: f32 = 0.06;
/// Cached lit images (backdrops, sprites, skins) kept at most.
const CACHE_MAX: usize = 96;
/// A layer is uploaded at about the size it is drawn (screen pixels, this
/// much over, in steps of [`SIZE_STEP`]) and never above [`LAYER_MAX`] on
/// its long side: art is often far larger than a card, and the atlas holds
/// one frame's layers (a layer it has no room for is drawn the old way).
const OVERSAMPLE: f32 = 1.25;
const SIZE_STEP: f32 = 64.0;
const LAYER_MAX: f32 = 1024.0;

fn enabled_id() -> egui::Id {
    egui::Id::new("gs_field_egui::field_gpu_stage::enabled")
}

/// Turn the v2 pass on the stage on or off (a host setting; off by
/// default). It only draws where [`field_gpu::available`].
pub fn set_enabled(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(enabled_id(), on));
}

/// The pass is wanted and can draw.
pub fn enabled(ctx: &egui::Context) -> bool {
    field_gpu::available(ctx) && ctx.data(|d| d.get_temp::<bool>(enabled_id()).unwrap_or(false))
}

// ------------------------------------------------------------------ cache

#[derive(Default)]
struct Cache {
    images: HashMap<u64, Arc<LitImage>>,
    order: Vec<u64>,
}

fn cache() -> &'static Mutex<Cache> {
    static C: OnceLock<Mutex<Cache>> = OnceLock::new();
    C.get_or_init(Mutex::default)
}

fn cached(key: u64, make: impl FnOnce() -> Option<LitImage>) -> Option<Arc<LitImage>> {
    if let Some(img) = cache().lock().ok()?.images.get(&key) {
        return Some(img.clone());
    }
    let img = Arc::new(make()?);
    let mut c = cache().lock().ok()?;
    c.images.insert(key, img.clone());
    c.order.push(key);
    while c.order.len() > CACHE_MAX {
        let old = c.order.remove(0);
        c.images.remove(&old);
    }
    Some(img)
}

/// [`scene_air`] for a backdrop, worked out once.
fn air_of(key: u64, px: &crate::creature_light::LitPixels) -> ([f32; 3], [f32; 3]) {
    static AIR: OnceLock<Mutex<HashMap<u64, ([f32; 3], [f32; 3])>>> = OnceLock::new();
    let map = AIR.get_or_init(Mutex::default);
    if let Some(a) = map.lock().ok().and_then(|m| m.get(&key).copied()) {
        return a;
    }
    let a = scene_air(px);
    if let Ok(mut m) = map.lock() {
        if m.len() > 32 {
            m.clear();
        }
        m.insert(key, a);
    }
    a
}

fn key_of(parts: impl Hash) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut h);
    h.finish()
}

// ------------------------------------------------------------------ sizes

/// The size to upload an image of `w` x `h` drawn `drawn_h` screen pixels
/// tall at: about the drawn size, never larger than the image.
fn upload_size(w: usize, h: usize, drawn_h: f32) -> (usize, usize) {
    let want = ((drawn_h * OVERSAMPLE / SIZE_STEP).ceil() * SIZE_STEP).max(SIZE_STEP);
    let k = (want / h as f32).min(LAYER_MAX / w.max(h) as f32).min(1.0);
    (((w as f32 * k).round() as usize).max(1), ((h as f32 * k).round() as usize).max(1))
}

/// Box-filter straight RGBA down to `nw` x `nh`, colour weighted by alpha
/// (no dark fringe where the art meets transparency).
fn downsample(rgba: &[u8], w: usize, h: usize, nw: usize, nh: usize) -> Vec<u8> {
    if (nw, nh) == (w, h) {
        return rgba.to_vec();
    }
    let mut out = vec![0u8; nw * nh * 4];
    for y in 0..nh {
        let (y0, y1) = (y * h / nh, ((y + 1) * h / nh).max(y * h / nh + 1));
        for x in 0..nw {
            let (x0, x1) = (x * w / nw, ((x + 1) * w / nw).max(x * w / nw + 1));
            let (mut c, mut a, mut n) = ([0.0f32; 3], 0.0f32, 0.0f32);
            for sy in y0..y1.min(h) {
                for sx in x0..x1.min(w) {
                    let p = &rgba[(sy * w + sx) * 4..][..4];
                    let al = p[3] as f32;
                    for k in 0..3 {
                        c[k] += p[k] as f32 * al;
                    }
                    a += al;
                    n += 1.0;
                }
            }
            let o = (y * nw + x) * 4;
            if a > 0.0 {
                for k in 0..3 {
                    out[o + k] = (c[k] / a).round() as u8;
                }
            }
            out[o + 3] = (a / n.max(1.0)).round() as u8;
        }
    }
    out
}

/// Geometry down to `nw` x `nh`: normals and depth averaged over the
/// covered pixels of each box (`alpha` says which), normals renormalised.
fn downsample_geo(geo: &[[f32; 4]], alpha: &[u8], w: usize, h: usize, nw: usize, nh: usize) -> Vec<[f32; 4]> {
    if (nw, nh) == (w, h) {
        return geo.to_vec();
    }
    let mut out = vec![[0.0f32; 4]; nw * nh];
    for y in 0..nh {
        let (y0, y1) = (y * h / nh, ((y + 1) * h / nh).max(y * h / nh + 1));
        for x in 0..nw {
            let (x0, x1) = (x * w / nw, ((x + 1) * w / nw).max(x * w / nw + 1));
            let (mut acc, mut n) = ([0.0f32; 4], 0.0f32);
            for sy in y0..y1.min(h) {
                for sx in x0..x1.min(w) {
                    let i = sy * w + sx;
                    if alpha[i] > 0 {
                        for k in 0..4 {
                            acc[k] += geo[i][k];
                        }
                        n += 1.0;
                    }
                }
            }
            if n > 0.0 {
                let l = (acc[0] * acc[0] + acc[1] * acc[1] + acc[2] * acc[2]).sqrt().max(1e-6);
                out[y * nw + x] = [acc[0] / l, acc[1] / l, acc[2] / l, acc[3] / n];
            }
        }
    }
    out
}

// ------------------------------------------------------------------ geometry

/// Flat ground below the horizon, sky above, for a backdrop image laid over
/// `rect` (scene pixels).
pub fn ground_geo(camera: &FieldCamera, w: usize, h: usize, rect: [f32; 4]) -> Vec<[f32; 4]> {
    let up = camera.up();
    let mut geo = Vec::with_capacity(w * h);
    for j in 0..h {
        let sy = rect[1] + (j as f32 + 0.5) * rect[3] / h as f32;
        let z = camera.ground_depth(sy);
        let g = if z.is_finite() && z < 100.0 { [up[0], up[1], up[2], z] } else { [0.0, 0.0, -1.0, 100.0] };
        geo.extend(std::iter::repeat_n(g, w));
    }
    geo
}

/// A backdrop's relative depth (`<stem>.depth.png`, [`depth_path`]; made by
/// `tools/depth_maps.py`): per pixel how near, 0..1, from a monocular model
/// -- inverse depth up to an unknown scale and shift.
#[derive(Debug, Clone, PartialEq)]
pub struct RelDepth {
    pub w: usize,
    pub h: usize,
    pub near: Vec<f32>,
}

impl RelDepth {
    /// From the PNG: 16 bit in R (high byte) and G.
    pub fn from_png(bytes: &[u8]) -> Result<RelDepth, String> {
        let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?.to_rgb8();
        let (w, h) = (img.width() as usize, img.height() as usize);
        let near = img.pixels().map(|p| (p.0[0] as f32 * 256.0 + p.0[1] as f32) / 65535.0).collect();
        Ok(RelDepth { w, h, near })
    }

    /// Bilinear, at `u`, `v` across it (0..1).
    fn at(&self, u: f32, v: f32) -> f32 {
        let (x, y) = ((u * self.w as f32 - 0.5).clamp(0.0, (self.w - 1) as f32), (v * self.h as f32 - 0.5).clamp(0.0, (self.h - 1) as f32));
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let p = |x: usize, y: usize| self.near[y * self.w + x];
        let top = p(x0, y0) + (p(x1, y0) - p(x0, y0)) * fx;
        let bottom = p(x0, y1) + (p(x1, y1) - p(x0, y1)) * fx;
        top + (bottom - top) * fy
    }
}

/// `v` (`w` x `h`) under a 1-4-6-4-1 kernel along rows, then columns.
fn smooth(v: &[f32], w: usize, h: usize) -> Vec<f32> {
    const K: [f32; 5] = [1.0 / 16.0, 4.0 / 16.0, 6.0 / 16.0, 4.0 / 16.0, 1.0 / 16.0];
    let pass = |src: &[f32], along_rows: bool| {
        let mut out = vec![0.0f32; src.len()];
        for j in 0..h {
            for i in 0..w {
                out[j * w + i] = K
                    .iter()
                    .enumerate()
                    .map(|(t, k)| {
                        let d = t as isize - 2;
                        let (x, y) = if along_rows { ((i as isize + d).clamp(0, w as isize - 1) as usize, j) } else { (i, (j as isize + d).clamp(0, h as isize - 1) as usize) };
                        k * src[y * w + x]
                    })
                    .sum();
            }
        }
        out
    };
    pass(&pass(v, true), false)
}

/// Where a backdrop's relative depth lives: `<stem>.depth.png` beside it.
pub fn depth_path(background: &str) -> String {
    let stem = background.rsplit_once('.').map_or(background, |(s, _)| s);
    format!("{stem}.depth.png")
}

/// A backdrop's geometry from its relative depth, in metres: the model's
/// nearness `n` is `a / z + b` for true depth `z`; `a` and `b` are fitted
/// on the painting's lower part against the flat ground this camera sees
/// there (rows [`FIT_NEAREST_M`]..[`FIT_FARTHEST_M`] away), twice more
/// without the pixels that disagree most (a rock, a trunk: not ground).
/// Then every pixel has its depth; what the fit puts past [`SKY_M`], or
/// no nearer than the far limit, is sky. Normals from the surface in 3D.
/// None when the fit fails (too little ground, or nearness that does not
/// grow toward the camera): the flat ground stands in.
pub fn backdrop_geo(camera: &FieldCamera, w: usize, h: usize, rect: [f32; 4], rel: &RelDepth) -> Option<Vec<[f32; 4]>> {
    if w < 3 || h < 3 || rel.w == 0 || rel.h == 0 {
        return None;
    }
    let near: Vec<f32> = (0..h).flat_map(|j| (0..w).map(move |i| (i, j))).map(|(i, j)| rel.at((i as f32 + 0.5) / w as f32, (j as f32 + 0.5) / h as f32)).collect();
    // A monocular model works at about 518 px and its depth is scaled up:
    // a ripple every few pixels, which slopes turn into streaks under a
    // grazing lamp. Smoothed away first (a 1-4-6-4-1 kernel, twice, each way).
    let near = smooth(&smooth(&near, w, h), w, h);
    let row_y = |j: usize| rect[1] + (j as f32 + 0.5) * rect[3] / h as f32;
    let col_x = |i: usize| rect[0] + (i as f32 + 0.5) * rect[2] / w as f32;
    // the fit: nearness against 1 / ground depth, on the near ground
    let step = (w / 160).max(1);
    let mut pairs: Vec<(f32, f32)> = Vec::new();
    for j in 0..h {
        let z = camera.ground_depth(row_y(j));
        if z.is_finite() && (FIT_NEAREST_M..FIT_FARTHEST_M).contains(&z) {
            pairs.extend((0..w).step_by(step).map(|i| (1.0 / z, near[j * w + i])));
        }
    }
    let fit = |pairs: &[(f32, f32)]| {
        let n = pairs.len() as f64;
        let (sx, sy) = pairs.iter().fold((0.0f64, 0.0f64), |(a, b), (x, y)| (a + *x as f64, b + *y as f64));
        let (mx, my) = (sx / n, sy / n);
        let (sxx, sxy) = pairs.iter().fold((0.0f64, 0.0f64), |(a, b), (x, y)| (a + (*x as f64 - mx).powi(2), b + (*x as f64 - mx) * (*y as f64 - my)));
        let a = sxy / sxx.max(1e-12);
        (a as f32, (my - a * mx) as f32)
    };
    if pairs.len() < 64 {
        return None;
    }
    let (mut a, mut b) = fit(&pairs);
    for _ in 0..2 {
        let mut res: Vec<f32> = pairs.iter().map(|(x, y)| (y - (a * x + b)).abs()).collect();
        res.sort_by(f32::total_cmp);
        let cut = res[res.len() * 7 / 10].max(1e-4) * 2.0;
        let kept: Vec<(f32, f32)> = pairs.iter().copied().filter(|(x, y)| (y - (a * x + b)).abs() <= cut).collect();
        if kept.len() < 32 {
            break;
        }
        (a, b) = fit(&kept);
    }
    if !(a > 1e-4) {
        return None;
    }
    let depth: Vec<f32> = near.iter().map(|&n| if n - b > 1e-5 { (a / (n - b)).clamp(0.3, 100.0) } else { 100.0 }).collect();
    let sky = |z: f32| z > SKY_M;
    let point = |i: usize, j: usize| {
        let z = depth[j * w + i];
        let r = camera.ray(col_x(i), row_y(j));
        [r[0] * z, r[1] * z, r[2] * z]
    };
    let mut geo = vec![[0.0f32, 0.0, -1.0, 100.0]; w * h];
    for j in 0..h {
        for i in 0..w {
            let z = depth[j * w + i];
            if sky(z) {
                continue;
            }
            // central differences, one-sided across a jump (an edge: more
            // than EDGE_JUMP of the depth) or the sky
            let same = |ii: usize, jj: usize| {
                let q = depth[jj * w + ii];
                !sky(q) && (q - z).abs() < EDGE_JUMP * z
            };
            let (l, r) = (i.saturating_sub(1), (i + 1).min(w - 1));
            let (u, d) = (j.saturating_sub(1), (j + 1).min(h - 1));
            let (l, r) = match (same(l, j), same(r, j)) {
                (true, true) => (l, r),
                (true, false) => (l, i),
                (false, true) => (i, r),
                _ => (i, i),
            };
            let (u, d) = match (same(i, u), same(i, d)) {
                (true, true) => (u, d),
                (true, false) => (u, j),
                (false, true) => (j, d),
                _ => (j, j),
            };
            let p = point(i, j);
            let mut n = if l == r || u == d {
                [0.0, 0.0, -1.0]
            } else {
                let (pl, pr, pu, pd) = (point(l, j), point(r, j), point(i, u), point(i, d));
                let dx = [pr[0] - pl[0], pr[1] - pl[1], pr[2] - pl[2]];
                let dy = [pd[0] - pu[0], pd[1] - pu[1], pd[2] - pu[2]];
                let c = [dx[1] * dy[2] - dx[2] * dy[1], dx[2] * dy[0] - dx[0] * dy[2], dx[0] * dy[1] - dx[1] * dy[0]];
                let len = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
                if len > 1e-9 {
                    c.map(|v| v / len)
                } else {
                    [0.0, 0.0, -1.0]
                }
            };
            // facing the camera
            if n[0] * p[0] + n[1] * p[1] + n[2] * p[2] > 0.0 {
                n = n.map(|v| -v);
            }
            geo[j * w + i] = [n[0], n[1], n[2], z];
        }
    }
    Some(geo)
}

/// A sprite with no depth map, rounded from its outline: the pixels a rim's
/// width in from the edge turn toward it on a quarter circle, the middle
/// faces the viewer and bulges toward it by [`PROFILE_RELIEF_M`].
pub fn profile_geo(rgba: &[u8], w: usize, h: usize) -> Vec<[f32; 4]> {
    let inside: Vec<bool> = rgba.chunks_exact(4).map(|p| p[3] > 127).collect();
    // chamfer distance to the outside, pixels (3-4 weights / 3)
    let big = u32::MAX / 4;
    let mut d: Vec<u32> = inside.iter().map(|&i| if i { big } else { 0 }).collect();
    let at = |d: &Vec<u32>, x: isize, y: isize| -> u32 {
        if x < 0 || y < 0 || x >= w as isize || y >= h as isize {
            0
        } else {
            d[y as usize * w + x as usize]
        }
    };
    for y in 0..h as isize {
        for x in 0..w as isize {
            let i = y as usize * w + x as usize;
            if d[i] == 0 {
                continue;
            }
            let m = (at(&d, x - 1, y) + 3).min(at(&d, x, y - 1) + 3).min(at(&d, x - 1, y - 1) + 4).min(at(&d, x + 1, y - 1) + 4);
            d[i] = d[i].min(m);
        }
    }
    for y in (0..h as isize).rev() {
        for x in (0..w as isize).rev() {
            let i = y as usize * w + x as usize;
            if d[i] == 0 {
                continue;
            }
            let m = (at(&d, x + 1, y) + 3).min(at(&d, x, y + 1) + 3).min(at(&d, x + 1, y + 1) + 4).min(at(&d, x - 1, y + 1) + 4);
            d[i] = d[i].min(m);
        }
    }
    let rows: Vec<usize> = (0..h).filter(|&y| inside[y * w..(y + 1) * w].iter().any(|&b| b)).collect();
    let content_h = rows.last().zip(rows.first()).map_or(h, |(b, t)| b - t + 1) as f32;
    let rim = (content_h * PROFILE_RIM).max(3.0);
    // height toward the viewer, pixels, on a quarter circle across the rim
    let hgt: Vec<f32> = d
        .iter()
        .map(|&v| {
            let t = (v as f32 / 3.0 / rim).min(1.0);
            rim * (1.0 - (1.0 - t) * (1.0 - t)).sqrt()
        })
        .collect();
    let hh = |x: usize, y: usize| hgt[y * w + x];
    let mut geo = vec![[0.0f32; 4]; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if !inside[i] {
                continue;
            }
            let dx = (hh((x + 1).min(w - 1), y) - hh(x.saturating_sub(1), y)) * 0.5;
            let dy = (hh(x, (y + 1).min(h - 1)) - hh(x, y.saturating_sub(1))) * 0.5;
            let (dx, dy) = (dx.clamp(-3.0, 3.0), dy.clamp(-3.0, 3.0));
            let l = (dx * dx + dy * dy + 1.0).sqrt();
            geo[i] = [-dx / l, -dy / l, -1.0 / l, -PROFILE_RELIEF_M * hgt[i] / rim];
        }
    }
    geo
}

/// A creature's depth and normal maps (the prototype's packing: depth 16
/// bit in R and G over [`CREATURE_Z`] metres from the feet, B = glow; the
/// normal map RGB = xyz * 0.5 + 0.5). Returns the geometry and the glow per
/// pixel, or None when a map is missing, unreadable or another size.
fn maps_geo(depth: &std::path::Path, normal: &std::path::Path, w: usize, h: usize) -> Option<(Vec<[f32; 4]>, Vec<u8>)> {
    let d = image::open(depth).ok()?.to_rgb8();
    let n = image::open(normal).ok()?.to_rgb8();
    if (d.width() as usize, d.height() as usize) != (w, h) || (n.width() as usize, n.height() as usize) != (w, h) {
        tracing::warn!("field lighting: {} / {} are not {w}x{h}, the art's size", depth.display(), normal.display());
        return None;
    }
    let mut geo = Vec::with_capacity(w * h);
    let mut glow = Vec::with_capacity(w * h);
    for (dp, np) in d.pixels().zip(n.pixels()) {
        let (z, g) = field_gpu::unpack_depth(dp.0, CREATURE_Z[0], CREATURE_Z[1]);
        let nn = field_gpu::unpack_normal(np.0);
        geo.push([nn[0], nn[1], nn[2], z]);
        glow.push((g * 255.0).round() as u8);
    }
    Some((geo, glow))
}

/// `lit` with its alpha replaced by the glow (the pass reads the lit
/// image's alpha as "never lit, never darkened").
fn with_glow(lit: &[u8], glow: Option<&[u8]>) -> Vec<u8> {
    let mut out = lit.to_vec();
    for (i, p) in out.chunks_exact_mut(4).enumerate() {
        p[3] = glow.map_or(0, |g| g[i]);
    }
    out
}

// ------------------------------------------------------------------ lights

/// How a lamp kind lights in the pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LampPhysics {
    /// Flicker, colour and drawing: a flame's (torch) or a mote's.
    pub model: field_gpu::LightKind,
    /// At full brightness, before flicker.
    pub power: f32,
    /// Falloff `power / (d^2 + r0^2)`, metres.
    pub r0: f32,
    /// Where it has faded to nothing, metres, at the reach the kind is made
    /// with; a lamp's own reach scales it.
    pub range: f32,
    /// The light's own size, metres: how soft its shadows are.
    pub radius: f32,
    /// Light it spreads round the area whatever a surface faces.
    pub bounce: f32,
    /// How far round a surface it wraps, on creatures and on the ground.
    pub wrap_creature: f32,
    pub wrap_ground: f32,
}

/// Each lamp kind's physics. The torch and the wisp (a mote of light) are
/// the notes' table (lights-and-weather.md); the rest by likeness: a
/// campfire is a bigger fire, softer-shadowed; a lantern a small steady
/// flame behind glass, sharp-shadowed, throwing its beam farther; a glow a
/// wide soft source that fills the area.
pub fn lamp_physics(kind: LampKind) -> LampPhysics {
    use field_gpu::LightKind::{Mote, Torch};
    let (model, power, r0, range, radius, bounce, wrap_creature, wrap_ground) = match kind {
        LampKind::Torch => (Torch, 9.0, 0.55, 14.0, 0.10, 0.04, 0.10, 0.0),
        LampKind::Campfire => (Torch, 13.5, 0.8, 16.0, 0.30, 0.06, 0.15, 0.05),
        LampKind::Lantern => (Torch, 8.0, 0.45, 18.0, 0.05, 0.02, 0.05, 0.0),
        LampKind::Wisp => (Mote, 7.0, 0.9, 10.0, 0.22, 0.20, 0.35, 0.25),
        LampKind::Glow => (Mote, 5.0, 1.2, 8.0, 0.45, 0.25, 0.40, 0.30),
    };
    LampPhysics { model, power, r0, range, radius, bounce, wrap_creature, wrap_ground }
}

/// How far a lamp reaches, metres: its kind's range at the reach it is made
/// with, scaled by its own.
pub fn lamp_range(light: &PointLight) -> f32 {
    let made = PointLight::new(light.kind, 0.0, 1.0).radius.max(1e-3);
    (lamp_physics(light.kind).range * light.radius.max(0.1) / made).clamp(2.0, 40.0)
}

/// A mote's pulse depth at a lamp's flicker (the notes' +-18% at a wisp's
/// usual 0.6) and how far its colour follows the pulse (60%).
const PULSE_PER_FLICKER: f32 = 0.3;
const MOTE_COLOUR_SHIFT: f32 = 0.6;

fn hex_lin(hex: &str) -> [f32; 3] {
    let h = hex.trim().trim_start_matches('#');
    let p = |i: usize| h.get(i..i + 2).and_then(|s| u8::from_str_radix(s, 16).ok()).unwrap_or(200) as f32 / 255.0;
    [p(0), p(2), p(4)].map(|c| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) })
}

/// One scene lamp as the pass's light, now (`t` seconds).
fn scene_light(field: &gs_field::solver::CreatureField, cam: &FieldCamera, k: f32, light: &PointLight, index: usize, t: f32) -> Option<FieldLight> {
    if light.intensity <= 0.0 {
        return None;
    }
    // where it stands and how high its light is, as field_lights draws it
    let drift = light.drift(index, t);
    let (_, ppu) = field.project_ground(light.x, light.z);
    let (x, z) = (light.x + drift[0] * ppu, (light.z + drift[2]).max(0.4));
    let up_m = (crate::field_lights::flame_height(light.kind) + drift[1]) * METRES_PER_CARD_UNIT;
    let z_m = z * k;
    let pos = [(x - cam.cx) / cam.f * z_m, cam.height - up_m, z_m];
    let phys = lamp_physics(light.kind);
    let range = lamp_range(light);
    let base = hex_lin(&light.color);
    let scale = light.intensity.clamp(0.0, 4.0);
    let mut l = match phys.model {
        field_gpu::LightKind::Mote => {
            let (f, kk) = field_gpu::mote_pulse(t + index as f32 * 3.7, PULSE_PER_FLICKER * light.flicker.clamp(0.0, 1.0), 1.0);
            let (bright, dim) = field_gpu::mote_shades(&light.color);
            let kk = kk * MOTE_COLOUR_SHIFT;
            let colour = [0, 1, 2].map(|i| if kk >= 0.0 { base[i] + (bright[i] - base[i]) * kk } else { base[i] + (dim[i] - base[i]) * -kk });
            let mut l = FieldLight::mote(pos, colour, f, 1.0, range);
            l.power = phys.power * scale * f;
            l.show = light.kind == LampKind::Wisp;
            l
        }
        // a flame (no lamp kind is a moon)
        field_gpu::LightKind::Torch | field_gpu::LightKind::Moon => {
            let (f, sway) = field_gpu::torch_flicker(t + index as f32 * 11.3, light.flicker.clamp(0.0, 1.0));
            // the author's colour, warming and reddening as the notes' torch does
            let (now, rest) = (field_gpu::torch_colour(f), field_gpu::torch_colour(1.0));
            let colour = [0, 1, 2].map(|i| base[i] * now[i] / rest[i].max(1e-4));
            let swayed = [pos[0] + sway[0], pos[1] + sway[1], pos[2] + sway[2]];
            let mut l = FieldLight::torch(swayed, pos, up_m, f, 1.0);
            l.colour = colour;
            l.power = phys.power * scale * f;
            l.show = light.kind == LampKind::Torch;
            l
        }
    };
    l.r0 = phys.r0;
    l.range = range;
    l.radius = phys.radius;
    l.bounce = phys.bounce;
    l.wrap_creature = phys.wrap_creature;
    l.wrap_ground = phys.wrap_ground;
    Some(l.aimed(cam, light.aim, light.spread))
}

// ------------------------------------------------------------------ weather

/// A storm's lightning: strikes a minute, how far off (0 near .. 1 far) and
/// how strongly a strike lifts the night (the notes' defaults).
const STORM_PER_MIN: f32 = 7.0;
const STORM_DIST: f32 = 0.4;
const STORM_STRENGTH: f32 = 0.6;
/// The night air's light as a share of the lit backdrop's mean.
const FOG_OF_SCENE: f32 = 0.9;
/// Blood rain's colour.
const BLOOD_RAIN: &str = "#b0302a";

/// A backdrop's air: its fog colour (the lit backdrop's mean, linear, a
/// share of it) and its dust (the notes': half #c9a472, half the painted
/// ground's colour -- the lower 40% of the backdrop).
fn scene_air(px: &crate::creature_light::LitPixels) -> ([f32; 3], [f32; 3]) {
    let (w, h) = (px.w.max(1), px.h.max(1));
    let mut lit = [0.0f32; 3];
    let mut ground = [0.0f32; 3];
    let (mut n, mut ng) = (0.0f32, 0.0f32);
    let step = ((w * h) / 20000).max(1);
    for i in (0..w * h).step_by(step) {
        let (l, a) = (&px.lit[i * 4..i * 4 + 3], &px.albedo[i * 4..i * 4 + 3]);
        for k in 0..3 {
            lit[k] += s2l(l[k]);
        }
        n += 1.0;
        if i / w >= h * 6 / 10 {
            for k in 0..3 {
                ground[k] += a[k] as f32 / 255.0;
            }
            ng += 1.0;
        }
    }
    let base = hex_lin("#c9a472");
    let fog = lit.map(|c| c / n.max(1.0) * FOG_OF_SCENE);
    let dust = [0, 1, 2].map(|k| {
        let g = ground[k] / ng.max(1.0);
        let g = if g <= 0.04045 { g / 12.92 } else { ((g + 0.055) / 1.055).powf(2.4) };
        0.5 * base[k] + 0.5 * g
    });
    (fog, dust)
}

fn s2l(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// The host's weather as the pass draws it, at `t` seconds, in a scene whose
/// air is `fog` and dust `dust`.
pub fn field_weather(w: Option<&crate::field_weather::Weather>, t: f32, fog: [f32; 3], dust: [f32; 3]) -> FieldWeather {
    use crate::field_weather::WeatherKind as K;
    let mut f = FieldWeather { fog, dust, ..FieldWeather::default() };
    let Some(w) = w.filter(|w| w.intensity > 0.0) else { return f };
    let i = w.intensity.clamp(0.0, 1.0);
    let snow_wind = |amount: f32| w.wind * (0.6 + 11.0 * amount * amount);
    match w.kind {
        K::Rain | K::BloodRain => {
            f.rain = i;
            f.wind = w.wind;
            if w.kind == K::BloodRain {
                f.rain_tint = hex_lin(BLOOD_RAIN).map(|c| c * 2.5);
            }
        }
        K::Storm => {
            f.rain = i;
            f.wind = w.wind;
            let (l, x) = field_gpu::lightning(t, STORM_PER_MIN, STORM_DIST);
            f.flash = l * STORM_STRENGTH;
            f.flash_x = x;
        }
        K::Snow => {
            f.snow = i;
            f.snow_wind = snow_wind(i);
            f.snow_cover = 0.2 + 0.4 * i;
        }
        K::Sleet => {
            f.rain = 0.6 * i;
            f.wind = w.wind;
            f.snow = 0.5 * i;
            f.snow_wind = snow_wind(0.5 * i);
        }
        K::Fog => f.mist = 0.35 * i,
        K::Sand => {
            f.sand = i;
            f.sand_wind = if w.wind < 0.0 { -1.0 } else { 1.0 } * (3.0 + 15.0 * w.wind.abs()) * (0.8 + 0.5 * i);
        }
    }
    f
}

/// The moon's brightness at full ([`MoonNow::brightness`] 1): what it adds
/// to a surface facing it, before the surface's colour.
const MOON_POWER: f32 = 0.32;
/// How fast cloud shadows drift with no wind, and how much a full wind adds
/// (m/s, left to right).
const CLOUD_DRIFT: f32 = 3.0;
const CLOUD_WIND: f32 = 9.0;

/// The moon as the pass's light, from where it is in the sky.
fn moon_light(cam: &FieldCamera, moon: &crate::creature_light::MoonNow, wind: f32) -> FieldLight {
    let up = cam.up();
    let deeper = [0.0, up[2], -up[1]];
    let (sa, ca) = moon.azimuth.to_radians().sin_cos();
    let (se, ce) = moon.elevation.to_radians().sin_cos();
    // across the floor sin(az), toward the viewer cos(az); up by elevation
    let toward = [0, 1, 2].map(|i| ce * (sa * [1.0, 0.0, 0.0][i] - ca * deeper[i]) + se * up[i]);
    // with the wind, else left to right; a little away from the viewer too
    let drift = (CLOUD_DRIFT + CLOUD_WIND * wind.abs()) * if wind < 0.0 { -1.0 } else { 1.0 };
    // Its disc: where it is round the viewer, but its height squeezed into
    // the sky the frame shows -- the camera looks level, and a moon at its
    // true height would leave the frame an hour after rising.
    let top_deg = (cam.cy / cam.f.max(1e-3)).atan().to_degrees();
    let disc_el = (moon.elevation / gs_field::lighting::MOON_PEAK_DEG).clamp(0.0, 1.0) * top_deg * DISC_SKY_SHARE;
    let (sd, cd) = disc_el.to_radians().sin_cos();
    let disc = [0, 1, 2].map(|i| cd * (sa * [1.0, 0.0, 0.0][i] - ca * deeper[i]) + sd * up[i]);
    FieldLight::moon(toward, hex_lin(&moon.colour), MOON_POWER * moon.brightness, moon.clouds, [drift, 0.2 * drift.abs()]).with_disc(
        disc,
        moon.disc_deg.to_radians(),
        moon.phase,
        moon.companion,
    )
}

/// How much of the sky above the horizon a moon's disc climbs through, at
/// its highest.
const DISC_SKY_SHARE: f32 = 0.8;

/// How wide a constellation's drawing is on the sky, degrees (less in a
/// frame with less sky: [`CONSTELLATION_SKY_SHARE`] of it at most).
const CONSTELLATION_DEG: f32 = 16.0;
const CONSTELLATION_SKY_SHARE: f32 = 0.9;
/// An ordinary star's radius, scene pixels, and how bright it is (linear).
const STAR_PX: f32 = 1.25;
const STAR_POWER: f32 = 1.0;
const NEBULA_POWER: f32 = 0.16;

/// The stars as the pass draws them: each constellation up now where it is
/// round the viewer, its height squeezed into the frame's sky as a moon's
/// disc is ([`moon_light`]) -- at its highest, its top just under the
/// frame's -- its figure at its own shape, leaning as the sky turns it;
/// only what lands on the stage.
fn star_field(cam: &FieldCamera, now: &crate::creature_light::StarsNow, wind: f32) -> field_gpu::FieldStars {
    let up = cam.up();
    let deeper = [0.0, up[2], -up[1]];
    let top_deg = (cam.cy / cam.f.max(1e-3)).atan().to_degrees();
    // scene pixels per unit of a drawing (-1..1 across)
    let span = (CONSTELLATION_DEG).min(top_deg * CONSTELLATION_SKY_SHARE);
    let unit = cam.f * (span / 2.0).to_radians().tan();
    let all = gs_field::stars::constellations();
    let (mut stars, mut nebulae) = (Vec::new(), Vec::new());
    let on_stage = |p: [f32; 2], m: f32| p[0] > -m && p[0] < cam.size[0] + m && p[1] > -m && p[1] < cam.size[1] + m;
    for placed in &now.up {
        let Some(c) = all.get(placed.index) else { continue };
        let (sa, ca) = placed.azimuth.to_radians().sin_cos();
        let el = (placed.elevation / gs_field::lighting::MOON_PEAK_DEG).clamp(0.0, 1.0) * (top_deg - span / 2.0).max(0.0);
        let (se, ce) = el.to_radians().sin_cos();
        let dir = [0, 1, 2].map(|i| ce * (sa * [1.0, 0.0, 0.0][i] - ca * deeper[i]) + se * up[i]);
        if dir[2] < 0.2 {
            continue;
        }
        let centre = [cam.cx + cam.f * dir[0] / dir[2], cam.cy + cam.f * dir[1] / dir[2]];
        if !on_stage(centre, 1.5 * unit) {
            continue;
        }
        // drawing (y up) to the screen (y down), turned by its lean
        let (st, ct) = placed.tilt.to_radians().sin_cos();
        let place = |x: f32, y: f32| [centre[0] + unit * (x * ct - y * st), centre[1] - unit * (x * st + y * ct)];
        for s in &c.stars {
            let at = place(s.x, s.y);
            if on_stage(at, 8.0) {
                let points = if s.name.is_some() && c.points > 0 { c.points as f32 } else { 0.0 };
                stars.push(field_gpu::FieldStar { at, radius: STAR_PX * s.size.sqrt(), points, colour: hex_lin(&s.color), brightness: STAR_POWER * s.size * now.light });
            }
        }
        for n in &c.nebulae {
            nebulae.push(field_gpu::FieldNebula {
                at: place(n.x, n.y),
                long: unit * n.long,
                wide: unit * n.wide,
                angle: -(n.angle + placed.tilt).to_radians(),
                colour: hex_lin(&n.color),
                brightness: NEBULA_POWER * now.light,
            });
        }
    }
    // brightest first, so a full frame drops the faintest
    stars.sort_by(|a, b| b.brightness.total_cmp(&a.brightness));
    let drift = (CLOUD_DRIFT + CLOUD_WIND * wind.abs()) * if wind < 0.0 { -1.0 } else { 1.0 };
    field_gpu::FieldStars {
        stars,
        nebulae,
        light: now.light,
        clouds: now.clouds,
        drift: [drift, 0.2 * drift.abs()],
        turn_px: now.turn.to_radians() * cam.f,
    }
}

// ------------------------------------------------------------------ the stage

thread_local! {
    /// The frame the stage is drawing now (the cards reach it the way they
    /// reach the wearer's light: a marked addition may not change the
    /// ported renderer's signatures).
    static CURRENT: std::cell::RefCell<Option<StageGpu>> = const { std::cell::RefCell::new(None) };
}

/// Hand the stage's frame to the card painters (None: draw the old way).
/// Called once per stage draw, before the backdrop.
pub fn set_current(gpu: Option<StageGpu>) {
    CURRENT.with(|c| *c.borrow_mut() = gpu);
}

/// Run `f` on the stage's frame, if the pass is drawing it.
pub fn with_current<R>(f: impl FnOnce(&StageGpu) -> R) -> Option<R> {
    CURRENT.with(|c| c.borrow().as_ref().map(f))
}

/// One stage frame of the pass (see the module docs).
pub struct StageGpu {
    frame: SharedFrame,
    place: Placement,
    /// Metres per camera unit.
    k: f32,
    /// The atlas as this frame's layers fill it: a layer it has no room
    /// for is drawn the old way ([`field_gpu::AtlasPlan`]).
    plan: std::cell::RefCell<field_gpu::AtlasPlan>,
    /// Screen pixels per point, for sizing uploads.
    ppp: f32,
    /// Per scene lamp: lit by this pass ([`StageGpu::lamps_in_pass`]).
    in_pass: Vec<bool>,
    /// The camera's focal length, stage pixels (metres per stage pixel at
    /// depth z is z / f).
    f: f32,
}

/// The pass's lights: the moons first, then the scene's lamps in order (None:
/// a lamp that gives no light) while [`field_gpu::MAX_LIGHTS`] leaves room --
/// and, per lamp, whether it got in.
fn admit_lamps(moons: Vec<FieldLight>, lamps: impl Iterator<Item = Option<FieldLight>>) -> (Vec<FieldLight>, Vec<bool>) {
    let mut lights = moons;
    lights.truncate(field_gpu::MAX_LIGHTS);
    let mut in_pass = Vec::new();
    for lamp in lamps {
        let admitted = match lamp {
            Some(l) if lights.len() < field_gpu::MAX_LIGHTS => {
                lights.push(l);
                true
            }
            _ => false,
        };
        in_pass.push(admitted);
    }
    (lights, in_pass)
}

impl StageGpu {
    /// Start this frame's pass, or None to draw the old way. `origin` and
    /// `scale` place the 880x470 stage on screen (`StageMap`);
    /// `backdrop_rect` is where the backdrop image lies, stage pixels.
    #[allow(clippy::too_many_arguments)]
    pub fn begin(
        ctx: &egui::Context,
        field: &gs_field::solver::CreatureField,
        scene: Option<&gs_field::scenes::StageScene>,
        cache: Option<&crate::creature_art::CreatureArtCache>,
        backdrop: Option<&egui::TextureHandle>,
        backdrop_rect: Option<[f32; 4]>,
        origin: egui::Pos2,
        scale: f32,
    ) -> Option<StageGpu> {
        if !enabled(ctx) {
            return None;
        }
        let (cache, tex, rect) = (cache?, backdrop?, backdrop_rect?);
        let px = cache.light.gpu_pixels(tex.id())?;
        // the camera, from the solver's own projection
        let p = &field.params;
        let z = 5.0;
        let ((_, sy), ppu) = field.project_ground(gs_field::solver::STAGE_W * 0.5, z);
        let f = ((sy - p.horizon) * z / p.cam_h.max(1e-3)).max(1.0);
        let mscale = (ppu * z / f).max(1e-3);
        let k = METRES_PER_CARD_UNIT / mscale;
        let camera = FieldCamera {
            size: [gs_field::solver::STAGE_W, gs_field::solver::STAGE_H],
            f,
            cx: gs_field::solver::STAGE_W * 0.5,
            cy: p.horizon,
            pitch: 0.0,
            height: p.cam_h * k,
        };
        // its depth map, if it has one: fitted to this camera's ground
        let depth = scene.and_then(|s| s.background.as_deref()).and_then(|bg| cache.backdrop_depth(bg));
        let bg_key = key_of((
            "backdrop",
            tex.id(),
            rect.map(f32::to_bits),
            f.to_bits(),
            p.horizon.to_bits(),
            camera.height.to_bits(),
            depth.map(|d| Arc::as_ptr(d) as usize),
        ));
        let backdrop = cached(bg_key, || {
            let geo = depth.and_then(|d| backdrop_geo(&camera, px.w, px.h, rect, d)).unwrap_or_else(|| ground_geo(&camera, px.w, px.h, rect));
            Some(LitImage { key: bg_key, w: px.w as u32, h: px.h as u32, albedo: px.albedo.clone(), lit: Arc::new(with_glow(&px.lit, None)), geo: Arc::new(geo) })
        })?;
        let t = ctx.input(|i| i.time) as f32;
        let wind = crate::field_weather::weather(ctx).map_or(0.0, |w| w.wind);
        let moons: Vec<FieldLight> = cache.light.moons_now().iter().map(|m| moon_light(&camera, m, wind)).collect();
        let lamps = scene.map_or(&[][..], |s| &s.lights[..]);
        let (lights, in_pass) = admit_lamps(moons, lamps.iter().enumerate().map(|(i, l)| scene_light(field, &camera, k, l, i, t)));
        let (fog, dust) = air_of(bg_key, &px);
        let weather = field_weather(crate::field_weather::weather(ctx).as_ref(), t, fog, dust);
        let frame = FieldFrame {
            id: FieldFrame::next_id(),
            camera,
            backdrop,
            backdrop_rect: rect,
            layers: Vec::new(),
            lights,
            time: t,
            water: 0.0,
            shadows: true,
            view: FieldView::Lit,
            weather,
            detail: field_gpu::DETAIL,
            bounce: field_gpu::BOUNCE,
            stars: cache.light.stars_now().map(|s| star_field(&camera, &s, wind)).unwrap_or_default(),
        };
        Some(StageGpu { frame: Arc::new(Mutex::new(frame)), place: Placement { origin, scale }, k, plan: Default::default(), ppp: ctx.pixels_per_point(), in_pass, f })
    }

    /// Which of the scene's lamps (by index) this pass lights and draws: the
    /// rest -- past the pass's [`field_gpu::MAX_LIGHTS`] -- are drawn the old
    /// way (`field_lights::set_gpu`), not lost.
    pub fn lamps_in_pass(&self) -> &[bool] {
        &self.in_pass
    }

    /// Draw the backdrop piece (in place of the backdrop image).
    pub fn paint_backdrop(&self, painter: &egui::Painter) {
        field_gpu::paint(painter, &self.frame, self.place, Piece::Backdrop);
    }

    fn stage_rect(&self, screen: egui::Rect) -> [f32; 4] {
        let s = self.place.scale.max(1e-6);
        [(screen.min.x - self.place.origin.x) / s, (screen.min.y - self.place.origin.y) / s, screen.width() / s, screen.height() / s]
    }

    /// Add a layer and paint it -- only when the frame has a layer to
    /// spare and the atlas room for its image, decided before anything is
    /// committed, so a refused layer is drawn the old way instead of
    /// vanishing.
    fn push(&self, painter: &egui::Painter, image: Arc<LitImage>, screen: egui::Rect, ground_z: f32, tint: f32) -> bool {
        if self.frame.lock().map_or(true, |f| f.layers.len() >= field_gpu::MAX_LAYERS) || !self.plan.borrow_mut().admit(&image) {
            return false;
        }
        let layer = LayerDraw { image, rect: self.stage_rect(screen), feet_z: ground_z * self.k, tint };
        match field_gpu::push_layer(&self.frame, layer) {
            Some(i) => {
                field_gpu::paint(painter, &self.frame, self.place, Piece::Layer(i));
                true
            }
            None => false,
        }
    }

    /// Draw a sprite body (`tex`, the texture the cache shows for it) as a
    /// lit layer at `dest` (screen), standing at solver depth `ground_z`.
    /// False = not drawn (no pixels kept for it yet, or the frame is full):
    /// draw it the old way.
    #[allow(clippy::too_many_arguments)]
    pub fn paint_sprite(
        &self,
        painter: &egui::Painter,
        cache: &crate::creature_art::CreatureArtCache,
        tex: &egui::TextureHandle,
        art: &crate::creature_art::CreatureArt,
        dest: egui::Rect,
        ground_z: f32,
        dead: bool,
    ) -> bool {
        let Some(px) = cache.light.gpu_pixels(tex.id()) else { return false };
        let maps = (art.extra("depth"), art.extra("normal"));
        let (w, h) = upload_size(px.w, px.h, dest.height() * self.ppp);
        // depth maps (and the bulge of art with none) are made for a figure
        // NOMINAL_FIGURE_M tall: this creature's, at its height on the stage
        let figure_m = self.stage_rect(dest)[3] * (art.bbox[3] - art.bbox[1]).clamp(0.05, 1.0) * ground_z * self.k / self.f.max(1e-3);
        let depth_scale = ((figure_m / NOMINAL_FIGURE_M).clamp(0.05, 20.0) * 50.0).round() / 50.0;
        let key = key_of(("sprite", tex.id(), maps.0.cloned(), maps.1.cloned(), w, h, depth_scale.to_bits()));
        let Some(image) = cached(key, || {
            let albedo = downsample(&px.albedo, px.w, px.h, w, h);
            let lit = downsample(&px.lit, px.w, px.h, w, h);
            let alpha: Vec<u8> = px.albedo.chunks_exact(4).map(|p| p[3]).collect();
            let (geo, glow) = match maps.0.zip(maps.1).and_then(|(d, n)| maps_geo(d, n, px.w, px.h)) {
                Some((geo, glow)) => {
                    // the glow mask rides the alpha channel through the box filter
                    let g: Vec<u8> = glow.iter().zip(&alpha).flat_map(|(&g, &a)| [g, g, g, a]).collect();
                    let glow = downsample(&g, px.w, px.h, w, h).chunks_exact(4).map(|p| p[0]).collect::<Vec<u8>>();
                    (downsample_geo(&geo, &alpha, px.w, px.h, w, h), Some(glow))
                }
                None => (profile_geo(&albedo, w, h), None),
            };
            let geo: Vec<[f32; 4]> = geo.into_iter().map(|g| [g[0], g[1], g[2], g[3] * depth_scale]).collect();
            Some(LitImage {
                key,
                w: w as u32,
                h: h as u32,
                albedo: Arc::new(albedo),
                lit: Arc::new(with_glow(&lit, glow.as_deref())),
                geo: Arc::new(geo),
            })
        }) else {
            return false;
        };
        self.push(painter, image, dest, ground_z, if dead { DEAD_TINT } else { 1.0 })
    }

    /// Draw a puppet's skin as a lit layer at `screen` (where its image
    /// would go), standing at solver depth `ground_z`; `m_per_ft` is how
    /// many metres a foot of the figure is on this stage. A dead puppet's
    /// skin is a corpse's colour already, so it is drawn as it is. False =
    /// not drawn.
    pub fn paint_skin(
        &self,
        painter: &egui::Painter,
        skin: &Arc<gs_puppet::Skin>,
        surface: &gs_puppet::SkinSurface,
        screen: egui::Rect,
        ground_z: f32,
        m_per_ft: f32,
    ) -> bool {
        if skin.w != surface.w || skin.h != surface.h || skin.w == 0 {
            return false;
        }
        // the Arc's address alone could be reused by a later skin: fold in
        // a sample of its pixels
        let step = (skin.rgba.len() / 4096).max(1);
        let sample: Vec<u8> = skin.rgba.iter().step_by(step).copied().collect();
        let key = key_of(("skin", Arc::as_ptr(skin) as usize, skin.w, skin.h, sample, m_per_ft.to_bits()));
        let Some(image) = cached(key, || Some(skin_image(key, skin, surface, m_per_ft))) else {
            return false;
        };
        self.push(painter, image, screen, ground_z, 1.0)
    }
}

/// How far round the figure an outline pixel looks for the body's depth,
/// pixels: past the widest outline a skin draws (the contour and a boss's
/// gold round it).
const OUTLINE_REACH: usize = 8;

/// A puppet's skin as the pass's layer image. Every pixel the skin shows is
/// in it, at the skin's own coverage: where the figure's surface is
/// (`surface.texel`), its own colour, normal and depth, lit by the lamps;
/// the contour drawn round it and a boss's gold outline -- skin with no
/// surface -- keep their colour and are left unlit (no albedo for a lamp
/// to light), at the depth of the nearest surface pixel, so the shadow
/// they cast is the body's edge.
fn skin_image(key: u64, skin: &gs_puppet::Skin, surface: &gs_puppet::SkinSurface, m_per_ft: f32) -> LitImage {
    // the skin's frame (x right, y up, +z toward the viewer) to the
    // camera's (x right, y down, z away)
    let back = surface.to_cam();
    let fwd = [-back[0], -back[1], -back[2]];
    let up = [0.0f32, 1.0, 0.0];
    let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let unit = |v: [f32; 3]| {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
        v.map(|c| c / l)
    };
    let right = unit(cross(fwd, up));
    let down = cross(fwd, right);
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let (w, h) = (skin.w, skin.h);
    let n = w * h;
    let mut albedo = vec![0u8; n * 4];
    let mut geo = vec![[0.0f32; 4]; n];
    let mut surfaced = vec![false; n];
    for i in 0..n {
        let shown = skin.rgba[i * 4 + 3];
        if let Some((pos, nrm, alb, _)) = surface.texel(i) {
            albedo[i * 4..i * 4 + 3].copy_from_slice(&alb);
            geo[i] = [dot(nrm, right), dot(nrm, down), dot(nrm, fwd), dot(pos, fwd) * m_per_ft];
            surfaced[i] = true;
        }
        albedo[i * 4 + 3] = shown;
    }
    // an outline pixel: the depth of the nearest surface pixel, ring by ring
    for i in 0..n {
        if surfaced[i] || skin.rgba[i * 4 + 3] == 0 {
            continue;
        }
        let (x, y) = ((i % w) as isize, (i / w) as isize);
        'rings: for r in 1..=OUTLINE_REACH as isize {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dy.abs() != r {
                        continue;
                    }
                    let (qx, qy) = (x + dx, y + dy);
                    if qx < 0 || qy < 0 || qx >= w as isize || qy >= h as isize {
                        continue;
                    }
                    let j = qy as usize * w + qx as usize;
                    if surfaced[j] {
                        geo[i][3] = geo[j][3];
                        break 'rings;
                    }
                }
            }
        }
    }
    LitImage { key, w: w as u32, h: h as u32, albedo: Arc::new(albedo), lit: Arc::new(with_glow(&skin.rgba, None)), geo: Arc::new(geo) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two moons leave six of the eight slots: the first six lamps that
    /// give light get them; one that gives none is passed over; the rest
    /// are marked out of the pass, so the old drawing keeps them.
    #[test]
    fn lamps_past_the_limit_are_left_to_the_old_drawing() {
        let torch = || Some(FieldLight::torch([0.0, 0.0, 5.0], [0.0, 0.0, 5.0], 0.8, 1.0, 1.0));
        let moon = || FieldLight::moon([0.0, -1.0, 0.0], [1.0; 3], 0.3, 0.0, [0.0; 2]);
        let lamps: Vec<Option<FieldLight>> = (0..9).map(|i| if i == 2 { None } else { torch() }).collect();
        let (lights, in_pass) = admit_lamps(vec![moon(), moon()], lamps.into_iter());
        assert_eq!(lights.len(), field_gpu::MAX_LIGHTS);
        assert_eq!(in_pass, [true, true, false, true, true, true, true, false, false]);
        let (lights, in_pass) = admit_lamps(Vec::new(), std::iter::repeat_with(torch).take(3));
        assert_eq!((lights.len(), in_pass), (3, vec![true; 3]));
    }

    fn humanoid() -> gs_puppet::Puppet {
        gs_puppet::Puppet::from_baked_toml(include_str!("../../gs_field/data/puppets/humanoid.puppet.toml")).expect("shipped humanoid")
    }

    /// The review's case: a humanoid with a boss's outline, framed at 300
    /// px. Every pixel the skin shows is in the layer at the skin's own
    /// coverage; the outline -- skin with no surface -- is kept, unlit, at
    /// the body's depth.
    #[test]
    fn a_puppets_outline_reaches_the_pass() {
        let p = humanoid();
        let posed = p.pose(&gs_puppet::Pose::default());
        let cam = gs_puppet::Camera::framing(p.height, [300.0, 300.0]);
        let project = |q| cam.project(q);
        for look in [gs_puppet::Look { boss_outline: true, ..Default::default() }, gs_puppet::Look { dead: true, ..Default::default() }] {
            let (skin, surface) = posed.skin_surface(&project, &look, None, gs_puppet::Effects::default());
            let img = skin_image(1, &skin, &surface, 0.3);
            let mut outline = 0;
            for i in 0..skin.w * skin.h {
                assert_eq!(img.albedo[i * 4 + 3], skin.rgba[i * 4 + 3], "coverage at {i}");
                if skin.rgba[i * 4 + 3] > 0 && surface.texel(i).is_none() {
                    outline += 1;
                    assert_eq!(&img.albedo[i * 4..i * 4 + 3], &[0, 0, 0], "an outline is not lit");
                    assert!(img.geo[i][3] != 0.0, "an outline pixel stands at the body's depth");
                }
            }
            assert!(outline > 500, "the outline is there: {outline} pixels");
        }
    }

    /// A backdrop's relative depth -- ground that is `a / z + b`, a rock
    /// standing nearer on it, sky beyond the far limit -- fits back to the
    /// camera's ground in metres: the ground at its depth, facing up; the
    /// rock nearer, facing the viewer; the sky as sky. Nearness that does
    /// not grow toward the camera fits nothing.
    #[test]
    fn a_backdrops_depth_map_fits_to_the_ground_it_shows() {
        let cam = FieldCamera { size: [880.0, 470.0], f: 420.0, cx: 440.0, cy: 96.0, pitch: 0.0, height: 2.4 };
        let (w, h) = (176, 94);
        let rect = [0.0, 0.0, 880.0, 470.0];
        let flat = ground_geo(&cam, w, h, rect);
        let rock = |i: usize, j: usize| (70..90).contains(&i) && (70..85).contains(&j);
        let near: Vec<f32> = (0..w * h)
            .map(|k| {
                let (i, j) = (k % w, k / w);
                let z = flat[k][3];
                if z >= 100.0 {
                    0.01
                } else if rock(i, j) {
                    0.05 + 0.6 / 3.0
                } else {
                    0.05 + 0.6 / z
                }
            })
            .collect();
        let rel = RelDepth { w, h, near };
        let geo = backdrop_geo(&cam, w, h, rect, &rel).expect("a fit");
        for (k, g) in geo.iter().enumerate() {
            let (i, j) = (k % w, k / w);
            let want = flat[k][3];
            if want >= 100.0 {
                assert!(g[3] > SKY_M, "sky at {i},{j}: {g:?}");
            } else if rock(i, j) {
                let inner = (74..86).contains(&i) && (74..81).contains(&j);
                assert!(!inner || (g[3] - 3.0).abs() < 0.15, "the rock stands at 3 m: {g:?}");
            } else if want < 40.0 && !(-4..=4).any(|di: isize| (-4..=4).any(|dj: isize| rock((i as isize + di).max(0) as usize, (j as isize + dj).max(0) as usize))) {
                // (the smoothing softens the rock's edge over 4 pixels)
                assert!((g[3] - want).abs() < 0.03 * want, "ground at {i},{j}: {} vs {want}", g[3]);
                // (this grid is coarse -- 5 stage pixels a row -- so far
                // ground jumps more a row than an edge does; near it, not)
                // (nor on the last rows, where the smoothing runs out of rows)
                if want < 15.0 && j + 3 < h {
                    assert!(g[1] < -0.95, "ground faces up (y is down): {g:?}");
                }
            }
        }
        let flat_near = RelDepth { w, h, near: vec![0.5; w * h] };
        assert_eq!(backdrop_geo(&cam, w, h, rect, &flat_near), None, "nearness that does not grow toward the camera");
        assert_eq!(depth_path("scenes/forest.png"), "scenes/forest.depth.png");
    }

    #[test]
    fn a_flat_backdrop_is_ground_below_the_horizon_and_sky_above() {
        let cam = FieldCamera { size: [880.0, 470.0], f: 420.0, cx: 440.0, cy: 96.0, pitch: 0.0, height: 2.4 };
        let geo = ground_geo(&cam, 2, 470, [0.0, 0.0, 880.0, 470.0]);
        let row = |y: usize| geo[y * 2];
        assert_eq!(row(50)[3], 100.0, "above the horizon: sky");
        let g = row(300);
        let want = 2.4 / ((300.5 - 96.0) / 420.0);
        assert!((g[3] - want).abs() < 1e-3, "{} vs {want}", g[3]);
        assert!(g[1] < -0.99, "ground faces up (y is down): {g:?}");
    }

    #[test]
    fn an_outline_rounds_toward_its_edges_and_bulges_in_the_middle() {
        // a 40x40 disc
        let (w, h) = (48, 48);
        let rgba: Vec<u8> = (0..w * h)
            .flat_map(|i| {
                let (x, y) = ((i % w) as f32 - 24.0, (i / w) as f32 - 24.0);
                [200, 200, 200, if x * x + y * y < 400.0 { 255 } else { 0 }]
            })
            .collect();
        let geo = profile_geo(&rgba, w, h);
        let at = |x: usize, y: usize| geo[y * w + x];
        let (mid, left, right) = (at(24, 24), at(5, 24), at(43, 24));
        assert!(mid[2] < -0.99, "the middle faces the viewer: {mid:?}");
        assert!(left[0] < -0.3 && right[0] > 0.3, "the rims face out: {left:?} {right:?}");
        assert!(mid[3] < left[3], "the middle is nearer the viewer than the rim");
        assert_eq!(at(0, 0), [0.0; 4], "outside is empty");
    }

    /// The torch and the wisp light as the notes' table says, and a lamp
    /// made with its kind's reach reaches its kind's range.
    #[test]
    fn lamps_light_by_their_kind() {
        let torch = lamp_physics(LampKind::Torch);
        assert_eq!((torch.power, torch.r0, torch.range, torch.radius, torch.bounce), (9.0, 0.55, 14.0, 0.10, 0.04));
        assert_eq!((torch.wrap_creature, torch.wrap_ground), (0.10, 0.0));
        let wisp = lamp_physics(LampKind::Wisp);
        assert_eq!((wisp.power, wisp.r0, wisp.range, wisp.radius, wisp.bounce), (7.0, 0.9, 10.0, 0.22, 0.20));
        assert_eq!((wisp.wrap_creature, wisp.wrap_ground), (0.35, 0.25));
        assert_eq!(lamp_physics(LampKind::Glow).model, field_gpu::LightKind::Mote);
        assert_eq!(lamp_physics(LampKind::Lantern).model, field_gpu::LightKind::Torch);
        let mut lamp = PointLight::new(LampKind::Torch, 300.0, 3.0);
        assert!((lamp_range(&lamp) - 14.0).abs() < 1e-4);
        lamp.radius *= 2.0;
        assert!((lamp_range(&lamp) - 28.0).abs() < 1e-3, "twice the reach, twice the range");
        assert!((lamp_range(&PointLight::new(LampKind::Wisp, 0.0, 1.0)) - 10.0).abs() < 1e-4);
    }

    /// The pass's beam is v1's: a wedge standing on the floor, however high
    /// the point is and however the camera is pitched.
    #[test]
    fn a_beam_on_the_gpu_is_the_lamps_own() {
        let mut lamp = PointLight::new(LampKind::Lantern, 0.0, 1.0);
        lamp.aim = 120.0;
        for pitch in [0.0f32, 0.14] {
            let cam = FieldCamera { size: [880.0, 470.0], f: 420.0, cx: 440.0, cy: 96.0, pitch, height: 2.4 };
            let lp = [0.3, 0.5, 6.0];
            let light = FieldLight::torch(lp, lp, 1.0, 1.0, 1.0).aimed(&cam, lamp.aim, lamp.spread);
            let up = cam.up();
            let deeper = [0.0, up[2], -up[1]];
            for (across, deep, high) in [(0.0, 2.0, 0.0), (-1.0, 1.7, 0.5), (1.0, 0.2, -1.0), (-2.0, 0.1, 0.0), (0.4, -1.0, 0.3), (-1.2, 1.1, 0.0)] {
                let p = [0, 1, 2].map(|i| lp[i] + [1.0, 0.0, 0.0][i] * across + deeper[i] * deep + up[i] * high);
                let (gpu, v1) = (light.beam_at(&cam, p), lamp.beam(across, deep));
                assert!((gpu - v1).abs() < 1e-4, "pitch {pitch}, ({across}, {deep}, {high}): {gpu} vs {v1}");
            }
        }
        let round = FieldLight::torch([0.0; 3], [0.0; 3], 1.0, 1.0, 1.0);
        let cam = FieldCamera { size: [880.0, 470.0], f: 420.0, cx: 440.0, cy: 96.0, pitch: 0.0, height: 2.4 };
        assert_eq!(round.aimed(&cam, 45.0, 360.0).beam, field_gpu::ALL_ROUND);
    }

    /// A preset wisp swells and fades toward its preset's own shades; any
    /// other colour toward white and black.
    #[test]
    fn a_preset_wisp_keeps_its_shades() {
        let (bright, dim) = field_gpu::mote_shades("#6FA8FF");
        assert_eq!((bright, dim), field_gpu::mote_shades("#6fa8ff"));
        let lin = |h: &str| hex_lin(h);
        let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5);
        assert!(close(bright, lin("#cfe3ff")) && close(dim, lin("#3a4dff")), "Arcane");
        let (b2, d2) = field_gpu::mote_shades("#808080");
        let grey = lin("#808080");
        assert!(b2[0] > grey[0] && d2[0] < grey[0]);
    }

    /// The moon at midnight is high and behind the viewer; rising, it is
    /// low on the left; setting, low on the right.
    #[test]
    fn the_moon_shines_from_where_it_is() {
        let cam = FieldCamera { size: [880.0, 470.0], f: 420.0, cx: 440.0, cy: 96.0, pitch: 0.0, height: 2.4 };
        let at = |az: f32, el: f32| {
            let m = crate::creature_light::MoonNow {
                azimuth: az,
                elevation: el,
                colour: "#ffffff".into(),
                brightness: 1.0,
                clouds: 0.3,
                phase: 0.3,
                disc_deg: 2.0,
                companion: true,
            };
            moon_light(&cam, &m, 0.0)
        };
        let high = at(0.0, 55.0);
        assert!(high.pos[1] < -0.8 && high.pos[2] < -0.5 && high.pos[0].abs() < 1e-5, "up (y is down) and toward the viewer: {:?}", high.pos);
        let len = high.pos.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((len - 1.0).abs() < 1e-4);
        assert!(at(-70.0, 5.0).pos[0] < -0.9 && at(70.0, 5.0).pos[0] > 0.9, "rises left, sets right");
        assert_eq!(high.kind, field_gpu::LightKind::Moon);
        assert_eq!(high.prop[0], 0.3, "the cloud cover rides along");
        // ahead and high, its disc is drawn in the frame's sky, below its top
        let ahead = at(180.0, 55.0);
        let d = ahead.beam;
        assert!(d[2] > 0.5 && ahead.pos[2] > 0.3, "ahead: {d:?}");
        let y = cam.cy + cam.f * d[1] / d[2];
        assert!(y > 0.0 && y < cam.cy, "in the sky, in the frame: row {y}");
        assert!((d[3] - 2.0f32.to_radians()).abs() < 1e-6 && (ahead.prop[3] - 1.3).abs() < 1e-6, "radius, phase + companion");
    }

    /// A constellation ahead and highest stands in the frame's sky at its
    /// own shape, upright; rising on the left it leans left; one behind the
    /// viewer is not drawn. Krrska keeps its eight points.
    #[test]
    fn the_stars_stand_in_the_frames_sky() {
        let cam = FieldCamera { size: [880.0, 470.0], f: 420.0, cx: 440.0, cy: 96.0, pitch: 0.0, height: 2.4 };
        let all = gs_field::stars::constellations();
        let huntress = all.iter().position(|c| c.name == "The Huntress").unwrap();
        let hammer = all.iter().position(|c| c.name == "The Hammer").unwrap();
        let now = |index: usize, azimuth: f32, elevation: f32, tilt: f32| crate::creature_light::StarsNow {
            light: 1.0,
            clouds: 0.2,
            up: vec![gs_field::stars::Placed { index, azimuth, elevation, tilt }],
            turn: 0.0,
        };
        let f = star_field(&cam, &now(huntress, 180.0, 55.0, 0.0), 0.0);
        assert_eq!(f.stars.len(), 1);
        let k = f.stars[0];
        assert_eq!(k.points, 8.0, "Krrska");
        assert!(k.at[1] > 0.0 && k.at[1] < cam.cy, "in the sky: {:?}", k.at);
        // the Hammer upright: its drawing's shape, scaled, y flipped
        let h = &all[hammer].stars;
        let f = star_field(&cam, &now(hammer, 180.0, 40.0, 0.0), 0.0);
        assert_eq!(f.stars.len(), h.len(), "all on the stage");
        let unit = cam.f * (16.0f32.min((cam.cy / cam.f).atan().to_degrees() * 0.9) / 2.0).to_radians().tan();
        let find = |x: f32, y: f32, stars: &[field_gpu::FieldStar]| stars.iter().any(|s| (s.at[0] - x).abs() < 1e-3 && (s.at[1] - y).abs() < 1e-3);
        let c = [cam.cx, f.stars.iter().map(|s| s.at[1]).sum::<f32>() / f.stars.len() as f32 + unit * h.iter().map(|s| s.y).sum::<f32>() / h.len() as f32];
        assert!(h.iter().all(|s| find(c[0] + unit * s.x, c[1] - unit * s.y, &f.stars)), "upright");
        // leaning 30 degrees left: its top star's x moves left of where it was
        let top = h.iter().max_by(|a, b| a.y.total_cmp(&b.y)).unwrap();
        let leaned = star_field(&cam, &now(hammer, 180.0, 40.0, 30.0), 0.0);
        let upright_top = c[0] + unit * top.x;
        let (st, ct) = 30f32.to_radians().sin_cos();
        let x = c[0] + unit * (top.x * ct - top.y * st);
        assert!(x < upright_top && leaned.stars.iter().any(|s| (s.at[0] - x).abs() < 1e-3), "leans left");
        assert!(star_field(&cam, &now(hammer, 0.0, 40.0, 0.0), 0.0).stars.is_empty(), "behind the viewer");
    }

    /// Each weather kind turns on its own parts; none is none.
    #[test]
    fn weather_kinds_draw_their_own_parts() {
        use crate::field_weather::{Weather, WeatherKind as K};
        let at = |kind, wind: f32| field_weather(Some(&Weather { kind, intensity: 0.8, wind }), 2.0, [0.1; 3], [0.5; 3]);
        assert!(!field_weather(None, 2.0, [0.1; 3], [0.5; 3]).any());
        let rain = at(K::Rain, 0.2);
        assert!(rain.rain == 0.8 && rain.wind == 0.2 && rain.snow == 0.0 && rain.flash == 0.0);
        assert!(at(K::Storm, 0.0).rain > 0.0, "a storm rains");
        let snow = at(K::Snow, 0.5);
        assert!(snow.snow == 0.8 && snow.snow_cover > 0.0 && snow.snow_wind > 0.0 && snow.rain == 0.0);
        let sleet = at(K::Sleet, 0.0);
        assert!(sleet.rain > 0.0 && sleet.snow > 0.0);
        assert!(at(K::Fog, 0.0).mist > 0.0);
        let sand = at(K::Sand, -0.5);
        assert!(sand.sand == 0.8 && sand.sand_wind < -3.0, "the sand blows with the wind: {}", sand.sand_wind);
        assert!(at(K::BloodRain, 0.0).rain_tint[0] > at(K::BloodRain, 0.0).rain_tint[1] * 3.0, "blood rain is red");
        assert_eq!(rain.fog, [0.1; 3]);
    }

    #[test]
    fn a_glow_mask_goes_in_the_lit_alpha() {
        assert_eq!(with_glow(&[1, 2, 3, 255, 4, 5, 6, 255], Some(&[0, 200])), vec![1, 2, 3, 0, 4, 5, 6, 200]);
        assert_eq!(with_glow(&[1, 2, 3, 255], None), vec![1, 2, 3, 0]);
    }
}
