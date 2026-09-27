//! ADDITION (not in VellumFE): point lights on the field -- campfires,
//! torches, glows (`StageScene.lights`, Niffy's idea).
//!
//! A light stands on the floor, so everything about it goes through the
//! solver's own ground projection: the warm pool it casts on the ground is
//! an ellipse lying in the floor's perspective, and a creature's distance to
//! it is measured in world units along the floor.
//!
//! Creatures are brightened, not just tinted: a tint can only darken, and
//! at night the lit art IS darker. So over a creature's lit art the
//! renderer lays its UNLIT art again, tinted the fire's colour, at an
//! opacity that falls off with distance and is stronger on the side facing
//! the fire (a gradient across the sprite) -- the painting as the fire
//! would light it. Flicker is that opacity moving, so it costs nothing per
//! frame: no re-baking.
//!
//! A puppet is lit as a body instead ([`puppet_lamps`]): each flame is a
//! point in the figure's own space, and every pixel of its skin takes the
//! light that reaches it there -- by its distance from the flame and how
//! squarely it faces it (`gs_puppet::SkinSurface::lamp_light`) -- added
//! over the lit skin, the flicker scaling it.
//!
//! The ground pool, over a lit backdrop, is the backdrop as it is by day
//! (its lit twin's original), fire-tinted and fading out: the fire lights
//! the painted ground rather than laying colour on it. A glow hangs in the
//! air round each flame, added. A puppet casts a shadow on the ground away
//! from each lamp that reaches it (`gs_puppet::ShadowCaster`).
//!
//! A lamp with a beam (`PointLight::spread` under 360) lights only what lies
//! within it, by bearing across the floor (`PointLight::beam`): its pool is
//! a wedge, the glow in the air round it is thrown the same way, and
//! creatures outside it take none of its light or shadow. A lantern is made
//! that way. A wisp has no flame -- a ball of light that drifts and bobs
//! (`PointLight::drift`), so wherever a light's place is asked for it is
//! asked for NOW ([`at`]).
//!
//! Hooks in `creature_field.rs` (marked regions): [`set_creature`] as a
//! card starts, [`paint_sprite_glow`] after its base art,
//! [`paint_prop_glow`] after a scenery prop's, [`paint_back`] before the
//! cards (ground pools), [`paint_lamps_behind`] before each card or prop
//! (the flames farther away than it, so a lamp sits in depth order among
//! them) and [`paint_front`] after the last (the lamps nearer than all).

use std::cell::RefCell;

use egui::{pos2, vec2, Color32, Pos2};
use gs_field::lighting::{LampKind, PointLight};

/// Strongest the fire's copy of a creature ever gets.
const MAX_GLOW: f32 = 0.8;
/// World height a campfire's flame reaches (the depth axis's units).
const FLAME_H: f32 = 0.3;
const TORCH_POLE_H: f32 = 0.34;
/// Opacity of the day-lit backdrop at the middle of a ground pool.
const POOL_REVEAL: f32 = 0.85;
/// How far a pool's fire tint is lifted toward white, so the ground keeps
/// its own colours under it.
const POOL_TINT_LIFT: f32 = 0.3;
/// The glow in the air round a flame: its strength, and its radius in
/// flame heights.
const HALO: f32 = 0.45;
const HALO_FLAMES: f32 = 3.2;
/// A lantern's post, world units; its lantern hangs at the top.
const LANTERN_POST_H: f32 = 0.5;
/// How high a wisp floats, world units.
const WISP_H: f32 = 1.0;
/// How much the floor's depth axis is foreshortened on screen: what turns a
/// beam's bearing across the floor into a direction on the screen.
const FORESHORTEN: f32 = 0.4;

/// The backdrop as it is by day (the lit twin's original) and where it is
/// drawn on the stage, `[x0, y0, x1, y1]`: what a ground pool shows.
#[derive(Debug, Clone, Copy)]
pub struct Backdrop {
    pub texture: egui::TextureId,
    pub stage: [f32; 4],
}

/// A lamp as the creature being drawn sees it, in floor units about its
/// foot: `across` (+ = screen-right), `up`, `deeper` (+ = farther from the
/// viewer).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Near {
    across: f32,
    up: f32,
    deeper: f32,
    reach: f32,
    rgb: [u8; 3],
    strength: f32,
    /// Its flicker now, and how much it counts toward the figure's.
    flicker: f32,
    weight: f32,
    /// Its beam's bearing and width, degrees (360 = all round).
    aim: f32,
    spread: f32,
}

/// The fire light on the creature being drawn, per side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FireGlow {
    pub rgb: [u8; 3],
    /// Opacity of the fire-lit copy at the sprite's left and right edges.
    pub left: f32,
    pub right: f32,
}

