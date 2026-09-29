//! The injury doll: one puppet in a window of its own -- its wounds, lost
//! limbs, statuses, pose, breath and what it holds are all the
//! [`PuppetKey`]'s -- lit by the v2 pass ([`field_gpu`]) under the sky round
//! the player ([`gs_field::doll_sky`]: the sun or the moons from where they
//! are round the way the player faces, a room light indoors), over a
//! background of that sky and the ground it stands on.
//!
//! It is its own view of the pass ([`field_gpu::paint_view`], [`DOLL_VIEW`]),
//! so it draws in the same egui pass as the field, lit its own way.
//! Without the pass (no wgpu) it draws its skin unlit.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use gs_field::doll_sky::DollSky;

use crate::creature_puppet::{size_step, Bare, PosedPuppet, PuppetCache, PuppetKey, PuppetPlacement};
use crate::field_gpu::{self, FieldCamera, FieldFrame, FieldLight, FieldView, LayerDraw, LitImage, Piece, Placement};

fn sky_id() -> egui::Id {
    egui::Id::new("gs_field_egui::doll_view::sky")
}

/// The field's sky for a doll to stand under (its scene, clock and facing),
/// or None; the art cache publishes it every frame it lights the field.
pub fn publish_sky(ctx: &egui::Context, sky: Option<DollSky>) {
    ctx.data_mut(|d| d.insert_temp(sky_id(), sky));
}

/// The sky the field last published ([`publish_sky`]): what a doll beside
/// it follows. None before the field has drawn, or with no scene.
pub fn field_sky(ctx: &egui::Context) -> Option<DollSky> {
    ctx.data(|d| d.get_temp::<Option<DollSky>>(sky_id())).flatten()
}

/// The doll's view of the pass.
pub const DOLL_VIEW: u64 = 0xD011;

/// The standing figure's height as a share of the window's, and where its
/// feet are (from the top).
const FIGURE_SHARE: f32 = 0.78;
const FEET_AT: f32 = 0.9;
/// The camera: this many figure heights away, its eye this share of the
/// figure's height up.
const DISTANCE_HEIGHTS: f32 = 3.5;
const EYE_SHARE: f32 = 0.6;
const M_PER_FT: f32 = 0.3048;
/// The backdrop is made at this share of the window's size: it is only a
/// gradient.
const BACKDROP_SHARE: f32 = 0.5;
/// A hit on the back turns the doll to show it: this long in all, the turn
/// round and back each taking [`TURN_SECS`].
const SHOW_BACK_SECS: f64 = 2.4;
const TURN_SECS: f64 = 0.45;

/// How far round (degrees) the doll is turned `t` seconds after a hit on
/// its back: round to show it, held, and back.
fn show_back(t: f64) -> Option<f32> {
    if !(0.0..SHOW_BACK_SECS).contains(&t) {
        return None;
    }
    let ease = |u: f64| {
        let u = u.clamp(0.0, 1.0);
        u * u * (3.0 - 2.0 * u)
    };
    let k = ease(t / TURN_SECS).min(ease((SHOW_BACK_SECS - t) / TURN_SECS));
    Some((180.0 * k) as f32)
}

/// Built images kept: a breathing cycle's worth, and some.
const IMAGES_KEPT: usize = 48;

fn s2l(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn l2s(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round() as u8
}

fn srgb(c: [f32; 3]) -> [u8; 3] {
    c.map(l2s)
}

fn key_of(parts: impl std::hash::Hash) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut h);
    // never 0: an image key of 0 reads as none
    h.finish() | 1
}

/// Images the doll has built (its backdrop, its skin at each breath),
/// dropped oldest first past [`IMAGES_KEPT`].
fn images() -> &'static Mutex<(u64, HashMap<u64, (Arc<LitImage>, u64)>)> {
    static IMAGES: OnceLock<Mutex<(u64, HashMap<u64, (Arc<LitImage>, u64)>)>> = OnceLock::new();
    IMAGES.get_or_init(Default::default)
}

fn cached(key: u64, make: impl FnOnce() -> LitImage) -> Arc<LitImage> {
    let mut guard = images().lock().unwrap_or_else(|e| e.into_inner());
    let (tick, map) = &mut *guard;
    *tick += 1;
    let now = *tick;
    if let Some((img, used)) = map.get_mut(&key) {
        *used = now;
        return img.clone();
    }
    if map.len() >= IMAGES_KEPT {
        if let Some(oldest) = map.iter().min_by_key(|(_, (_, used))| *used).map(|(k, _)| *k) {
            map.remove(&oldest);
        }
    }
    let img = Arc::new(make());
    map.insert(key, (img.clone(), now));
    img
}

