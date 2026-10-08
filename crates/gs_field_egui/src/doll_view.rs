//! The injury doll: one puppet in a window of its own -- its wounds, lost
//! limbs, statuses, pose, breath and what it holds are all the
//! [`PuppetKey`]'s -- lit by the v2 pass ([`field_gpu`]) by the [`DollSky`]
//! it is given: as a display (`gs_field::doll_sky::display`, what a client
//! shows by default: a steady key and fill over a dark backdrop, so wounds
//! and statuses read at any hour), or under the sky round the player
//! (`gs_field::doll_sky::doll_sky`: the sun or the moons from where they
//! are round the way the player faces, a room light indoors) over that sky
//! and the ground it stands on.
//!
//! It is its own view of the pass ([`field_gpu::paint_view`], [`DOLL_VIEW`]),
//! so it draws in the same egui pass as the field, lit its own way.
//! Without the pass (no wgpu) it draws its skin unlit.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use gs_field::doll_sky::DollSky;
use gs_field::doll_state::{DollState, Status, Statuses};

use crate::creature_puppet::{body_size_step, has_body, size_step, Bare, PoseChoice, PosedPuppet, PuppetCache, PuppetKey, PuppetPlacement};
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
/// The most of the window's width a figure takes (lying down, drawn smaller).
const FIT_WIDTH: f32 = 0.92;
/// The camera: this many figure heights away, its eye this share of the
/// figure's height up.
const DISTANCE_HEIGHTS: f32 = 3.5;
const EYE_SHARE: f32 = 0.6;
const M_PER_FT: f32 = 0.3048;
/// An invisible modelled body's glints and shimmer move on a clock this
/// many seconds a tick (a new image each).
const GHOST_TICK_S: f64 = 1.0 / 30.0;
/// The backdrop is made at this share of the window's size: it is only a
/// gradient.
const BACKDROP_SHARE: f32 = 0.5;
/// The contact shadow under the doll: its half width and half depth as
/// shares of the standing height, and how dark at its middle.
const CONTACT_WIDTH: f32 = 0.16;
const CONTACT_DEPTH: f32 = 0.035;
const CONTACT_DARK: f32 = 0.55;
/// The rim: the sky behind the doll lighting its edges -- this high,
/// this strong outdoors (by the sky's horizon colour) and indoors.
const RIM_ELEVATION: f32 = 28.0;
const RIM_POWER: f32 = 0.28;
const RIM_POWER_INDOOR: f32 = 0.14;
/// The backdrop is the sky and ground a little dimmed -- the pass lights
/// it too, and at noon a true sky washes the doll out -- and darkened
/// toward the window's edges (`VIGNETTE` at the corners), so the figure
/// and what marks it stand out.
const SKY_GAIN: f32 = 0.55;
const GROUND_GAIN: f32 = 0.7;
const VIGNETTE: f32 = 0.45;
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
    // matte: a sheen off the figure bleaches its wounds and marks
    FieldLight { sheen_creature: 0.0, ..FieldLight::moon(toward, colour, power, 0.0, [0.0, 0.0]) }
}