thread_local! {
    static CURRENT: RefCell<Option<FireGlow>> = const { RefCell::new(None) };
    /// Which of the scene's lamps this frame has drawn already.
    static DRAWN: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
    /// The scene's lights this frame (set by [`paint_back`]): a prop's
    /// draw has no scene of its own to ask.
    static LIGHTS: RefCell<Vec<PointLight>> = const { RefCell::new(Vec::new()) };
    /// The lamps near the creature being drawn (set by [`set_creature`]).
    static NEAR: RefCell<Vec<Near>> = const { RefCell::new(Vec::new()) };
    /// The v2 pass (field_gpu_stage) lights this frame -- no fire-tinted
    /// copies -- and, per scene lamp, whether it is one the pass lights and
    /// draws (a torch's flame, a wisp): those get no ground pool or drawing
    /// here. A lamp past the pass's limit keeps both.
    static GPU: RefCell<Option<Vec<bool>>> = const { RefCell::new(None) };
}

/// The v2 lighting pass lights this frame, and these of the scene's lamps
/// (`StageGpu::lamps_in_pass`); None = it does not (see [`GPU`]'s docs).
pub fn set_gpu(lamps_in_pass: Option<Vec<bool>>) {
    GPU.with(|g| *g.borrow_mut() = lamps_in_pass);
}

fn gpu() -> bool {
    GPU.with(|g| g.borrow().is_some())
}

/// The pass lights (and draws) scene lamp `i`.
fn in_pass(i: usize) -> bool {
    GPU.with(|g| g.borrow().as_ref().is_some_and(|v| v.get(i).copied().unwrap_or(false)))
}

/// Height of a lamp's light above the floor, world units: the middle of
/// its flame as [`paint_lamp`] draws it.
pub(crate) fn flame_height(kind: LampKind) -> f32 {
    match kind {
        // The upper part of the flame: a tall fire lights from high in it.
        LampKind::Campfire => FLAME_H,
        LampKind::Torch => TORCH_POLE_H * 2.2 + FLAME_H * 0.4,
        LampKind::Glow => FLAME_H,
        LampKind::Lantern => LANTERN_POST_H + FLAME_H * 0.5,
        LampKind::Wisp => WISP_H,
    }
}

/// Where a light is now: its floor point plus its drift (stage x, world
/// depth), and how high its light sits above the floor (world units).
fn at(field: &gs_field::solver::CreatureField, light: &PointLight, index: usize, t: f32) -> (f32, f32, f32) {
    let d = light.drift(index, t);
    let (_, ppu) = field.project_ground(light.x, light.z);
    (light.x + d[0] * ppu, light.z + d[2], flame_height(light.kind) + d[1])
}

fn hex(c: &str) -> [u8; 3] {
    let h = c.trim().trim_start_matches('#');
    let p = |i: usize| h.get(i..i + 2).and_then(|s| u8::from_str_radix(s, 16).ok()).unwrap_or(200);
    [p(0), p(2), p(4)]
}

/// The light's brightness now: 1 when steady, dipping as it flickers.
pub fn flicker(light: &PointLight, index: usize, t: f32) -> f32 {
    let seed = index as f32 * 1.7;
    let n = (t * 7.3 + seed).sin() * 0.5 + (t * 13.1 + seed * 2.3).sin() * 0.3 + (t * 23.7 + seed * 0.7).sin() * 0.2;
    1.0 - light.flicker.clamp(0.0, 1.0) * 0.28 * (0.5 + 0.5 * n)
}

/// How strongly `light` reaches a point on the floor, 0..1 (smooth falloff
/// to nothing at its radius), and which way it lies (-1 left .. 1 right).
fn reach(field: &gs_field::solver::CreatureField, light: &PointLight, index: usize, t: f32, x: f32, z: f32) -> (f32, f32) {
    let (lx, lz, _) = at(field, light, index, t);
    let (_, ppu_l) = field.project_ground(lx, lz);
    let (_, ppu_c) = field.project_ground(x, z);
    let ppu = ((ppu_l + ppu_c) * 0.5).max(1e-3);
    let (dx, dz) = ((lx - x) / ppu, lz - z);
    let d = dx.hypot(dz);
    let r = light.radius.max(0.05);
    let fall = (1.0 - (d / r).powi(2)).max(0.0).powi(2) * light.beam(-dx, -dz);
    (fall * light.intensity.clamp(0.0, 1.0), if d > 1e-3 { (dx / d).clamp(-1.0, 1.0) } else { 0.0 })
}

/// The creature about to be drawn stands on `unit`: work out its fire glow
/// (None when no light reaches it) for [`paint_sprite_glow`] and friends.
pub fn set_creature(
    scene: Option<&gs_field::scenes::StageScene>,
    field: &gs_field::solver::CreatureField,
    unit: &gs_field::solver::Unit,
    now_ms: f64,
) {
    let (fx, _) = field.foot(unit);
    let (z, t) = (field.ground_z(unit), now_ms as f32 / 1000.0);
    let lights = scene.map(|s| s.lights.as_slice()).unwrap_or_default();
    let glow = glow_at(lights, field, fx, z, t);
    CURRENT.with(|g| *g.borrow_mut() = glow);
    let near = near_lamps(lights, field, fx, z, t);
    NEAR.with(|n| *n.borrow_mut() = near);
}