/// A light from the sky at (`azimuth`, `elevation`) degrees -- 0 behind the
/// viewer, -90 screen-left -- as the pass's light at infinity.
fn sky_light(cam: &FieldCamera, azimuth: f32, elevation: f32, colour: [f32; 3], power: f32) -> FieldLight {
    let up = cam.up();
    let deeper = [0.0, up[2], -up[1]];
    let (sa, ca) = azimuth.to_radians().sin_cos();
    let (se, ce) = elevation.to_radians().sin_cos();
    let toward = [0, 1, 2].map(|i| ce * (sa * [1.0, 0.0, 0.0][i] - ca * deeper[i]) + se * up[i]);
    FieldLight::moon(toward, colour, power, 0.0, [0.0, 0.0])
}

/// The background: the sky from its top to the horizon (a wall indoors),
/// the ground below, as a backdrop the pass lights like any other.
fn backdrop(sky: &DollSky, cam: &FieldCamera) -> Arc<LitImage> {
    let [sw, sh] = cam.size;
    let (w, h) = (((sw * BACKDROP_SHARE).round() as usize).max(8), ((sh * BACKDROP_SHARE).round() as usize).max(8));
    let bits = |c: [f32; 3]| c.map(f32::to_bits);
    let key = key_of(("doll backdrop", w, h, cam.cy.to_bits(), cam.f.to_bits(), bits(sky.sky_top), bits(sky.sky_horizon), bits(sky.ground), bits(sky.floor)));
    cached(key, || {
        let mut albedo = Vec::with_capacity(w * h * 4);
        let mut lit = Vec::with_capacity(w * h * 4);
        for j in 0..h {
            let sy = (j as f32 + 0.5) * sh / h as f32;
            let (a, l) = if sy < cam.cy {
                let t = (sy / cam.cy.max(1.0)).clamp(0.0, 1.0).powf(1.6);
                let c = [0, 1, 2].map(|i| sky.sky_top[i] + (sky.sky_horizon[i] - sky.sky_top[i]) * t);
                (c, c)
            } else {
                // the ground, darkening toward the viewer's feet so the doll
                // stands out of it
                let t = ((sy - cam.cy) / (sh - cam.cy).max(1.0)).clamp(0.0, 1.0);
                let c = sky.ground.map(|g| g * (1.0 - 0.35 * t));
                (c, [0, 1, 2].map(|i| c[i] * sky.floor[i]))
            };
            let (a, l) = (srgb(a), srgb(l));
            for _ in 0..w {
                albedo.extend_from_slice(&[a[0], a[1], a[2], 255]);
                lit.extend_from_slice(&[l[0], l[1], l[2], 0]);
            }
        }
        let geo = crate::field_gpu_stage::ground_geo(cam, w, h, [0.0, 0.0, sw, sh]);
        LitImage { key, w: w as u32, h: h as u32, albedo: Arc::new(albedo), lit: Arc::new(lit), geo: Arc::new(geo) }
    })
}

/// The doll's skin as the pass's layer: its colours dimmed to the sky's
/// floor -- what shows where no light reaches -- over the paint the lights
/// shine on.
/// A moving figure (`moving`) is new every frame: built, not kept.
fn figure(posed: &PosedPuppet, sky: &DollSky, moving: bool) -> Option<Arc<LitImage>> {
    let (skin, surface) = (posed.skin.as_ref()?, posed.surface.as_ref()?);
    if skin.is_empty() || skin.w != surface.w || skin.h != surface.h {
        return None;
    }
    let key = key_of(("doll skin", Arc::as_ptr(skin) as usize, skin.w, skin.h, sky.floor.map(f32::to_bits)));
    let make = || {
        let mut dim = (**skin).clone();
        for p in dim.rgba.chunks_exact_mut(4) {
            for c in 0..3 {
                p[c] = l2s(s2l(p[c]) * sky.floor[c]);
            }
        }
        crate::field_gpu_stage::skin_image(key, &dim, surface, M_PER_FT)
    };
    Some(if moving { Arc::new(make()) } else { cached(key, make) })
}

fn nerve_rank(key: &PuppetKey) -> u8 {
    gs_field::dolls::part_label("nsys").map_or(0, |l| key.ranks[l as usize - 1])
}