/// The background: the sky from its top to the horizon (a wall indoors),
/// the ground below, as a backdrop the pass lights like any other. Where
/// the doll stands the ground is darkened a little (`contact`: its feet at
/// that point, its standing height, half its width, and how dark: 0 for an
/// invisible figure): the sky's own shadow under it, which no one light
/// casts, so it stands on the ground and not in front of it.
fn backdrop(sky: &DollSky, cam: &FieldCamera, contact: (egui::Pos2, f32, f32, f32)) -> Arc<LitImage> {
    let [sw, sh] = cam.size;
    let (w, h) = (((sw * BACKDROP_SHARE).round() as usize).max(8), ((sh * BACKDROP_SHARE).round() as usize).max(8));
    let bits = |c: [f32; 3]| c.map(f32::to_bits);
    let (feet, tall, half_w, dark) = contact;
    // quantized, so a breathing figure does not remake it
    let half_w = (half_w / 4.0).round() * 4.0;
    let key = key_of(("doll backdrop", w, h, cam.cy.to_bits(), cam.f.to_bits(), bits(sky.sky_top), bits(sky.sky_horizon), bits(sky.ground), bits(sky.floor), (feet.x.to_bits(), feet.y.to_bits(), tall.to_bits(), half_w.to_bits(), dark.to_bits())));
    cached(key, || {
        let mut albedo = Vec::with_capacity(w * h * 4);
        let mut lit = Vec::with_capacity(w * h * 4);
        let (rx, ry) = ((tall * CONTACT_WIDTH).max(half_w * 0.9).max(1.0), (tall * CONTACT_DEPTH).max(1.0));
        for j in 0..h {
            let sy = (j as f32 + 0.5) * sh / h as f32;
            let ground = sy >= cam.cy;
            let (a, l) = if !ground {
                let t = (sy / cam.cy.max(1.0)).clamp(0.0, 1.0).powf(1.6);
                let c = [0, 1, 2].map(|i| (sky.sky_top[i] + (sky.sky_horizon[i] - sky.sky_top[i]) * t) * SKY_GAIN);
                (c, c)
            } else {
                // the ground, darkening toward the viewer's feet so the doll
                // stands out of it
                let t = ((sy - cam.cy) / (sh - cam.cy).max(1.0)).clamp(0.0, 1.0);
                let c = sky.ground.map(|g| g * GROUND_GAIN * (1.0 - 0.35 * t));
                (c, [0, 1, 2].map(|i| c[i] * sky.floor[i]))
            };
            for i in 0..w {
                let sx = (i as f32 + 0.5) * sw / w as f32;
                let k = if ground {
                    let d = ((sx - feet.x) / rx).powi(2) + ((sy - feet.y) / ry).powi(2);
                    1.0 - dark * (-1.6 * d).exp()
                } else {
                    1.0
                };
                // the vignette: 0 in the middle, 1 at the corners
                let (vx, vy) = (sx / sw * 2.0 - 1.0, sy / sh * 2.0 - 1.0);
                let k = k * (1.0 - VIGNETTE * ((vx * vx + vy * vy) * 0.5).min(1.0));
                let (a, l) = (srgb(a.map(|c| c * k)), srgb(l.map(|c| c * k)));
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

/// Paint the doll into `rect`: `family`'s puppet as `key` says (its height
/// is set here, to the window's), under `sky`. `id` names it for its
/// breathing cycle, and for the flinch of a wound rank rising and the ease
/// of a pose changing, as the field's creatures (None: it holds still).
/// Asks for the repaints those need. False when the puppet does not resolve.
/// `gpu_skin` draws its skin on the GPU (`skin_gpu`) when the figure's
/// mesh covers it, the CPU skin otherwise. It is the pass's [`DOLL_VIEW`];
/// [`paint_in_view`] names another.
#[allow(clippy::too_many_arguments)]
pub fn paint(ui: &egui::Ui, rect: egui::Rect, cache: &mut PuppetCache, family: &Arc<str>, key: PuppetKey, sky: &DollSky, id: Option<&str>, gpu_skin: bool) -> bool {
    paint_in_view(ui, rect, cache, family, key, sky, id, gpu_skin, DOLL_VIEW)
}

/// [`paint`] as view `view` of the pass (`field_gpu::paint_view`): its own
/// lights and atlas, so several dolls -- one per character a client shows
/// -- draw in one egui pass without one's frame overwriting another's.
/// Any id but a stage's own (`0`) will do; keep it the same frame to frame.
#[allow(clippy::too_many_arguments)]
pub fn paint_in_view(ui: &egui::Ui, rect: egui::Rect, cache: &mut PuppetCache, family: &Arc<str>, key: PuppetKey, sky: &DollSky, id: Option<&str>, gpu_skin: bool, view: u64) -> bool {
    let ctx = ui.ctx();
    let now = ctx.input(|i| i.time);
    cache.begin_frame(ctx.cumulative_frame_nr());
    let standing_px = rect.height() * FIGURE_SHARE;
    let mut key = key;
    key.height_px = size_step(standing_px * ctx.pixels_per_point());
    // a form with a modelled body is drawn as it, its trained splats,
    // unless the host said otherwise; hurt, scarred, coated or under a
    // status that marks the body, as the layered cloud
    if key.auto_body && has_body(family) {
        let draw = gs_field::doll_roster::figure(family).map_or(1, |f| f.draw.body_cloud());
        key = key.with_modelled(true, draw);
    }
    // a body's pose is bones, not pixels: drawn at the window's own size
    if key.modelled {
        key.height_px = body_size_step(standing_px * ctx.pixels_per_point());
    }
    key = layered_if_marked(key);
    // hit on the back: turned round to show it, a moment
    let back = gs_field::dolls::part_label("back");
    let turning = id.and_then(|id| cache.last_hit(id)).filter(|(label, _)| Some(*label) == back).and_then(|(_, at)| show_back(now - at));
    if let Some(turn) = turning {
        key = key.with_facing(key.yaw() + turn);
        ctx.request_repaint();
    }
    // at the pace its statuses set (held still: none)
    let pace = key.breath_pace();
    if let Some((step, until)) = pace.and_then(|(pace, _)| id.and_then(|id| cache.breath_paced(family, id, key.pose, now, pace))) {
        key = key.with_breath(step);
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(until));
    }
    // a layered figure's wound arrives: opening, flashing, throwing dots off.
    // A modelled body's arrival is played from when its build is in (it is
    // made on a worker), so it is seen from the start, a moment late
    let mut arrival_age = -1.0f32;
    let mut arrival_from: Option<(egui::Id, Option<f64>)> = None;
    if let Some((label, at)) = id.and_then(|id| cache.last_hit(id)).filter(|_| key.layers) {
        key = key.with_fresh(label, now - at);
        if key.fresh.is_some() {
            ctx.request_repaint();
            let started = egui::Id::new(("gs_field_egui::doll_arrival", id, at.to_bits()));
            let since = ctx.data(|d| d.get_temp::<f64>(started));
            arrival_age = since.map_or(0.0, |t| (now - t) as f32);
            arrival_from = Some((started, since));
        }
    }
    let gpu = field_gpu::available(ctx);
    // a modelled body's arrival is built once a hit and played by the GPU
    // at its age, frame by frame (`arrival_age`): its pose is the same
    // all through it
    if gpu && key.modelled && key.layers {
        key.fresh = key.fresh.map(|(label, _)| (label, 0));
    }
    let (posed, moving) = match (gpu, id) {
        // layered splats (`PuppetKey::with_layers`) are the GPU's to draw
        (true, Some(id)) => match cache.posed_live(family, &key, id, now, if gpu_skin || key.layers || key.modelled { Bare::Mesh } else { Bare::Surface }) {
            Some(p) => p,
            None => return false,
        },
        (true, None) if key.layers || key.modelled => match cache.posed_cached(family, &key, true, Bare::Mesh) {
            Some(p) => (p, false),
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
    // a hit's arrival is in: it plays from now
    if let Some((started, None)) = arrival_from {
        if !posed.pending {
            ctx.data_mut(|d| d.insert_temp(started, now));
        }
    }
    // its modelled body still loading, or its layered cloud being made:
    // this is a stand-in, look again soon
    if posed.pending {
        ctx.request_repaint_after(std::time::Duration::from_millis(40));
    }
    // a rank-3 nerve wound twitches every few seconds: look again soon
    if id.is_some() && crate::creature_puppet::nerve_rank(&key) >= 3 {
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }
    if moving {
        ctx.request_repaint();
    }
    // its box centred in the window, so a figure lying down stays in it --
    // drawn smaller where, lying down, it would be wider than the window
    let [lo, hi] = posed.bounds;
    let wide = (hi[0] - lo[0]) * standing_px / posed.standing_ft.max(0.1);
    let standing_px = if wide > rect.width() * FIT_WIDTH { standing_px * rect.width() * FIT_WIDTH / wide } else { standing_px };
    let px_ft = standing_px / posed.standing_ft.max(0.1);
    let foot = egui::pos2(rect.center().x - (lo[0] + hi[0]) * 0.5 * px_ft, rect.top() + rect.height() * FEET_AT);
    let half_w = (hi[0] - lo[0]) * 0.5 * px_ft;
    let place = PuppetPlacement::fit(&posed, foot, standing_px);
    let painter = ui.painter_at(rect);
    // the layer, and where its image goes: the GPU skin's or the CPU's
    let layer = if !gpu {
        None
    } else if let Some((mesh, texture)) = &posed.mesh {
        let key = key_of(("doll gpu skin", mesh.key, sky.floor.map(f32::to_bits)));
        crate::skin_gpu::submit(key, Arc::new(crate::skin_gpu::SkinJob { figure: crate::skin_gpu::Figure::Skin { mesh: mesh.clone(), texture: texture.clone() }, lit: crate::skin_gpu::SkinLit::Floor(sky.floor), m_per_ft: M_PER_FT, arrival_age: -1.0, clock: 0.0 }));
        let frame = gs_puppet::Skin { origin: mesh.origin, w: mesh.w, h: mesh.h, rgba: Vec::new() };
        let image = LitImage { key, w: mesh.w as u32, h: mesh.h as u32, albedo: Arc::new(Vec::new()), lit: Arc::new(Vec::new()), geo: Arc::new(Vec::new()) };
        Some((Arc::new(image), place.skin_rect(&frame)))
    } else if let Some(body) = &posed.body {
        // its modelled body, posed and rasterised by the GPU
        // (a hit arriving: a new image every frame, at its age; invisible,
        // its glints and shimmer on a running clock, a new image a tick)
        let clock = if key.layers && key.effects.invisible {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(GHOST_TICK_S));
            ((now / GHOST_TICK_S).floor() * GHOST_TICK_S % 3600.0) as f32
        } else {
            0.0
        };
        let key = key_of(("doll gpu body", body.key, sky.floor.map(f32::to_bits), arrival_age.to_bits(), clock.to_bits()));
        crate::skin_gpu::submit(key, Arc::new(crate::skin_gpu::SkinJob { figure: crate::skin_gpu::Figure::Body(body.clone()), lit: crate::skin_gpu::SkinLit::Floor(sky.floor), m_per_ft: M_PER_FT, arrival_age, clock }));
        let frame = gs_puppet::Skin { origin: body.origin, w: body.w, h: body.h, rgba: Vec::new() };
        let image = LitImage { key, w: body.w as u32, h: body.h as u32, albedo: Arc::new(Vec::new()), lit: Arc::new(Vec::new()), geo: Arc::new(Vec::new()) };
        Some((Arc::new(image), place.skin_rect(&frame)))
    } else if let Some(cloud) = &posed.splats {
        // its splat cloud (the layered one), rasterised by the same pass
        let key = key_of(("doll gpu splats", cloud.key, sky.floor.map(f32::to_bits)));
        crate::skin_gpu::submit(key, Arc::new(crate::skin_gpu::SkinJob { figure: crate::skin_gpu::Figure::Splats(cloud.clone()), lit: crate::skin_gpu::SkinLit::Floor(sky.floor), m_per_ft: M_PER_FT, arrival_age: -1.0, clock: 0.0 }));
        let frame = gs_puppet::Skin { origin: cloud.origin, w: cloud.w, h: cloud.h, rgba: Vec::new() };
        let image = LitImage { key, w: cloud.w as u32, h: cloud.h as u32, albedo: Arc::new(Vec::new()), lit: Arc::new(Vec::new()), geo: Arc::new(Vec::new()) };
        Some((Arc::new(image), place.skin_rect(&frame)))
    } else {
        figure(&posed, sky, moving).zip(posed.skin.as_ref().map(|s| place.skin_rect(s)))
    };
    let Some((image, at)) = layer else {
        // unlit, on a plain ground
        painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(40, 42, 46));
        place.paint_figure(&painter, &posed);
        signs(ctx, &painter, key.statuses, key.layers, &posed, foot, standing_px, now);
        return true;
    };
    let (w, h) = (rect.width().max(1.0), rect.height().max(1.0));
    let unseen = key.effects.invisible;
    let fig_m = posed.standing_ft.max(0.1) * M_PER_FT;
    let z = DISTANCE_HEIGHTS * fig_m;
    let f = standing_px * DISTANCE_HEIGHTS;
    let eye = EYE_SHARE * fig_m;
    let camera = FieldCamera { size: [w, h], f, cx: w * 0.5, cy: h * FEET_AT - f * eye / z, pitch: 0.0, height: eye };
    let layer = LayerDraw { image, rect: [at.min.x - rect.min.x, at.min.y - rect.min.y, at.width(), at.height()], feet_z: z, tint: 1.0 };
    let mut lights: Vec<FieldLight> = sky.lights.iter().map(|l| sky_light(&camera, l.azimuth, l.elevation, l.colour, l.power)).collect();
    // the sky behind it rims its edges: last, so the sun and moons keep
    // the shadow slots
    let rim = if sky.indoor { RIM_POWER_INDOOR } else { RIM_POWER };
    lights.push(sky_light(&camera, 180.0, RIM_ELEVATION, sky.sky_horizon, rim));
    let frame = FieldFrame {
        id: FieldFrame::next_id(),
        camera,
        // (an invisible figure throws no shadow, nor darkens the ground)
        backdrop: backdrop(sky, &camera, (egui::pos2(rect.center().x, foot.y) - rect.min.to_vec2(), standing_px, half_w, if unseen { 0.0 } else { CONTACT_DARK })),
        backdrop_rect: [0.0, 0.0, w, h],
        layers: vec![layer],
        lights,
        time: now as f32,
        water: 0.0,
        shadows: !unseen,
        view: FieldView::Lit,
        weather: Default::default(),
        // every pixel: one small window, and the backdrop is made small
        detail: 0.0,
        bounce: field_gpu::BOUNCE,
        stars: Default::default(),
    };
    field_gpu::paint_view(&painter, &Arc::new(Mutex::new(frame)), Placement { origin: rect.min, scale: 1.0 }, Piece::Everything, view);
    signs(ctx, &painter, key.statuses, key.layers, &posed, foot, standing_px, now);
    true
}

/// `text` at `at`, `size` points, over a dark shadow so it reads on a
/// bright sky as on a dark one.
fn shadowed_text(painter: &egui::Painter, at: egui::Pos2, text: &str, size: f32, colour: egui::Color32) {
    let font = egui::FontId::proportional(size);
    let shade = egui::Color32::from_black_alpha((colour.a() as f32 * 0.8) as u8);
    for d in [egui::vec2(1.2, 1.2), egui::vec2(-1.0, 1.0), egui::vec2(1.0, -1.0)] {
        painter.text(at + d, egui::Align2::CENTER_CENTER, text, font.clone(), shade);
    }
    painter.text(at, egui::Align2::CENTER_CENTER, text, font, colour);
}

/// How far the doll is turned from facing the viewer when the host does
/// not say: a little, so it reads as a figure and not a cut-out.
pub const DOLL_TURN_DEG: f32 = 20.0;

/// The body colour of a figure the host gives none.
pub const DOLL_BODY: [u8; 3] = [150, 140, 130];

/// The layered cloud's spacing for the doll (feet): finer than the
/// stage's cloud, one figure seen close.
pub const LAYERS_SPACING_FT: f32 = 0.015;

/// Whether `key` has anything on the body a modelled body's plain cloud
/// cannot show: a wound, a scar, a coating, death, or a status drawn on
/// the body (poison, disease, bleeding..., a web, ice).
pub fn body_marked(key: &PuppetKey) -> bool {
    key.ranks.iter().chain(&key.scars).any(|r| *r > 0) || key.coat.material > 0 || key.dead || key.effects.any()
}

/// `key` as the doll draws it when it is a modelled body's splats
/// ([`PuppetKey::with_modelled`], cloud 1 or 2) and [`body_marked`]: the
/// LAYERED cloud ([`PuppetKey::with_layers`]), which wears the wounds,
/// marks and coating; its causes and crits stay as the host set them. Any
/// other key comes back as it was. The studio's doll and a game client
/// share this rule, so they draw the same doll.
pub fn layered_if_marked(key: PuppetKey) -> PuppetKey {
    if key.layers || !key.modelled || key.body_cloud == 0 || !body_marked(&key) {
        return key;
    }
    let params = gs_puppet::splat::SplatParams { spacing: LAYERS_SPACING_FT, ..Default::default() };
    key.with_layers(true, params.into(), key.coat.coating())
}

/// The key a game state draws the doll with (ADDITION): its wounds, scars
/// and statuses -- the pose they ask for, what they draw on the body, how
/// it holds itself and breathes -- turned [`DOLL_TURN_DEG`].
pub fn key_for(state: &DollState) -> PuppetKey {
    let s = state.statuses.resolved();
    PuppetKey::new(PoseChoice::of_statuses(s), &state.wounds, s.has(Status::Dead), false, DOLL_BODY, false, 300.0)
        .with_facing(DOLL_TURN_DEG)
        .with_scars(&state.scars)
        .with_statuses(s)
}

/// [`paint`] the doll as a game state has it ([`key_for`]): what a game
/// client calls, its [`DollState`] filled from its own state
/// (`DollState::from_game`), as view `view` of the pass
/// ([`paint_in_view`]: one id per doll shown at once; [`DOLL_VIEW`] for a
/// single one).
#[allow(clippy::too_many_arguments)]
pub fn paint_state(ui: &egui::Ui, rect: egui::Rect, cache: &mut PuppetCache, family: &Arc<str>, state: &DollState, sky: &DollSky, id: Option<&str>, gpu_skin: bool, view: u64) -> bool {
    paint_in_view(ui, rect, cache, family, key_for(state), sky, id, gpu_skin, view)
}

/// The signs a status floats over the figure (ADDITION), drawn over the
/// lit doll: stars circling a stunned or dazed head, a sleeper's Zs, a
/// confused one's question marks, a bleeder's falling drops, a poisoned
/// one's rising bubbles, a haze over blinded eyes, a hush by a silenced
/// mouth, a red ring round the feet of the vulnerable, pale motes round
/// the calmed. Asks for the next frame while any shows. `built`: the
/// figure is layered splats, which build its blood and its poison on it
/// (`gs_puppet`'s `blood`, `afflict`), so it has no drops or bubbles
/// floated over it.
#[allow(clippy::too_many_arguments)]
fn signs(ctx: &egui::Context, painter: &egui::Painter, s: Statuses, built: bool, posed: &PosedPuppet, foot: egui::Pos2, standing_px: f32, now: f64) {
    use egui::{pos2, vec2, Color32, Shape, Stroke};
    if s.is_empty() || s.has(Status::Dead) {
        return;
    }
    let u = standing_px.max(8.0);
    let px_ft = u / posed.standing_ft.max(0.1);
    let [lo, hi] = posed.bounds;
    let mid_x = foot.x + (lo[0] + hi[0]) * 0.5 * px_ft;
    let half_w = ((hi[0] - lo[0]) * 0.5 * px_ft).max(u * 0.08);
    // how tall it stands now (lying down, little)
    let body = (hi[1] * px_ft).max(u * 0.05);
    // the head: its own box, else the top of the figure
    let [hlo, hhi] = posed.head.unwrap_or([[lo[0], hi[1] - 0.12 * posed.standing_ft, lo[2]], hi]);
    let top = foot.y - hhi[1] * px_ft;
    let head_r = ((hhi[1] - hlo[1]) * 0.5 * px_ft).max(u * 0.03);
    let head = pos2(foot.x + (hlo[0] + hhi[0]) * 0.5 * px_ft, foot.y - (hlo[1] + hhi[1]) * 0.5 * px_ft);
    let eyes_y = foot.y - (hlo[1] + (hhi[1] - hlo[1]) * 0.58) * px_ft;
    let t = now as f32;
    let tau = std::f32::consts::TAU;
    let fract = |v: f32| v - v.floor();
    let rgba = |r: u8, g: u8, b: u8, a: f32| Color32::from_rgba_unmultiplied(r, g, b, (a.clamp(0.0, 1.0) * 255.0) as u8);
    let mut shown = false;

    if s.has(Status::Stunned) || s.has(Status::Dazed) {
        shown = true;
        for k in 0..3 {
            let a = t * 2.4 + k as f32 * tau / 3.0;
            let c = pos2(head.x + a.cos() * u * 0.1, top - u * 0.02 + a.sin() * u * 0.025);
            let near = 0.5 + 0.5 * a.sin();
            let r = u * (0.026 + 0.012 * near);
            let pts: Vec<egui::Pos2> = (0..10)
                .map(|i| {
                    let ang = -tau / 4.0 + i as f32 * tau / 10.0 + t * 1.5;
                    let rr = if i % 2 == 0 { r } else { r * 0.45 };
                    pos2(c.x + ang.cos() * rr, c.y + ang.sin() * rr)
                })
                .collect();
            painter.add(Shape::convex_polygon(pts.clone(), rgba(255, 226, 90, 0.55 + 0.45 * near), Stroke::NONE));
            painter.add(Shape::closed_line(pts, Stroke::new(1.6, rgba(70, 40, 0, 0.75 + 0.25 * near))));
        }
    }
    if s.has(Status::Sleeping) {
        shown = true;
        for k in 0..3 {
            let p = fract(t / 2.6 + k as f32 / 3.0);
            let at = pos2(head.x + u * (0.06 + 0.1 * p) + (p * tau).sin() * u * 0.015, top - u * 0.02 - p * u * 0.2);
            let size = u * (0.055 + 0.06 * p);
            shadowed_text(painter, at, "z", size, rgba(225, 235, 255, (1.0 - p) * 1.4));
        }
    }
    if s.has(Status::Confused) {
        shown = true;
        for k in 0..2 {
            let a = t * 1.3 + k as f32 * std::f32::consts::PI;
            let at = pos2(head.x + a.cos() * u * 0.11, top - u * 0.05 + (t * 3.0 + k as f32).sin() * u * 0.02);
            shadowed_text(painter, at, "?", u * 0.09, rgba(222, 200, 255, 0.75 + 0.25 * a.sin()));
        }
    }
    if s.has(Status::Bleeding) && !built {
        shown = true;
        for k in 0..5 {
            let seed = k as f32 * 0.618;
            let p = fract(t / 1.4 + seed);
            let x = mid_x + (fract(seed * 7.3) - 0.5) * half_w * 0.9;
            let from = foot.y - body * (0.35 + 0.3 * fract(seed * 3.1));
            let y = from + (foot.y - from) * p * p;
            let r = u * 0.012;
            let red = rgba(170, 10, 14, if p > 0.92 { (1.0 - p) * 12.0 } else { 1.0 });
            painter.circle_filled(pos2(x, y), r, red);
            painter.add(Shape::convex_polygon(vec![pos2(x - r * 0.8, y - r * 0.3), pos2(x + r * 0.8, y - r * 0.3), pos2(x, y - r * 2.4)], red, Stroke::NONE));
        }
        // a pool gathering under it
        painter.add(Shape::ellipse_filled(foot, vec2(u * 0.07, u * 0.012), rgba(110, 8, 12, 0.7)));
    }
    if s.has(Status::Poisoned) && !built {
        shown = true;
        for k in 0..6 {
            let seed = k as f32 * 0.37;
            let p = fract(t / 2.8 + seed);
            let x = mid_x + (fract(seed * 5.7) - 0.5) * half_w * 1.4 + (p * tau * 1.5 + k as f32).sin() * u * 0.012;
            let y = foot.y - body * 0.4 - u * 0.8 * p;
            let r = u * (0.012 + 0.012 * fract(seed * 9.1)) * (0.6 + 0.6 * p);
            let a = (1.0 - p).min(1.0);
            painter.circle_stroke(pos2(x, y), r + 1.0, Stroke::new(1.5, rgba(20, 50, 10, a * 0.7)));
            painter.circle(pos2(x, y), r, rgba(110, 210, 60, a * 0.55), Stroke::new(1.5, rgba(170, 255, 100, a)));
        }
    }
    if s.has(Status::Blinded) {
        shown = true;
        let eyes = pos2(head.x, eyes_y);
        painter.add(Shape::ellipse_filled(eyes, vec2(head_r * 1.15, head_r * 0.32), rgba(18, 16, 22, 0.78)));
        for k in 0..4 {
            let p = fract(t / 3.0 + k as f32 * 0.25);
            let c = pos2(eyes.x + (k as f32 - 1.5) * head_r * 0.6 + p * head_r * 0.5, eyes.y - p * head_r * 0.8);
            painter.circle_filled(c, head_r * (0.25 + 0.25 * p), rgba(30, 28, 36, (1.0 - p) * 0.5));
        }
    }
    if s.has(Status::Silenced) || s.has(Status::Cutthroat) {
        // a hush by the mouth: a small circle, struck through
        let c = pos2(head.x + head_r * 1.9, eyes_y + head_r * 0.7);
        let r = u * 0.022;
        let r = r * 1.3;
        let dark = Stroke::new(4.0, rgba(20, 20, 26, 0.7));
        painter.circle_stroke(c, r, dark);
        painter.line_segment([pos2(c.x - r * 0.7, c.y + r * 0.7), pos2(c.x + r * 0.7, c.y - r * 0.7)], dark);
        let light = Stroke::new(2.0, rgba(235, 235, 245, 1.0));
        painter.circle_stroke(c, r, light);
        painter.line_segment([pos2(c.x - r * 0.7, c.y + r * 0.7), pos2(c.x + r * 0.7, c.y - r * 0.7)], light);
    }
    if s.has(Status::Vulnerable) {
        shown = true;
        let pulse = 0.5 + 0.5 * (t * 3.2).sin();
        let ring = (u * 0.14).max(half_w * 0.8);
        painter.add(Shape::ellipse_stroke(pos2(mid_x, foot.y), vec2(ring + u * 0.015 * pulse, u * 0.028 + u * 0.005 * pulse), Stroke::new(3.0, rgba(240, 40, 30, 0.6 + 0.4 * pulse))));
    }
    if s.has(Status::Calmed) {
        shown = true;
        for k in 0..4 {
            let a = t * 0.6 + k as f32 * tau / 4.0;
            let c = pos2(mid_x + a.cos() * half_w * 1.3, top + u * (0.25 + 0.12 * (a * 1.7).sin()));
            painter.circle_filled(c, u * 0.013, rgba(180, 220, 255, 0.6 + 0.3 * (a * 2.0).sin()));
        }
    }
    if shown {
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_modelled_body_is_layered_only_once_something_is_on_it() {
        let healthy = DollState { wounds: vec![], scars: vec![], statuses: Statuses::NONE };
        let key = key_for(&healthy);
        assert!(key.auto_body && !key.modelled, "the doll decides");
        assert_eq!(layered_if_marked(key), key, "not modelled: untouched");
        let body = key.with_modelled(true, 1);
        assert!(!body.auto_body);
        assert_eq!(layered_if_marked(body), body, "nothing on it: the plain cloud");
        assert_eq!(layered_if_marked(key.with_modelled(true, 0)), key.with_modelled(true, 0), "its mesh: never layered");
        let hurt = DollState { wounds: vec![("chest".to_string(), 2)], scars: vec![], statuses: Statuses::NONE };
        let layered = layered_if_marked(key_for(&hurt).with_modelled(true, 1));
        assert!(layered.layers && layered.splats && layered.modelled);
        assert_eq!(layered_if_marked(layered), layered, "applied twice is the same key");
        let scarred = DollState { wounds: vec![], scars: vec![("head".to_string(), 1)], statuses: Statuses::NONE };
        assert!(layered_if_marked(key_for(&scarred).with_modelled(true, 1)).layers);
        let sick = DollState { wounds: vec![], scars: vec![], statuses: Statuses::NONE.with(Status::Poisoned) };
        assert!(layered_if_marked(key_for(&sick).with_modelled(true, 1)).layers, "a status drawn as marks");
        for status in [Status::Webbed, Status::Frozen] {
            let held = DollState { wounds: vec![], scars: vec![], statuses: Statuses::NONE.with(status) };
            assert!(layered_if_marked(key_for(&held).with_modelled(true, 1)).layers, "{status:?}: a web and ice are layers");
        }
    }

    #[test]
    fn a_game_state_keys_its_pose_marks_and_breath() {
        let state = DollState::from_game([("chest", 2), ("nerves", 3)], [("rightArm", 1)], ["IconKNEELING", "Major Bleed", "cutthroat", "Staggered"]);
        let key = key_for(&state);
        assert_eq!(key.pose, PoseChoice::Kneeling);
        assert!(key.effects.bleeding && !key.effects.webbed);
        let label = |p: &str| gs_field::dolls::part_label(p).unwrap() as usize - 1;
        assert_eq!(key.ranks[label("chest")], 2);
        assert_eq!(key.ranks[label("neck")], 2, "a cut throat is a neck wound");
        assert_eq!(key.scars[label("rightArm")], 1);
        assert_eq!(key.breath_pace(), Some((1.0, 1.0)));
        let held = key_for(&DollState { statuses: Statuses::NONE.with(Status::Bound), ..DollState::default() });
        assert_eq!(held.breath_pace(), None, "bound fast, it does not breathe");
        let dead = key_for(&DollState { statuses: Statuses::NONE.with(Status::Dead).with(Status::Stunned), ..DollState::default() });
        assert!(dead.dead && matches!(dead.pose, PoseChoice::Dead { .. }));
    }

    #[test]
    fn a_back_hit_turns_the_doll_round_and_back() {
        assert_eq!(show_back(-0.1), None);
        assert_eq!(show_back(0.0), Some(0.0));
        assert!(show_back(1.2).unwrap() > 179.0, "held round");
        assert!(show_back(SHOW_BACK_SECS - 0.1).unwrap() < 40.0, "nearly back");
        assert_eq!(show_back(SHOW_BACK_SECS), None);
    }
}