/// The lamps that may reach something standing at floor point (x, z):
/// within their reach, with room for a body's own size.
fn near_lamps(lights: &[PointLight], field: &gs_field::solver::CreatureField, x: f32, z: f32, t: f32) -> Vec<Near> {
    let (_, ppu_c) = field.project_ground(x, z);
    lights
        .iter()
        .enumerate()
        .filter_map(|(i, light)| {
            let (lx, lz, up) = at(field, light, i, t);
            let (_, ppu_l) = field.project_ground(lx, lz);
            let ppu = ((ppu_l + ppu_c) * 0.5).max(1e-3);
            let (across, deeper) = ((lx - x) / ppu, lz - z);
            let r = light.radius.max(0.05);
            let d = across.hypot(deeper);
            // Its beam must reach near the creature (it lies at -across,
            // -deeper from the lamp); a body's width of slack.
            let aimed = light.beam(-across, -deeper).max(light.beam(-across * 0.8, -deeper * 1.2)).max(light.beam(-across * 1.2, -deeper * 0.8));
            (d < r * 1.3 && light.intensity > 0.0 && aimed > 0.0).then(|| Near {
                across,
                up,
                deeper,
                reach: r,
                rgb: hex(&light.color),
                strength: light.intensity.clamp(0.0, 1.0),
                flicker: flicker(light, i, t),
                weight: light.intensity.max(0.0) * (1.0 - (d / (r * 1.3)).min(1.0)),
                aim: light.aim,
                spread: light.spread,
            })
        })
        .collect()
}

/// The lamps near the creature being drawn ([`set_creature`]) as a puppet
/// standing there sees them: in its skin's frame (feet about its foot, +x
/// screen-right, +z toward the viewer), `ft_per_unit` feet to a floor unit
/// there; and their flicker now, one for the lot (weighted by reach).
pub fn puppet_lamps(ft_per_unit: f32) -> (Vec<gs_puppet::Lamp>, f32) {
    NEAR.with(|n| {
        let near = n.borrow();
        let ft = ft_per_unit.max(1e-3);
        let lamps = near
            .iter()
            .map(|l| gs_puppet::Lamp {
                pos: [l.across * ft, l.up * ft, -l.deeper * ft],
                rgb: l.rgb,
                reach: l.reach * ft,
                strength: l.strength,
                // Across the floor (+x right, +z toward the viewer): a bearing
                // of 90 points away from the viewer, to -z.
                beam: (l.spread < 360.0).then(|| {
                    let (s, c) = l.aim.to_radians().sin_cos();
                    gs_puppet::Beam { dir: [c, 0.0, -s], spread: l.spread, soft: gs_field::lighting::BEAM_SOFT_DEG }
                }),
            })
            .collect();
        let weight: f32 = near.iter().map(|l| l.weight).sum();
        let flicker = if weight > 1e-6 { near.iter().map(|l| l.flicker * l.weight).sum::<f32>() / weight } else { 1.0 };
        (lamps, flicker)
    })
}

/// The fire glow on something standing at floor point (x, z) at time `t`.
fn glow_at(lights: &[PointLight], field: &gs_field::solver::CreatureField, x: f32, z: f32, t: f32) -> Option<FireGlow> {
    let (mut left, mut right, mut rgb, mut best) = (0.0f32, 0.0f32, [255u8; 3], 0.0f32);
    for (i, light) in lights.iter().enumerate() {
        let (c, side) = reach(field, light, i, t, x, z);
        if c <= 0.0 {
            continue;
        }
        let c = c * flicker(light, i, t) * MAX_GLOW;
        left += c * (0.6 - 0.4 * side);
        right += c * (0.6 + 0.4 * side);
        if c > best {
            best = c;
            rgb = hex(&light.color);
        }
    }
    (left.max(right) > 0.01).then_some(FireGlow { rgb, left: left.min(MAX_GLOW), right: right.min(MAX_GLOW) })
}

/// The fire glow on the creature being drawn.
pub fn current() -> Option<FireGlow> {
    CURRENT.with(|g| *g.borrow())
}