/// Paint the doll into `rect`: `family`'s puppet as `key` says (its height
/// is set here, to the window's), under `sky`. `id` names it for its
/// breathing cycle, and for the flinch of a wound rank rising and the ease
/// of a pose changing, as the field's creatures (None: it holds still).
/// Asks for the repaints those need. False when the puppet does not resolve.
/// `gpu_skin` draws its skin on the GPU (`skin_gpu`) when the figure's
/// mesh covers it, the CPU skin otherwise.
#[allow(clippy::too_many_arguments)]
pub fn paint(ui: &egui::Ui, rect: egui::Rect, cache: &mut PuppetCache, family: &Arc<str>, key: PuppetKey, sky: &DollSky, id: Option<&str>, gpu_skin: bool) -> bool {
    let ctx = ui.ctx();
    let now = ctx.input(|i| i.time);
    cache.begin_frame(ctx.cumulative_frame_nr());
    let standing_px = rect.height() * FIGURE_SHARE;
    let mut key = key;
    key.height_px = size_step(standing_px * ctx.pixels_per_point());
    // hit on the back: turned round to show it, a moment
    let back = gs_field::dolls::part_label("back");
    let turning = id.and_then(|id| cache.last_hit(id)).filter(|(label, _)| Some(*label) == back).and_then(|(_, at)| show_back(now - at));
    if let Some(turn) = turning {
        key = key.with_facing(key.yaw() + turn);
        ctx.request_repaint();
    }
    if let Some((step, until)) = id.and_then(|id| cache.breath(family, id, key.pose, now)) {
        key = key.with_breath(step);
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(until));
    }
    let gpu = field_gpu::available(ctx);
    let (posed, moving) = match (gpu, id) {
        (true, Some(id)) => match cache.posed_live(family, &key, id, now, if gpu_skin { Bare::Mesh } else { Bare::Surface }) {
            Some(p) => p,
            None => return false,
        },
        (true, None) => match cache.posed_surface(family, &key) {
            Some(p) => (p, false),
            None => return false,
        },
        (false, _) => match cache.posed_with(family, &key, true) {
            Some(p) => (p, false),
            None => return false,
        },
    };
    let moving = moving || turning.is_some();
    // a rank-3 nerve wound twitches every few seconds: look again soon
    if id.is_some() && nerve_rank(&key) >= 3 {
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }
    if moving {
        ctx.request_repaint();
    }
    let foot = egui::pos2(rect.center().x, rect.top() + rect.height() * FEET_AT);
    let place = PuppetPlacement::fit(&posed, foot, standing_px);
    let painter = ui.painter_at(rect);
    // the layer, and where its image goes: the GPU skin's or the CPU's
    let layer = if !gpu {
        None
    } else if let Some((mesh, texture)) = &posed.mesh {
        let key = key_of(("doll gpu skin", mesh.key, sky.floor.map(f32::to_bits)));
        crate::skin_gpu::submit(key, Arc::new(crate::skin_gpu::SkinJob { figure: crate::skin_gpu::Figure::Skin { mesh: mesh.clone(), texture: texture.clone() }, lit: crate::skin_gpu::SkinLit::Floor(sky.floor), m_per_ft: M_PER_FT }));
        let frame = gs_puppet::Skin { origin: mesh.origin, w: mesh.w, h: mesh.h, rgba: Vec::new() };
        let image = LitImage { key, w: mesh.w as u32, h: mesh.h as u32, albedo: Arc::new(Vec::new()), lit: Arc::new(Vec::new()), geo: Arc::new(Vec::new()) };
        Some((Arc::new(image), place.skin_rect(&frame)))
    } else {
        figure(&posed, sky, moving).zip(posed.skin.as_ref().map(|s| place.skin_rect(s)))
    };
    let Some((image, at)) = layer else {
        // unlit, on a plain ground
        painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(40, 42, 46));
        place.paint_figure(&painter, &posed);
        return true;
    };
    let (w, h) = (rect.width().max(1.0), rect.height().max(1.0));
    let fig_m = posed.standing_ft.max(0.1) * M_PER_FT;
    let z = DISTANCE_HEIGHTS * fig_m;
    let f = standing_px * DISTANCE_HEIGHTS;
    let eye = EYE_SHARE * fig_m;
    let camera = FieldCamera { size: [w, h], f, cx: w * 0.5, cy: h * FEET_AT - f * eye / z, pitch: 0.0, height: eye };
    let layer = LayerDraw { image, rect: [at.min.x - rect.min.x, at.min.y - rect.min.y, at.width(), at.height()], feet_z: z, tint: 1.0 };
    let lights = sky.lights.iter().map(|l| sky_light(&camera, l.azimuth, l.elevation, l.colour, l.power)).collect();
    let frame = FieldFrame {
        id: FieldFrame::next_id(),
        camera,
        backdrop: backdrop(sky, &camera),
        backdrop_rect: [0.0, 0.0, w, h],
        layers: vec![layer],
        lights,
        time: now as f32,
        water: 0.0,
        shadows: true,
        view: FieldView::Lit,
        weather: Default::default(),
        // every pixel: one small window, and the backdrop is made small
        detail: 0.0,
        bounce: field_gpu::BOUNCE,
        stars: Default::default(),
    };
    field_gpu::paint_view(&painter, &Arc::new(Mutex::new(frame)), Placement { origin: rect.min, scale: 1.0 }, Piece::Everything, DOLL_VIEW);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_back_hit_turns_the_doll_round_and_back() {
        assert_eq!(show_back(-0.1), None);
        assert_eq!(show_back(0.0), Some(0.0));
        assert!(show_back(1.2).unwrap() > 179.0, "held round");
        assert!(show_back(SHOW_BACK_SECS - 0.1).unwrap() < 40.0, "nearly back");
        assert_eq!(show_back(SHOW_BACK_SECS), None);
    }
}
