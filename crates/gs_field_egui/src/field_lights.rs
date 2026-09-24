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
fn reach(field: &gs_field::solver::CreatureField, light: &PointLight, x: f32, z: f32) -> (f32, f32) {
    let (_, ppu_l) = field.project_ground(light.x, light.z);
    let (_, ppu_c) = field.project_ground(x, z);
    let ppu = ((ppu_l + ppu_c) * 0.5).max(1e-3);
    let (dx, dz) = ((light.x - x) / ppu, light.z - z);
    let d = dx.hypot(dz);
    let r = light.radius.max(0.05);
    let fall = (1.0 - (d / r).powi(2)).max(0.0).powi(2);
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
    let glow = scene.and_then(|s| glow_at(&s.lights, field, fx, field.ground_z(unit), now_ms as f32 / 1000.0));
    CURRENT.with(|g| *g.borrow_mut() = glow);
}

/// The fire glow on something standing at floor point (x, z) at time `t`.
fn glow_at(lights: &[PointLight], field: &gs_field::solver::CreatureField, x: f32, z: f32, t: f32) -> Option<FireGlow> {
    let (mut left, mut right, mut rgb, mut best) = (0.0f32, 0.0f32, [255u8; 3], 0.0f32);
    for (i, light) in lights.iter().enumerate() {
        let (c, side) = reach(field, light, x, z);
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
/// the middle, fading to nothing at its radius.
fn ground_pool(painter: &egui::Painter, field: &gs_field::solver::CreatureField, to_screen: &dyn Fn(f32, f32) -> Pos2, scale: f32, light: &PointLight, f: f32) {
    let rgb = hex(&light.color);
    let (centre, _) = place(field, to_screen, scale, light.x, light.z);
    let r = light.radius.max(0.05);
    let rings = 6;
    let around = 28;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(centre, Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], (140.0 * light.intensity * f) as u8));
    for ring in 1..=rings {
        let u = ring as f32 / rings as f32;
        let a = 140.0 * light.intensity * f * (1.0 - u * u).powi(2);
        let c = Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], a as u8);
        for k in 0..around {
            let ang = k as f32 / around as f32 * std::f32::consts::TAU;
            // A circle ON THE FLOOR, through the floor's perspective.
            let wz = (light.z + ang.sin() * r * u).max(0.45);
            let (_, ppu) = field.project_ground(light.x, wz);
            let wx = light.x + ang.cos() * r * u * ppu;
            let (p, _) = place(field, to_screen, scale, wx, wz);
            mesh.colored_vertex(p, c);
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

/// A flame: layered tongues, dark red outside to a pale core, swaying.
/// Each tongue is round at the base, widest just above it, and tapers to a
/// tip that sways further than its root.
fn flame(painter: &egui::Painter, base: Pos2, h: f32, t: f32, seed: f32, f: f32) {
    let layers: [([u8; 4], f32); 3] = [([210, 70, 22, 210], 1.0), ([255, 150, 40, 235], 0.72), ([255, 236, 170, 255], 0.44)];
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
    let (foot, ppu) = place(field, to_screen, scale, light.x, light.z);
    let f = flicker(light, i, t);
    match light.kind {
        LampKind::Glow => {}
        LampKind::Campfire => {
            // Logs, then the flame on them.
            let w = FLAME_H * ppu * 1.1;
            for (dx, tilt) in [(-0.2f32, 0.25f32), (0.2, -0.25)] {
                let c = foot + vec2(dx * w, -w * 0.06);
                let d = vec2(tilt.cos(), tilt.sin()) * w * 0.55;
                painter.line_segment([c - d, c + d], egui::Stroke::new((w * 0.16).max(1.5), Color32::from_rgb(70, 44, 28)));
            }
            flame(painter, foot + vec2(0.0, -w * 0.08), FLAME_H * ppu * 1.6, t, i as f32, f);
        }
        LampKind::Torch => {
            let top = foot + vec2(0.0, -TORCH_POLE_H * ppu * 2.2);
            painter.line_segment([foot, top], egui::Stroke::new((ppu * 0.025).max(1.5), Color32::from_rgb(82, 56, 34)));
            flame(painter, top, FLAME_H * ppu * 1.2, t, i as f32 + 3.0, f);
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
        ground_pool(painter, field, to_screen, scale, light, flicker(light, i, t));
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
        let (at, _) = reach(&field, &light, 440.0, 3.0);
        assert!((at - 1.0).abs() < 1e-4, "full strength at the fire");
        let (near, side) = reach(&field, &light, 400.0, 3.0);
        assert!(near > 0.0 && near < 1.0 && side > 0.0, "a creature left of the fire: lit, its right side more");
        let (far, _) = reach(&field, &light, 440.0, 3.0 + light.radius + 0.1);
        assert_eq!(far, 0.0, "nothing past the radius");
        let steady = PointLight { flicker: 0.0, ..light.clone() };
        assert_eq!(flicker(&steady, 0, 1.23), 1.0);
        assert!((0..50).map(|k| flicker(&light, 0, k as f32 * 0.07)).any(|f| f < 0.97), "a fire flickers");
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