/// Lay `original` (the unlit art) over `dest`, tinted the fire's colour,
/// fading from the side facing the fire to the other.
pub fn paint_glow_texture(painter: &egui::Painter, original: egui::TextureId, dest: egui::Rect, glow: FireGlow) {
    let tint = |a: f32| Color32::from_rgba_unmultiplied(glow.rgb[0], glow.rgb[1], glow.rgb[2], (a.clamp(0.0, 1.0) * 255.0) as u8);
    let mut mesh = egui::Mesh::with_texture(original);
    let (l, r) = (tint(glow.left), tint(glow.right));
    for (p, uv, c) in [
        (dest.left_top(), pos2(0.0, 0.0), l),
        (dest.right_top(), pos2(1.0, 0.0), r),
        (dest.right_bottom(), pos2(1.0, 1.0), r),
        (dest.left_bottom(), pos2(0.0, 1.0), l),
    ] {
        mesh.vertices.push(egui::epaint::Vertex { pos: p, uv, color: c });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// After a sprite's base art: the fire's copy over it, if a fire reaches
/// this creature and scene lighting has a lit twin in the slot (daylight
/// art needs no brightening).
pub fn paint_sprite_glow(
    painter: &egui::Painter,
    cache: &crate::creature_art::CreatureArtCache,
    drawn: &egui::TextureHandle,
    dest: egui::Rect,
) {
    if gpu() {
        return;
    }
    let Some(glow) = current() else { return };
    let Some(original) = cache.light.original_of(drawn.id()) else { return };
    paint_glow_texture(painter, original.id(), dest, glow);
}

/// After a scenery prop's art: the fire's copy over it, as for a sprite.
pub fn paint_prop_glow(
    painter: &egui::Painter,
    cache: &crate::creature_art::CreatureArtCache,
    field: &gs_field::solver::CreatureField,
    prop: &gs_field::scenes::SceneProp,
    drawn: &egui::TextureHandle,
    dest: egui::Rect,
) {
    let Some(original) = cache.light.original_of(drawn.id()) else { return };
    let t = painter.ctx().input(|i| i.time) as f32;
    let glow = LIGHTS.with(|l| glow_at(&l.borrow(), field, prop.x, prop.z.max(0.4), t));
    if let Some(glow) = glow {
        paint_glow_texture(painter, original.id(), dest, glow);
    }
}

/// Screen pixels per world unit at a floor point, and the point on screen.
fn place(
    field: &gs_field::solver::CreatureField,
    to_screen: &dyn Fn(f32, f32) -> Pos2,
    scale: f32,
    x: f32,
    z: f32,
) -> (Pos2, f32) {
    let ((sx, sy), ppu) = field.project_ground(x, z);
    (to_screen(sx, sy), ppu * scale)
}

/// The warm pool a light casts: an ellipse lying on the floor, bright in
/// the middle, fading to nothing at its radius. Over a lit backdrop it is
/// the backdrop by day, fire-tinted (the fire lights the ground); without
/// one, the fire's colour laid on.
fn ground_pool(
    painter: &egui::Painter,
    field: &gs_field::solver::CreatureField,
    to_screen: &dyn Fn(f32, f32) -> Pos2,
    scale: f32,
    light: &PointLight,
    index: usize,
    t: f32,
    backdrop: Option<Backdrop>,
) {
    let f = flicker(light, index, t);
    let (lx, lz, _) = at(field, light, index, t);
    let rgb = hex(&light.color);
    let r = light.radius.max(0.05);
    let rings = 6;
    // A beam's wedge needs its edges drawn finer than a round pool.
    let around = if light.all_round() { 28 } else { 72 };
    let tint = rgb.map(|c| c as f32 + (255.0 - c as f32) * POOL_TINT_LIFT);
    let (mut mesh, strength) = match backdrop {
        Some(b) => (egui::Mesh::with_texture(b.texture), POOL_REVEAL * 255.0),
        None => (egui::Mesh::default(), 140.0),
    };
    let colour = |a: f32| match backdrop {
        Some(_) => Color32::from_rgba_unmultiplied(tint[0] as u8, tint[1] as u8, tint[2] as u8, a.clamp(0.0, 255.0) as u8),
        None => Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], a.clamp(0.0, 255.0) as u8),
    };
    // Stage point -> where it is on the backdrop's texture.
    let uv = |sx: f32, sy: f32| match backdrop {
        Some(b) => pos2((sx - b.stage[0]) / (b.stage[2] - b.stage[0]).max(1e-3), (sy - b.stage[1]) / (b.stage[3] - b.stage[1]).max(1e-3)),
        None => egui::epaint::WHITE_UV,
    };
    let vertex = |mesh: &mut egui::Mesh, x: f32, z: f32, a: f32| {
        let ((sx, sy), _) = field.project_ground(x, z);
        let (p, _) = place(field, to_screen, scale, x, z);
        mesh.vertices.push(egui::epaint::Vertex { pos: p, uv: uv(sx, sy), color: colour(a) });
    };
    vertex(&mut mesh, lx, lz, strength * light.intensity * f);
    for ring in 1..=rings {
        let u = ring as f32 / rings as f32;
        let a = strength * light.intensity * f * (1.0 - u * u).powi(2);
        for k in 0..around {
            let ang = k as f32 / around as f32 * std::f32::consts::TAU;
            // A circle ON THE FLOOR, through the floor's perspective.
            let wz = (lz + ang.sin() * r * u).max(0.45);
            let (_, ppu) = field.project_ground(lx, wz);
            let wx = lx + ang.cos() * r * u * ppu;
            vertex(&mut mesh, wx, wz, a * light.beam(ang.cos(), ang.sin()));
        }
    }
    for ring in 0..rings {
        let base = 1 + ring * around;
        for k in 0..around {
            let (a, b) = ((base + k) as u32, (base + (k + 1) % around) as u32);
            if ring == 0 {
                mesh.add_triangle(0, a, b);
            } else {
                let (pa, pb) = ((a as usize - around) as u32, (b as usize - around) as u32);
                mesh.add_triangle(pa, pb, b);
                mesh.add_triangle(pa, b, a);
            }
        }
    }
    // A backdrop's texture must not show past the stage's edges.
    let stage = egui::Rect::from_two_pos(to_screen(0.0, 0.0), to_screen(gs_field::solver::STAGE_W, gs_field::solver::STAGE_H));
    painter.with_clip_rect(painter.clip_rect().intersect(stage)).add(egui::Shape::mesh(mesh));
}

/// The glow in the air round a flame, `radius` pixels, ADDED (premultiplied
/// colours with alpha 0), strongest at the flame. With `beam`, thrown the
/// way that lamp's beam points (a quarter of it stays all round: the lamp
/// itself glows).
fn halo(painter: &egui::Painter, at: Pos2, radius: f32, rgb: [u8; 3], strength: f32, beam: Option<&PointLight>) {
    if radius < 1.0 || strength <= 0.0 {
        return;
    }
    let rings = 5;
    let around = 24;
    let colour = |u: f32, way: f32| {
        let k = strength * (1.0 - u).powi(2) * way;
        Color32::from_rgba_premultiplied((rgb[0] as f32 * k) as u8, (rgb[1] as f32 * k) as u8, (rgb[2] as f32 * k) as u8, 0)
    };
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(at, colour(0.0, 1.0));
    for ring in 1..=rings {
        let u = ring as f32 / rings as f32;
        for k in 0..around {
            let ang = k as f32 / around as f32 * std::f32::consts::TAU;
            // Screen up is deeper across the floor, foreshortened.
            let way = beam.map_or(1.0, |l| 0.25 + 0.75 * l.beam(ang.cos(), -ang.sin() / FORESHORTEN));
            mesh.colored_vertex(at + vec2(ang.cos(), ang.sin()) * radius * u, colour(u, way));
        }
    }
    for ring in 0..rings {
        let base = 1 + ring * around;
        for k in 0..around {
            let (a, b) = ((base + k) as u32, (base + (k + 1) % around) as u32);
            if ring == 0 {
                mesh.add_triangle(0, a, b);
            } else {
                let (pa, pb) = ((a as usize - around) as u32, (b as usize - around) as u32);
                mesh.add_triangle(pa, pb, b);
                mesh.add_triangle(pa, b, a);
            }
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// A flame's tongues in the light's colour, outside in: `(rgb, alpha,
/// size)`. The outer one is the colour deepened (each channel squared, so
/// its strongest channel leads), the middle the colour, the core it paled
/// toward white -- the default fire colour gives the old red / orange /
/// pale yellow.
fn flame_layers(rgb: [u8; 3]) -> [([u8; 3], u8, f32); 3] {
    let c = rgb.map(|v| v as f32 / 255.0);
    let byte = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    let outer = c.map(|v| byte(v * v * 0.82));
    let core = c.map(|v| byte(v + (1.0 - v) * 0.7));
    [(outer, 210, 1.0), (rgb, 235, 0.72), (core, 255, 0.44)]
}

/// A flame: layered tongues in its light's colour, deep outside to a pale
/// core, swaying.
/// Each tongue is round at the base, widest just above it, and tapers to a
/// tip that sways further than its root.
fn flame(painter: &egui::Painter, base: Pos2, h: f32, t: f32, seed: f32, f: f32, rgb: [u8; 3]) {
    let layers: [([u8; 4], f32); 3] = flame_layers(rgb).map(|(c, a, size)| ([c[0], c[1], c[2], a], size));
    let steps = 12;
    for (k, (c, size)) in layers.iter().enumerate() {
        let hh = h * size * (0.85 + 0.15 * f) * (1.0 + 0.08 * (t * 9.0 + seed + k as f32).sin());
        let w = hh * 0.36;
        let sway = (t * 5.0 + seed * 2.0 + k as f32).sin() * w * 0.35;
        let at = |s: f32, side: f32| {
            let half = w * (1.0 - s).powf(1.3) * (s * 6.0).min(1.0).sqrt();
            pos2(base.x + side * half + sway * s * s, base.y - s * hh)
        };
        let mut pts: Vec<Pos2> = (0..=steps).map(|i| at(i as f32 / steps as f32, -1.0)).collect();
        pts.extend((0..steps).rev().map(|i| at(i as f32 / steps as f32, 1.0)));
        painter.add(egui::Shape::convex_polygon(pts, Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]), egui::Stroke::NONE));
    }
}

pub(crate) fn paint_lamp(painter: &egui::Painter, field: &gs_field::solver::CreatureField, to_screen: &dyn Fn(f32, f32) -> Pos2, scale: f32, light: &PointLight, i: usize, t: f32) {
    if in_pass(i) && matches!(light.kind, LampKind::Torch | LampKind::Wisp) {
        return;
    }
    let (lx, lz, up) = at(field, light, i, t);
    let (foot, ppu) = place(field, to_screen, scale, lx, lz);
    let f = flicker(light, i, t);
    let flame_at = foot + vec2(0.0, -up * ppu);
    let rgb = hex(&light.color);
    // A lamp shining one way is no ball of light: its glow goes with the
    // beam and keeps closer.
    let (halo_r, halo_k) = if light.all_round() { (HALO_FLAMES, HALO) } else { (HALO_FLAMES * 0.7, HALO * 0.8) };
    halo(painter, flame_at, FLAME_H * ppu * halo_r, rgb, halo_k * light.intensity.clamp(0.0, 1.0) * f, (!light.all_round()).then_some(light));
    match light.kind {
        LampKind::Glow => {}
        LampKind::Lantern => {
            // A post with the lantern hung at its top: a dark frame round a
            // lit pane, a small steady flame inside.
            let top = foot + vec2(0.0, -LANTERN_POST_H * ppu);
            let frame = Color32::from_rgb(52, 42, 30);
            painter.line_segment([foot, top], egui::Stroke::new((ppu * 0.025).max(1.5), Color32::from_rgb(60, 48, 34)));
            let (w, h) = (FLAME_H * ppu * 0.8, FLAME_H * ppu * 1.1);
            let body = egui::Rect::from_center_size(top + vec2(0.0, -h * 0.5), vec2(w, h));
            let rounding = egui::CornerRadius::same((w * 0.12).clamp(0.0, 255.0) as u8);
            painter.rect_filled(body, rounding, Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], 200));
            painter.rect_stroke(body, rounding, egui::Stroke::new((ppu * 0.02).max(1.0), frame), egui::StrokeKind::Middle);
            painter.line_segment([body.center_top(), body.center_top() + vec2(0.0, -h * 0.25)], egui::Stroke::new((ppu * 0.02).max(1.0), frame));
            flame(painter, body.center_bottom() + vec2(0.0, -h * 0.12), FLAME_H * ppu * 0.55, t, i as f32 + 5.0, 0.9 + 0.1 * f, rgb);
        }
        LampKind::Wisp => {
            // A ball of light with a bright heart, pulsing; no flame.
            let r = FLAME_H * ppu;
            halo(painter, flame_at, r * 1.6, rgb, 0.9 * light.intensity.clamp(0.0, 1.0) * f, None);
            let core = rgb.map(|c| c + ((255 - c) as f32 * 0.6) as u8);
            painter.circle_filled(flame_at, r * 0.22 * (0.9 + 0.2 * f), Color32::from_rgba_unmultiplied(core[0], core[1], core[2], 235));
        }
        LampKind::Campfire => {
            // Logs, then the flame on them.
            let w = FLAME_H * ppu * 1.1;
            for (dx, tilt) in [(-0.2f32, 0.25f32), (0.2, -0.25)] {
                let c = foot + vec2(dx * w, -w * 0.06);
                let d = vec2(tilt.cos(), tilt.sin()) * w * 0.55;
                painter.line_segment([c - d, c + d], egui::Stroke::new((w * 0.16).max(1.5), Color32::from_rgb(70, 44, 28)));
            }
            flame(painter, foot + vec2(0.0, -w * 0.08), FLAME_H * ppu * 1.6, t, i as f32, f, hex(&light.color));
        }
        LampKind::Torch => {
            let top = foot + vec2(0.0, -TORCH_POLE_H * ppu * 2.2);
            painter.line_segment([foot, top], egui::Stroke::new((ppu * 0.025).max(1.5), Color32::from_rgb(82, 56, 34)));
            flame(painter, top, FLAME_H * ppu * 1.2, t, i as f32 + 3.0, f, hex(&light.color));
        }
    }
}

/// Before the cards: every light's ground pool. The lamps themselves wait
/// for [`paint_lamps_behind`] / [`paint_front`]. Returns true when there
/// are lights (the field keeps repainting for the flicker).
pub fn paint_back(
    painter: &egui::Painter,
    to_screen: &dyn Fn(f32, f32) -> Pos2,
    scale: f32,
    field: &gs_field::solver::CreatureField,
    scene: Option<&gs_field::scenes::StageScene>,
    backdrop: Option<Backdrop>,
    ctx: &egui::Context,
) -> bool {
    let lights = scene.map(|s| s.lights.as_slice()).unwrap_or_default();
    LIGHTS.with(|l| {
        let mut l = l.borrow_mut();
        if l.as_slice() != lights {
            *l = lights.to_vec();
        }
    });
    DRAWN.with(|d| {
        let mut d = d.borrow_mut();
        d.clear();
        d.resize(lights.len(), false);
    });
    if lights.is_empty() {
        return false;
    }
    let t = ctx.input(|i| i.time) as f32;
    for (i, light) in lights.iter().enumerate() {
        if !in_pass(i) {
            ground_pool(painter, field, to_screen, scale, light, i, t, backdrop);
        }
    }
    true
}

/// Just before a card or prop standing at depth `z`: the lamps not yet
/// drawn that stand at least as far away, so it covers them.
pub fn paint_lamps_behind(
    painter: &egui::Painter,
    to_screen: &dyn Fn(f32, f32) -> Pos2,
    scale: f32,
    field: &gs_field::solver::CreatureField,
    scene: Option<&gs_field::scenes::StageScene>,
    z: f32,
    ctx: &egui::Context,
) {
    let Some(scene) = scene.filter(|s| !s.lights.is_empty()) else { return };
    let t = ctx.input(|i| i.time) as f32;
    for k in lamps_due(&scene.lights, z) {
        paint_lamp(painter, field, to_screen, scale, &scene.lights[k], k, t);
    }
}

/// After the cards: the lamps nearer the viewer than everything.
pub fn paint_front(
    painter: &egui::Painter,
    to_screen: &dyn Fn(f32, f32) -> Pos2,
    scale: f32,
    field: &gs_field::solver::CreatureField,
    scene: Option<&gs_field::scenes::StageScene>,
    ctx: &egui::Context,
) {
    let Some(scene) = scene.filter(|s| !s.lights.is_empty()) else { return };
    paint_lamps_behind(painter, to_screen, scale, field, Some(scene), f32::NEG_INFINITY, ctx);
    ctx.request_repaint_after(std::time::Duration::from_millis(40));
}

/// The lamps not yet drawn this frame at depth >= `z`, far to near, marked
/// drawn.
pub(crate) fn lamps_due(lights: &[PointLight], z: f32) -> Vec<usize> {
    DRAWN.with(|d| {
        let mut d = d.borrow_mut();
        if d.len() != lights.len() {
            d.clear();
            d.resize(lights.len(), false);
        }
        let mut due: Vec<usize> = (0..lights.len()).filter(|&k| !d[k] && lights[k].z >= z).collect();
        due.sort_by(|&a, &b| lights[b].z.total_cmp(&lights[a].z));
        for &k in &due {
            d[k] = true;
        }
        due
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fire_reaches_near_creatures_on_the_side_facing_it() {
        let field = gs_field::solver::CreatureField::default();
        let light = PointLight::new(LampKind::Campfire, 440.0, 3.0);
        let (at, _) = reach(&field, &light, 0, 0.0, 440.0, 3.0);
        assert!((at - 1.0).abs() < 1e-4, "full strength at the fire");
        let (near, side) = reach(&field, &light, 0, 0.0, 400.0, 3.0);
        assert!(near > 0.0 && near < 1.0 && side > 0.0, "a creature left of the fire: lit, its right side more");
        let (far, _) = reach(&field, &light, 0, 0.0, 440.0, 3.0 + light.radius + 0.1);
        assert_eq!(far, 0.0, "nothing past the radius");
        let steady = PointLight { flicker: 0.0, ..light.clone() };
        assert_eq!(flicker(&steady, 0, 1.23), 1.0);
        assert!((0..50).map(|k| flicker(&light, 0, k as f32 * 0.07)).any(|f| f < 0.97), "a fire flickers");
    }

    #[test]
    fn a_puppet_sees_a_lamp_where_it_stands() {
        let field = gs_field::solver::CreatureField::default();
        let lights = vec![
            PointLight::new(LampKind::Campfire, 400.0, 3.5),
            PointLight::new(LampKind::Torch, 480.0, 2.5),
            PointLight::new(LampKind::Campfire, 440.0, 3.0 + 10.0),
        ];
        let near = near_lamps(&lights, &field, 440.0, 3.0, 0.0);
        assert_eq!(near.len(), 2, "the far fire reaches nothing here");
        let (fire, torch) = (near[0], near[1]);
        assert!(fire.across < 0.0 && fire.deeper > 0.0, "the fire is left of it and farther away: {fire:?}");
        assert!(torch.across > 0.0 && torch.deeper < 0.0, "the torch is right of it and nearer: {torch:?}");
        assert!(torch.up > fire.up, "a torch's flame is up on its pole");
        NEAR.with(|n| *n.borrow_mut() = near);
        let (lamps, flicker) = puppet_lamps(2.0);
        // In the skin's frame: +x screen-right, +z toward the viewer, feet.
        assert!(lamps[0].pos[0] < 0.0 && lamps[0].pos[2] < 0.0, "{:?}", lamps[0]);
        assert!(lamps[1].pos[0] > 0.0 && lamps[1].pos[2] > 0.0, "{:?}", lamps[1]);
        assert!((lamps[0].reach - fire.reach * 2.0).abs() < 1e-4, "reach in feet");
        assert!(flicker > 0.0 && flicker <= 1.0);
        NEAR.with(|n| n.borrow_mut().clear());
        assert!(puppet_lamps(2.0).0.is_empty());

        // A beam pointed away from it does not reach it; pointed at it, it
        // does, and hands its bearing over in the skin's frame.
        let mut aimed = PointLight::new(LampKind::Glow, 400.0, 3.0);
        aimed.spread = 60.0;
        aimed.aim = 180.0; // screen-left, away from the creature at 440
        assert!(near_lamps(std::slice::from_ref(&aimed), &field, 440.0, 3.0, 0.0).is_empty());
        aimed.aim = 0.0;
        let near = near_lamps(std::slice::from_ref(&aimed), &field, 440.0, 3.0, 0.0);
        assert_eq!(near.len(), 1);
        NEAR.with(|n| *n.borrow_mut() = near);
        let beam = puppet_lamps(2.0).0[0].beam.expect("a beam");
        assert!(beam.dir[0] > 0.99 && beam.spread == 60.0, "{beam:?}");
        NEAR.with(|n| n.borrow_mut().clear());
    }

    /// A wisp is somewhere else a moment later, up in the air; a torch is
    /// where it was planted.
    #[test]
    fn a_wisp_wanders_and_a_torch_stays_put() {
        let field = gs_field::solver::CreatureField::default();
        let torch = PointLight::new(LampKind::Torch, 440.0, 3.0);
        assert_eq!(at(&field, &torch, 0, 0.0), at(&field, &torch, 0, 5.0));
        let wisp = PointLight::new(LampKind::Wisp, 440.0, 3.0);
        let (x0, z0, up0) = at(&field, &wisp, 0, 0.0);
        let (x1, z1, _) = at(&field, &wisp, 0, 2.5);
        assert!((x1 - x0).abs() > 0.5 || (z1 - z0).abs() > 0.02, "it moved: {x0},{z0} -> {x1},{z1}");
        assert!((up0 - WISP_H).abs() <= gs_field::lighting::WISP_BOB + 1e-6, "afloat at {up0}");
        let near = near_lamps(std::slice::from_ref(&wisp), &field, 440.0, 3.0, 0.0);
        assert_eq!(near.len(), 1);
        assert!(near[0].up > flame_height(LampKind::Torch), "a wisp lights from above a torch's flame");
    }

    #[test]
    fn a_flame_burns_in_its_light_s_colour() {
        // The default fire keeps the old red / orange / pale yellow.
        let [outer, mid, core] = flame_layers(hex(&fire_colour_for_test()));
        assert!(outer.0[0] > 180 && outer.0[1] < 100 && outer.0[2] < 40, "{outer:?}");
        assert_eq!(mid.0, [255, 160, 74]);
        assert!(core.0.iter().all(|c| *c > 190), "{core:?}");
        // A blue light burns blue: its strongest channel leads every tongue.
        for (c, _, _) in flame_layers([74, 140, 255]) {
            assert!(c[2] >= c[0] && c[2] >= c[1], "{c:?}");
        }
    }

    fn fire_colour_for_test() -> String {
        PointLight::new(LampKind::Campfire, 0.0, 1.0).color
    }

    #[test]
    fn lamps_slot_between_the_cards_by_depth() {
        let lights = vec![
            PointLight::new(LampKind::Campfire, 400.0, 3.0),
            PointLight::new(LampKind::Torch, 300.0, 5.0),
            PointLight::new(LampKind::Torch, 500.0, 1.0),
        ];
        DRAWN.with(|d| d.borrow_mut().clear());
        // Cards at z 4 then 2 (far to near), then the front pass.
        assert_eq!(lamps_due(&lights, 4.0), vec![1], "the far torch goes behind the first card");
        assert_eq!(lamps_due(&lights, 2.0), vec![0], "the fire between the two cards");
        assert_eq!(lamps_due(&lights, 2.0), Vec::<usize>::new(), "each lamp once");
        assert_eq!(lamps_due(&lights, f32::NEG_INFINITY), vec![2], "the near torch in front of all");
    }
}
