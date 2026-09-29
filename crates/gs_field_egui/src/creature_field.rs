//! The creature field: the room's hostiles as cards on a perspective
//! floor, placed by `gs_field::solver`. P3 renders generated
//! placeholder cards (vector standees); sprite art replaces them when the
//! `[creature_card]` resolve cascade finds files (P4/P5).
//!
//! The solver plans on a fixed 880x470 virtual stage; this renderer maps
//! that stage into the widget rect uniformly (fit, centred), which is what
//! keeps every solver guarantee intact under any window size.
//!
//! Ported from VellumFE `src/frontend/gui/app/widgets/creature_field.rs`
//! @60c84a79, where these were associated fns of `VellumGuiApp` (no `self`).

use gs_field::scenes::StageScene;
use gs_field::Condition;
use gs_field::solver::{CreatureField, ScreenRect, Unit, STAGE_H, STAGE_W};
use gs_field::{Creature, CreatureFlags};
use crate::creature_art::SharedCreatureArt;
use egui::{Color32, Stroke};

/// Everything the creature-field renderer reads, borrowed. The renderer
/// takes this instead of the whole `AppCore` so its real inputs are
/// visible (and so hosts other than the game — the Studio Stage — can
/// feed it without fabricating application state). VellumFE builds one
/// with its own `from_app_core` glue (creature_field_host.rs, not here).
pub struct CreatureFieldView<'a> {
    /// The solved field: units, draw order, projection.
    pub field: &'a CreatureField,
    /// The room's creatures, looked up by unit member id.
    pub creatures: &'a [Creature],
    /// Raw current-target id (normalized by the renderer).
    pub current_target: &'a str,
    /// Does this skin "show when…" condition hold for this creature? The
    /// host owns the game state the answer comes from; the renderer only
    /// asks (via `gs_field::cards::resolve_card_with`).
    pub eval_condition: Box<dyn Fn(&Condition, &CreatureFlags) -> bool + 'a>,
    /// Derived-effect severity for (creature exist id, effect name).
    pub effect_severity: Box<dyn Fn(&str, &str) -> Option<u8> + 'a>,
    /// Paint the perspective floor grid.
    pub show_grid: bool,
    /// Paint the target-order strip.
    pub show_order: bool,
    /// Creature-card art cache; None = placeholder standees.
    pub creature_art: Option<&'a SharedCreatureArt>,
    /// Stage scene (backdrop + props); None everywhere in the game today.
    pub scene: Option<&'a StageScene>,
}

/// A click on a creature's card. What it MEANS is the host's business:
/// the game sends a target command, the Studio Stage sets its own target.
#[derive(Debug, Clone, PartialEq)]
pub struct CreatureFieldClick {
    /// The clicked creature's exist id, normalized (no leading '#').
    pub exist_id: String,
    /// Where the pointer was, in egui screen points. Only primary-button
    /// clicks are reported.
    pub pointer_pos: egui::Pos2,
}

/// Exist ids arrive with or without a leading '#'; compare them bare.
fn bare_id(id: &str) -> String {
    id.trim().trim_start_matches('#').to_string()
}

/// Virtual-stage -> widget-rect mapping.
struct StageMap {
    scale: f32,
    origin: egui::Pos2,
}

impl StageMap {
    fn fit(rect: egui::Rect) -> Self {
        let scale = (rect.width() / STAGE_W).min(rect.height() / STAGE_H);
        let w = STAGE_W * scale;
        let h = STAGE_H * scale;
        let origin = egui::pos2(
            rect.left() + (rect.width() - w) / 2.0,
            rect.top() + (rect.height() - h) / 2.0,
        );
        Self { scale, origin }
    }

    fn pt(&self, x: f32, y: f32) -> egui::Pos2 {
        egui::pos2(
            self.origin.x + x * self.scale,
            self.origin.y + y * self.scale,
        )
    }

    fn rect(&self, r: &ScreenRect) -> egui::Rect {
        egui::Rect::from_min_max(self.pt(r.x0, r.y0), self.pt(r.x1, r.y1))
    }

    /// Widget-space point back to virtual-stage coordinates (the inverse
    /// of `pt`), for editors dragging things on the stage.
    fn stage_pos(&self, p: egui::Pos2) -> (f32, f32) {
        (
            (p.x - self.origin.x) / self.scale,
            (p.y - self.origin.y) / self.scale,
        )
    }

    /// The logical scene viewport in widget space: where the full 880x470
    /// stage lands after the fit. Everything composed on the stage —
    /// backdrop, ground, props, creatures, overlays — lives inside this
    /// rect; outside it is neutral fill.
    fn stage_rect(&self) -> egui::Rect {
        egui::Rect::from_min_max(self.pt(0.0, 0.0), self.pt(STAGE_W, STAGE_H))
    }
}

/// Where a backdrop texture sits WITHIN the logical 880x470 stage, in
/// stage coordinates: cover-fit to the stage (not the window), centred.
/// This placement depends only on the texture's aspect ratio, so a given
/// backdrop pixel always maps to the same stage point regardless of the
/// widget size — the single transform (`StageMap`) then carries stage
/// points to the screen for backdrop and objects alike. A stage-aspect
/// backdrop fills the stage exactly; other aspects overflow symmetrically
/// and are clipped at the stage edge.
fn backdrop_stage_rect(tex_w: f32, tex_h: f32) -> Option<(f32, f32, f32, f32)> {
    if tex_w <= 0.0 || tex_h <= 0.0 {
        return None;
    }
    let cover = (STAGE_W / tex_w).max(STAGE_H / tex_h);
    let w = tex_w * cover;
    let h = tex_h * cover;
    let x0 = (STAGE_W - w) / 2.0;
    let y0 = (STAGE_H - h) / 2.0;
    Some((x0, y0, x0 + w, y0 + h))
}

/// A scene's "#rrggbb" background color, or None for anything malformed
/// (a bad hand-edited scene degrades to the default fill, never errors).
fn parse_hex_rgb(text: &str) -> Option<Color32> {
    let hex = text.trim().strip_prefix('#').unwrap_or(text.trim());
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color32::from_rgb(r, g, b))
}

/// Stable placeholder body color from the creature's noun, muted so status
/// glyphs and the reticule stay the loud elements.
fn body_color(noun: &str, dead: bool) -> Color32 {
    if dead {
        return Color32::from_gray(85);
    }
    let mut h: u32 = 2166136261;
    for b in noun.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    // Hue from the hash, fixed low saturation / mid value.
    let hue = (h % 360) as f32 / 360.0;
    let (r, g, b) = hsv_muted(hue);
    Color32::from_rgb(r, g, b)
}

fn hsv_muted(hue: f32) -> (u8, u8, u8) {
    let (s, v) = (0.35f32, 0.55f32);
    let i = (hue * 6.0).floor();
    let f = hue * 6.0 - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    let (r, g, b) = match (i as i32) % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    ((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

// Studio's Stage drives this exact production path.
pub fn render_creature_field_content(
    view: &CreatureFieldView<'_>,
    ui: &mut egui::Ui,
) -> Option<CreatureFieldClick> {
    let (show_grid, show_order) = (view.show_grid, view.show_order);

    let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click());
    let painter = ui.painter_at(rect);
    let map = StageMap::fit(rect);
    let field = view.field;

    // Skin creature art (and scene art), prepared in the update loop;
    // render only reads.
    let art_cache = view
        .creature_art
        .map(|a| a.lock().expect("creature art lock"));
    let art_cache = art_cache.as_deref();

    // Scene backdrop under everything: the color fill covers the whole
    // widget (it doubles as the neutral fill outside the stage), then
    // the background image is placed within the LOGICAL STAGE
    // (cover-fit to 880x470, a window-independent placement) and drawn
    // through the same StageMap as ground/props/creatures, clipped to
    // the stage rect. One transform for the painted scene and the
    // objects on it — resizing the window can no longer make them
    // drift apart.
    let scene = view.scene;
// >>>doll
    // The v2 lighting pass (field_gpu_stage.rs), when a host turned it on:
    // the backdrop and every creature body become GPU pieces lit by the
    // scene's lamps. None = the old drawing, untouched.
    let backdrop_tex = scene
        .and_then(|s| s.background.as_deref())
        .and_then(|bg| art_cache.and_then(|c| c.scene_background(bg)));
    let backdrop_rect = backdrop_tex
        .and_then(|t| backdrop_stage_rect(t.size_vec2().x, t.size_vec2().y))
        .map(|(x0, y0, x1, y1)| [x0, y0, x1 - x0, y1 - y0]);
    let gpu = crate::field_gpu_stage::StageGpu::begin(ui.ctx(), field, scene, art_cache, backdrop_tex, backdrop_rect, map.origin, map.scale);
    crate::field_lights::set_gpu(gpu.as_ref().map(|g| g.lamps_in_pass().to_vec()));
    // The pass moves on the clock too -- the moons' drifting cloud, the
    // stars -- where no lamp or weather is there to ask for frames: without
    // this a moonlit scene stood still until the pointer moved.
    if gpu.is_some() && art_cache.is_some_and(|c| !c.light.moons_now().is_empty() || c.light.stars_now().is_some()) {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    }
    crate::field_gpu_stage::set_current(gpu);
// <<<doll
    if let Some(scene) = scene {
        if let Some(color) = scene.background_color.as_deref().and_then(parse_hex_rgb) {
            painter.rect_filled(rect, 0.0, color);
        }
        if let Some(texture) = scene
            .background
            .as_deref()
            .and_then(|bg| art_cache.and_then(|c| c.scene_background(bg)))
        {
            let ts = texture.size_vec2();
            if let Some((x0, y0, x1, y1)) = backdrop_stage_rect(ts.x, ts.y) {
                let stage_painter = painter.with_clip_rect(map.stage_rect());
                stage_painter.image(
                    texture.id(),
                    egui::Rect::from_min_max(map.pt(x0, y0), map.pt(x1, y1)),
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
// >>>doll
                // The v2 pass's backdrop, lit by the lamps, over the image.
                crate::field_gpu_stage::with_current(|g| g.paint_backdrop(&stage_painter));
// <<<doll
            }
        }
    }
// >>>doll
    // Weather behind the creatures: the far half of the rain / snow / fog.
    let weather_on = crate::field_weather::paint_back(&painter, map.stage_rect(), ui.ctx());
    // Point lights: their warm pools on the ground, and the lamps behind
    // the nearest creature. A pool shows the backdrop as it is by day
    // (the lit twin's original), fire-tinted: the fire lights the ground.
    let unlit_backdrop = scene.and_then(|s| s.background.as_deref()).and_then(|bg| {
        let cache = art_cache?;
        let lit = cache.scene_background(bg)?;
        let ts = lit.size_vec2();
        let (x0, y0, x1, y1) = backdrop_stage_rect(ts.x, ts.y)?;
        let unlit = cache.light.original_of(lit.id())?;
        Some(crate::field_lights::Backdrop { texture: unlit.id(), stage: [x0, y0, x1, y1] })
    });
    crate::field_lights::paint_back(&painter, &|x, y| map.pt(x, y), map.scale, field, scene, unlit_backdrop, ui.ctx());
    // The painted things standing in front of creatures (the backdrop's
    // occluder mask), placed by where each meets the floor.
    let occluders = crate::field_occluders::Prepared::new(ui.ctx(), scene, art_cache, &backdrop_stage_rect, field);
// <<<doll
    let props: &[gs_field::scenes::SceneProp] =
        scene.map(|s| s.props.as_slice()).unwrap_or_default();

    // A scene keeps painting with nobody home; the game (scene = None)
    // keeps its placeholder text.
    if field.units().is_empty() && scene.is_none() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "No hostiles in the room",
            egui::FontId::proportional(13.0),
            ui.visuals().weak_text_color(),
        );
        return None;
    }

    if show_grid {
        paint_field_floor(&painter, &map, field);
    }

    let current_target = bare_id(view.current_target);
    let now_ms = ui.input(|i| i.time) * 1000.0;
    let mut any_animated = false;

    // Far -> near (painter's algorithm), ground-z keyed in the solver.
    // Scenery props carry their own z and slot into the same sequence,
    // so a rock can stand in front of one creature and behind another.
    enum DrawItem {
        Unit(usize),
        Prop(usize),
    }
    let mut items: Vec<(f32, DrawItem)> = field
        .draw_order()
        .iter()
        .map(|&i| (field.ground_z(&field.units()[i]), DrawItem::Unit(i)))
        .collect();
    for (k, prop) in props.iter().enumerate() {
        items.push((prop.z.max(0.4), DrawItem::Prop(k)));
    }
    // Stable: units keep the solver's order on equal depth.
    items.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, item) in &items {
// >>>doll
        // Lamps and painted occluders farther away than this item stand
        // behind it.
        let item_z = match item {
            DrawItem::Unit(i) => field.ground_z(&field.units()[*i]),
            DrawItem::Prop(k) => props[*k].z.max(0.4),
        };
        crate::field_occluders::paint_behind(&painter, &|x, y| map.pt(x, y), map.scale, occluders.as_ref(), field, scene, item_z, ui.ctx());
// <<<doll
        let i = match item {
            DrawItem::Prop(k) => {
                paint_scene_prop(&painter, &map, field, &props[*k], art_cache);
                continue;
            }
            DrawItem::Unit(i) => *i,
        };
        let unit = &field.units()[i];
        for member in &unit.members {
            let Some(creature) = view.creatures.iter().find(|c| &c.id == member) else {
                continue;
            };
            let is_target = !current_target.is_empty()
                && bare_id(member) == current_target;
            any_animated |= paint_creature_card(
                &painter,
                &map,
                field,
                unit,
                creature,
                is_target,
                now_ms,
                ui.visuals().dark_mode,
                art_cache,
                view,
            );
        }
    }

// >>>doll
    // Occluders and lamps nearer than every card, then the near half of the
    // weather, in front of them all.
    crate::field_occluders::paint_behind(&painter, &|x, y| map.pt(x, y), map.scale, occluders.as_ref(), field, view.scene, f32::NEG_INFINITY, ui.ctx());
    if weather_on {
        crate::field_weather::paint_front(&painter, map.stage_rect(), ui.ctx());
    }
    crate::field_lights::paint_front(&painter, &|x, y| map.pt(x, y), map.scale, field, view.scene, ui.ctx());
// <<<doll
    if show_order {
        paint_target_order(&painter, &map, field, &current_target);
    }

    // The stun swirl is the only motion; idle rooms request no frames.
    if any_animated {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
    }

    // Click-to-target: nearest card under the pointer wins.
    if response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let mut order = field.draw_order();
            order.reverse(); // nearest first
            for &i in &order {
                let unit = &field.units()[i];
                let card = map.rect(&lifted_rect(view.creatures, field, unit));
                if card.contains(pos) {
                    // A mounted pair: pick the rider half above the
                    // saddle line, the mount below.
                    let member = if unit.members.len() > 1
                        && pos.y < card.top() + card.height() * 0.45
                    {
                        &unit.members[1]
                    } else {
                        &unit.members[0]
                    };
                    return Some(CreatureFieldClick {
                        exist_id: bare_id(member),
                        pointer_pos: pos,
                    });
                }
            }
        }
    }
    None
}

/// The card rect with any airborne lift applied (screen-space, after
/// projection — the floor footprint does not move).
fn lifted_rect(
    creatures: &[Creature],
    field: &CreatureField,
    unit: &Unit,
) -> ScreenRect {
    let mut r = field.rect(unit);
    if let Some(lift) = unit_lift(creatures, unit) {
        let dy = lift * (r.y1 - r.y0);
        r.y0 += dy;
        r.y1 += dy;
    }
    r
}

/// Airborne lift fraction for a unit (negative = up), from the lead
/// member's flags. Manifest-driven LiftSpec values take over once
/// creature-card skins are authored; these are the built-in defaults.
fn unit_lift(creatures: &[Creature], unit: &Unit) -> Option<f32> {
    let creature = creatures
        .iter()
        .find(|c| unit.members.first() == Some(&c.id))?;
    let flags = creature.flags.as_ref()?;
    if flags.has_flag("flying") {
        Some(-0.22)
    } else if flags.has_flag("hovering") {
        Some(-0.12)
    } else {
        None
    }
}

/// Widget-space point back to virtual-stage coordinates for a field
/// drawn into `rect` — the Studio's drag-to-place inverts through this
/// so it can never drift from the renderer's own stage mapping.
pub fn creature_field_stage_pos(
    rect: egui::Rect,
    pos: egui::Pos2,
) -> (f32, f32) {
    StageMap::fit(rect).stage_pos(pos)
}

/// One scenery prop, painted exactly like a creature card's base:
/// feet-anchored at the ground projection of (x, z), world height from
/// the sidecar (or the 1.0 default) times the prop's scale, through
/// the same perspective scaling the cards use, with the sidecar
/// footprint (or generic ellipse) as its contact shadow. Missing art
/// draws a muted placeholder block — a placed prop is never invisible.
fn paint_scene_prop(
    painter: &egui::Painter,
    map: &StageMap,
    field: &gs_field::solver::CreatureField,
    prop: &gs_field::scenes::SceneProp,
    art_cache: Option<&crate::creature_art::CreatureArtCache>,
) {
// >>>doll
    // Scenery is part of the painting: clipped to the stage like the
    // backdrop, so a tall prop (an arch over the road) is cut at the
    // painting's edge instead of drawing over the panel round it.
    let clipped = painter.with_clip_rect(map.stage_rect());
    let painter = &clipped;
// <<<doll
    let ((fx, fy), px_per_unit) = field.project_ground(prop.x, prop.z);
    let foot = map.pt(fx, fy);
    let art = art_cache.and_then(|c| c.scenery(&prop.image));
    let world_h = art.and_then(|a| a.size).unwrap_or(1.0) * prop.scale.max(0.01);
    let draw_h = world_h * px_per_unit * map.scale;
    let Some(art) = art else {
        let w = draw_h * 0.8;
        let body = egui::Rect::from_min_max(
            egui::pos2(foot.x - w / 2.0, foot.y - draw_h),
            egui::pos2(foot.x + w / 2.0, foot.y),
        );
        painter.rect_filled(body, w * 0.15, Color32::from_rgba_unmultiplied(120, 120, 120, 90));
        return;
    };
    let ts = art.texture.size_vec2();
    let draw_w = draw_h * ts.x / ts.y.max(1.0);
    let dest = egui::Rect::from_min_size(
        egui::pos2(foot.x - art.feet[0] * draw_w, foot.y - art.feet[1] * draw_h),
        egui::vec2(draw_w, draw_h),
    );
// >>>doll
    // The v2 pass lights the prop itself, as it does a creature: lamps and
    // the moon on it from where they are, the shadows it casts and takes.
    // Else (pass off, frame full, pixels not kept yet) the old way below.
    let gpu_drawn = art_cache
        .and_then(|cache| crate::field_gpu_stage::with_current(|g| g.paint_sprite(painter, cache, &art.texture, art, dest, prop.z.max(0.4), false)))
        .unwrap_or(false);
    if !gpu_drawn {
// <<<doll
    // Contact shadow only when the prop opts in (scene `shadow = true`
    // — scenery defaults shadowless, most of it paints its own
    // grounding). Sized by the sidecar footprint when authored, the
    // generic standee ellipse otherwise.
    if prop.shadow {
        let (shadow_c, shadow_rx, shadow_ry) = match art.footprint {
            Some(fp) => {
                let cx = fp
                    .center
                    .map(|c| dest.left() + c[0] * dest.width())
                    .unwrap_or(foot.x);
                (
                    egui::pos2(cx, foot.y),
                    dest.width() * fp.rx,
                    dest.width() * fp.effective_ry(),
                )
            }
            None => {
                let w = dest.width() * 0.55;
                (foot, w, w * 0.24)
            }
        };
        painter.add(egui::epaint::PathShape::convex_polygon(
            ellipse_points(shadow_c, shadow_rx, shadow_ry),
            Color32::from_black_alpha(60),
            Stroke::NONE,
        ));
    }
    painter.image(
        art.texture.id(),
        dest,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
// >>>doll
    // Firelight on the prop, as on a creature.
    if let Some(cache) = art_cache {
        crate::field_lights::paint_prop_glow(painter, cache, field, prop, &art.texture, dest);
    }
    }
// <<<doll
}

fn paint_field_floor(
    painter: &egui::Painter,
    map: &StageMap,
    field: &gs_field::solver::CreatureField,
) {
    let grid = Stroke::new(1.0, Color32::from_white_alpha(14));
    let edge = Stroke::new(1.4, Color32::from_white_alpha(26));
    let cols = field.columns();
    let rows = field.params.rows;
// >>>doll
    // Deep growth (solver::Growth) adds rows at the back and widens the
    // floor with depth: the floor's own rows and columns, not the port's.
    let _ = (cols, rows);
    let cols = field.floor_columns();
    let rows = field.rows();
// <<<doll
    // Row lines (near/far edges heavier).
    for r in 0..=rows {
        let (a, b) = field.floor_row_line(r);
        let stroke = if r == 0 || r == rows { edge } else { grid };
        painter.line_segment([map.pt(a.0, a.1), map.pt(b.0, b.1)], stroke);
    }
    // Column lines.
    for k in 0..=cols.len() {
        let (a, b) = field.floor_col_line(k);
        let stroke = if k == 0 || k == cols.len() {
            edge
        } else {
            grid
        };
        painter.line_segment([map.pt(a.0, a.1), map.pt(b.0, b.1)], stroke);
    }
}

/// One creature's card: skin sprite art with manifest-driven overlays
/// when the resolve cascade found a base, the generated placeholder
/// standee otherwise. Returns true when it painted a moving effect
/// (caller schedules a repaint).
#[allow(clippy::too_many_arguments)]
fn paint_creature_card(
    painter: &egui::Painter,
    map: &StageMap,
    field: &gs_field::solver::CreatureField,
    unit: &Unit,
    creature: &gs_field::Creature,
    is_target: bool,
    now_ms: f64,
    dark: bool,
    art_cache: Option<&crate::creature_art::CreatureArtCache>,
    view: &CreatureFieldView<'_>,
) -> bool {
    let flags = creature.flags.as_ref();
    let dead = creature.is_dead();
    let noun = creature.noun.as_deref().unwrap_or("creature");

    // Skin path: base art + the manifest's resolved card (variant,
    // lift, overlays), all evaluated host-style through resolve_card.
    // Art keys on the NAME token (boon-stripped slug — matches the
    // tiered art folders), same normalization the prepare pass used.
    let token = gs_field::naming::name_token(&creature.name);
    let art = art_cache.and_then(|c| c.base(&token));
    let resolved = match (art_cache, flags) {
        (Some(cache), Some(flags)) => Some(gs_field::cards::resolve_card_with(
            &cache.card,
            flags,
            &*view.eval_condition,
        )),
        _ => None,
    };

    // Lift: the manifest's airborne variant wins; built-in defaults
    // cover skinless rooms.
    let lift = resolved
        .as_ref()
        .and_then(|r| r.lift())
        .map(|l| l.offset_y)
        .or_else(|| {
            flags
                .filter(|f| f.has_flag("flying"))
                .map(|_| -0.22f32)
                .or_else(|| flags.filter(|f| f.has_flag("hovering")).map(|_| -0.12f32))
        });

    let base = field.rect(unit);
    let (foot_x, foot_y) = field.foot(unit);
    let mut r = base;
    if let Some(l) = lift {
        let dy = l * (r.y1 - r.y0);
        r.y0 += dy;
        r.y1 += dy;
    }
    let card = map.rect(&r);

    // Active art: a matched variant with placeholder-free authored pose
    // art replaces the cascade's base wholesale — texture AND grounding
    // metadata (its own anchors, bbox, footprint), so a prone image
    // grounds by its own contact point instead of inheriting the
    // standing base's. Template paths keep the ground pose.
    // Tier pose art: the locked tier's own {token}_prone image swaps
    // in while the creature is prone, unless an authored variant
    // already handles the pose.
    let prone_art = flags
        .filter(|f| f.has_flag("prone"))
        .and(art)
        .and_then(|a| a.extra("prone"))
        .and_then(|p| art_cache.and_then(|c| c.variant_base(p.to_string_lossy().as_ref())));
    let active_art = resolved
        .as_ref()
        .and_then(|r| r.base_override())
        .filter(|p| !p.contains('{'))
        .and_then(|p| art_cache.and_then(|c| c.variant_base(p)))
        .or(prone_art)
        .or(art);
// >>>doll
    // A host may ask for the puppet over the sprite (creature_puppet.rs).
    let active_art = active_art
        .filter(|_| !crate::creature_puppet::is_forced_puppet(painter.ctx(), &creature.id));
    // Lighting: the layers drawn next are this creature's (its frame for
    // shared overlays, its corpse grey for every layer).
    crate::creature_light::set_wearer(&token, dead);
    // ...and how much firelight reaches it.
    crate::field_lights::set_creature(view.scene, field, unit, now_ms);
// <<<doll

    // Sprite geometry before the shadow (the shadow needs the drawn
    // footprint). Aspect-fit into the card rect, then hang the image so
    // its feet anchor lands on the foot point: manifest calibration
    // wins, then the image's own (sidecar or alpha-derived) feet.
    let sprite = active_art.map(|a| {
        let feet = resolved
            .as_ref()
            .and_then(|r| r.authored_anchor("feet"))
            .unwrap_or(a.feet);
        let ts = a.texture.size_vec2();
        // Scale by art CONTENT (alpha bbox), not the raw texture:
        // transparent padding must not shrink the creature, and wide art
        // (quadrupeds, prone poses) must not be width-crushed by the
        // narrow card box.
        let px_per_unit = card.height() / unit.size.h.max(0.01);
        let scale = if let Some(s) = a.size.filter(|s| *s > 0.0) {
            // Authored world size (sidecar) wins — but displayed
            // through the creature's readability factor, which is
            // already folded into the resolved box heights (geometry
            // precedence 1). Target the box for the pose this art
            // depicts instead of raw `s`: a readability-clamped
            // creature otherwise draws its prone pose unclamped and
            // towers over its own standing pose (the giant prone
            // golem). Standing art keeps the standing box even while
            // the unit lies (no prone art shipped): lying must not
            // squash an upright sprite.
            let pose_art = art.is_some_and(|base| !std::ptr::eq(base, a));
            let target_h = if pose_art {
                unit.size.h
            } else {
                unit.standing.h
            };
            let content_h = ((a.bbox[3] - a.bbox[1]) * ts.y).max(1.0);
            let sc = target_h * px_per_unit / content_h;
            tracing::debug!(
                "[field-scale] {} authored_size={s:.2} target_h={target_h:.2} content_h={content_h:.0} scale={sc:.4}",
                creature.name,
            );
            sc
        } else {
            // No authored size: the BASE art's content height maps to
            // the creature's STANDING height (not the card box, which
            // shrinks while prone), and pose/variant art inherits that
            // same pixel scale — art sets are drawn at one canvas scale
            // (Niffy's rule; his prototype's pose_ratio band flags sets
            // that break it), so a prone image (content height = body
            // thickness) isn't stretched to standing height. A pose
            // file at a genuinely different scale is a content bug:
            // author `size` in its sidecar to override. The prepare
            // pass feeds this same pose-to-standing content ratio into
            // the core geometry store (finding 7), so the prone
            // reservation/hit box resolves to exactly the height drawn
            // here — bounds and pixels share one pose scale.
            let r = art.unwrap_or(a);
            let rts = r.texture.size_vec2();
            let content_h = ((r.bbox[3] - r.bbox[1]) * rts.y).max(1.0);
            // The unit's RESERVED standing height — the shared
            // geometry the solver placed with — so the drawn sprite,
            // the reservation, and the click rect agree on world
            // scale (finding 6: one geometry contract).
            let standing = unit.standing.h;
            let s = standing * px_per_unit / content_h;
            tracing::debug!(
                "[field-scale] {} pose_art={} standing={standing:.2} content_h={content_h:.0} authored_size=None scale={s:.4}",
                creature.name,
                !std::ptr::eq(r, a),
            );
            s
        };
        let (draw_w, draw_h) = (ts.x * scale, ts.y * scale);
        let dest = egui::Rect::from_min_size(
            egui::pos2(
                card.center().x - feet[0] * draw_w,
                card.bottom() - feet[1] * draw_h,
            ),
            egui::vec2(draw_w, draw_h),
        );
        (a, dest)
    });

    // Contact shadow stays at the floor footprint; softens with lift.
    // A sidecar footprint sizes it to the pose (a sprawled body casts a
    // long, wide shadow); the generic standee ellipse otherwise.
    let (shadow_scale, shadow_alpha) = resolved
        .as_ref()
        .and_then(|r| r.lift())
        .map(|l| (l.shadow_scale, (l.shadow_opacity * 60.0) as u8))
        .unwrap_or(if lift.is_some() {
            (0.55, 24)
        } else {
            (1.0, 60)
        });
    let foot_pt = map.pt(foot_x, foot_y);
    let (shadow_c, shadow_rx, shadow_ry) = match sprite
        .as_ref()
        .and_then(|(a, dest)| a.footprint.map(|fp| (fp, dest)))
    {
        Some((fp, dest)) => {
            let cx = fp
                .center
                .map(|c| dest.left() + c[0] * dest.width())
                .unwrap_or(foot_pt.x);
            (
                egui::pos2(cx, foot_pt.y),
                dest.width() * fp.rx * shadow_scale,
                dest.width() * fp.effective_ry() * shadow_scale,
            )
        }
        None => {
            let w = card.width() * 0.55 * shadow_scale;
            (foot_pt, w, w * 0.24)
        }
    };
    painter.add(egui::epaint::PathShape::convex_polygon(
        ellipse_points(shadow_c, shadow_rx, shadow_ry),
        Color32::from_black_alpha(shadow_alpha),
        Stroke::NONE,
    ));

    let mut animated = false;
    let mut body = card;
    if let (Some(cache), Some((art, dest))) = (art_cache, sprite) {
        // ---- sprite card -------------------------------------------
        let tint = if dead {
            Color32::from_gray(110)
        } else {
            Color32::WHITE
        };
// >>>doll
        // The v2 pass lights the body itself (lamps, shadows), when on.
        let gpu_drawn = crate::field_gpu_stage::with_current(|g| g.paint_sprite(painter, cache, &art.texture, art, dest, field.ground_z(unit), dead)).unwrap_or(false);
        if !gpu_drawn {
// <<<doll
        painter.image(
            art.texture.id(),
            dest,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            tint,
        );
// >>>doll
        // Firelight: the unlit art laid back over the lit, fire-tinted.
        if !dead {
            crate::field_lights::paint_sprite_glow(painter, cache, &art.texture, dest);
        }
        }
// <<<doll
        body = dest;
        // Manifest overlays: quad layers warp/scale with the card,
        // screen layers sit flat above it. Ranked art resolves its
        // {severity} from the live derived-effect store.
        if let Some(resolved) = &resolved {
            animated |= paint_card_overlays(
                painter,
                cache,
                resolved,
                art,
                dest,
                now_ms,
                &|name| (view.effect_severity)(&creature.id, name),
            );
            // Per-part wound art from the extended feed (bridge-fed
            // `injuries` attr): the skin's part tables key the same
            // R1-R3 ranks as the player doll.
            if let Some(f) = flags {
                paint_wound_overlays(painter, cache, resolved, art, dest, &f.injuries);
            }
        }
    } else /* >>>doll */ if let Some(puppet_body) = paint_creature_puppet(
        painter,
        card,
        unit,
        creature,
        now_ms,
        view,
        art_cache,
        resolved.as_ref(),
        &mut animated,
    ) {
        // ---- creature puppet (addition; not in VellumFE) -------------
        // The family's BAKED 3D puppet, posed from the creature's state and
        // shaded, replaces the flat doll whenever one resolves. The doll
        // below stays as the fallback. See creature_puppet.rs.
        body = puppet_body;
    } else if let Some(doll_body) = paint_creature_doll(
        painter,
        card,
        unit,
        creature,
        now_ms,
        view,
        art_cache,
        resolved.as_ref(),
        &mut animated,
    ) {
        // ---- creature doll (addition; not in VellumFE) ---------------
        // The family's baked label map, tinted by wound rank, replaces the
        // placeholder standee whenever a rig resolves. See creature_doll.rs.
        body = doll_body;
    } else /* <<<doll */ {
        // ---- placeholder standee -----------------------------------
        // Posture: prone/kneeling/sitting squash toward the ground
        // until real variant art exists.
        let downed = flags.is_some_and(|f| {
            f.has_flag("prone") || f.has_flag("kneeling") || f.has_flag("sitting")
        });
        if downed {
            body.set_top(body.top() + body.height() * 0.45);
        }
        let color = body_color(noun, dead);
        let head_r = body.width() * 0.22;
        let torso = egui::Rect::from_min_max(
            egui::pos2(body.left() + body.width() * 0.18, body.top() + head_r * 1.6),
            egui::pos2(body.right() - body.width() * 0.18, body.bottom()),
        );
        painter.rect_filled(torso, head_r * 0.8, color);
        painter.circle_filled(
            egui::pos2(body.center().x, body.top() + head_r),
            head_r,
            color,
        );
        // Boss brow: a heavier outline instead of a bigger palette.
        if flags.is_some_and(|f| f.is_boss()) {
            painter.rect_stroke(
                torso,
                head_r * 0.8,
                Stroke::new(2.0, Color32::from_rgb(0xc9, 0xa2, 0x27)),
                egui::StrokeKind::Outside,
            );
        }
        if let Some(f) = flags {
            paint_placeholder_wounds(painter, body, &f.injuries);
        }
    }
    if dead {
        let m = body.width() * 0.24;
        let stroke = Stroke::new(2.5, Color32::from_rgb(0xd2, 0x4b, 0x3c));
        painter.line_segment(
            [
                egui::pos2(body.left() + m, body.top() + m * 0.6),
                egui::pos2(body.right() - m, body.top() + m * 1.4),
            ],
            stroke,
        );
        painter.line_segment(
            [
                egui::pos2(body.right() - m, body.top() + m * 0.6),
                egui::pos2(body.left() + m, body.top() + m * 1.4),
            ],
            stroke,
        );
    }

// >>>doll
    // Everything from here on labels the card: over the scene's occluders.
    let labels = crate::field_occluders::label_painter(painter);
    let painter = &labels;
// <<<doll
    // HP bar under the card, when the extended feed reports health
    // (bridge-fed). Estimated maxes render dimmed.
    let mut label_y = card.bottom() + 2.0;
    if let Some((hp, max)) = flags.and_then(|f| Some((f.health?, f.max_health?))) {
        if max > 0 && !dead {
            let frac = (hp as f32 / max as f32).clamp(0.0, 1.0);
            let bar = egui::Rect::from_min_max(
                egui::pos2(card.left(), card.bottom() + 1.0),
                egui::pos2(card.right(), card.bottom() + 4.0),
            );
            let fill_color = if frac > 0.6 {
                Color32::from_rgb(0x4c, 0xaf, 0x50)
            } else if frac > 0.3 {
                Color32::from_rgb(0xd4, 0xa0, 0x17)
            } else {
                Color32::from_rgb(0xd2, 0x4b, 0x3c)
            };
            let alpha = if flags.is_some_and(|f| f.hp_estimated) {
                140
            } else {
                230
            };
            painter.rect_filled(bar, 1.5, Color32::from_black_alpha(120));
            let fill = egui::Rect::from_min_max(
                bar.min,
                egui::pos2(bar.left() + bar.width() * frac, bar.bottom()),
            );
            painter.rect_filled(
                fill,
                1.5,
                Color32::from_rgba_unmultiplied(
                    fill_color.r(),
                    fill_color.g(),
                    fill_color.b(),
                    alpha,
                ),
            );
            label_y = bar.bottom() + 2.0;
        }
    }

    // Noun label at the feet.
    let label_color = if dark {
        Color32::from_white_alpha(150)
    } else {
        Color32::from_black_alpha(150)
    };
    painter.text(
        egui::pos2(card.center().x, label_y),
        egui::Align2::CENTER_TOP,
        noun,
        egui::FontId::proportional((11.0 * map.scale.max(0.6)).clamp(9.0, 13.0)),
        label_color,
    );

    // Status badges above the head (skip posture; it's drawn).
    let mut top_y = body.top();
    if let Some(f) = flags {
        let badges: Vec<&str> = f
            .statuses
            .iter()
            .map(String::as_str)
            .filter(|s| {
                !matches!(
                    *s,
                    "prone" | "kneeling" | "sitting" | "flying" | "hovering" | "stunned"
                )
            /* >>>doll */ && !crate::creature_indicators::has_icon(s) /* <<<doll */ })
            .collect();
        if !badges.is_empty() {
            let text = badges.join(" ");
            painter.text(
                egui::pos2(card.center().x, top_y - 4.0),
                egui::Align2::CENTER_BOTTOM,
                text,
                egui::FontId::proportional(9.5),
                Color32::from_rgb(0x4f, 0xd1, 0xc5),
            );
            top_y -= 14.0;
        }
    }
// >>>doll
    // Status icons (sleeping Zz, web, spiral...) for the statuses that have
    // one; the text above covers the rest.
    if let Some(f) = flags {
        let (y, moving) = crate::creature_indicators::paint(
            painter,
            card.center().x,
            top_y,
            card.width(),
            &f.statuses,
            dead,
            now_ms,
        );
        top_y = y;
        animated |= moving;
    }
// <<<doll

    // Stun swirl: three orbiting stars, wall-clock phased.
    if flags.is_some_and(|f| f.has_flag("stunned")) && !dead {
        animated = true;
        let orbit_r = (card.width() * 0.42).max(9.0);
        let cy = top_y - orbit_r * 0.42;
        for k in 0..3 {
            let ph = now_ms as f32 * 0.0026 + k as f32 * 2.0944;
            let sx = card.center().x + ph.cos() * orbit_r;
            let sy = cy + ph.sin() * orbit_r * 0.34;
            let depth = (ph.sin() + 1.0) / 2.0;
            let size = (card.width() * (0.10 + 0.05 * depth)).max(3.2);
            painter.circle_filled(
                egui::pos2(sx, sy),
                size * 0.5,
                Color32::from_rgba_unmultiplied(0xf2, 0xc2, 0x3c, 140 + (depth * 100.0) as u8),
            );
        }
        top_y = cy - orbit_r * 0.34;
    }

    // Targeting reticule: downward green triangle, above everything.
    if is_target {
        let w = (card.width() * 0.34).max(11.0);
        let h = w * 0.86;
        let tip = egui::pos2(card.center().x, (top_y - w * 0.30).min(body.top()));
        painter.add(egui::epaint::PathShape::convex_polygon(
            vec![
                tip,
                egui::pos2(tip.x - w / 2.0, tip.y - h),
                egui::pos2(tip.x + w / 2.0, tip.y - h),
            ],
            Color32::from_rgb(0x48, 0xc7, 0x74),
            Stroke::new(1.2, Color32::from_rgba_unmultiplied(12, 40, 22, 217)),
        ));
    }
    animated
}

/// Manifest-driven overlay layers over one sprite card. Quad layers
/// scale with the card (body-wrap = the art's alpha bbox, anchored =
/// placed at an anchor fraction); screen layers sit flat above it and
/// may animate (orbit / pulse from wall clock). Returns true when any
/// active layer animates.
fn paint_card_overlays(
    painter: &egui::Painter,
    cache: &crate::creature_art::CreatureArtCache,
    resolved: &gs_field::cards::ResolvedCard<'_>,
    art: &crate::creature_art::CreatureArt,
    dest: egui::Rect,
    now_ms: f64,
    severity_of: &dyn Fn(&str) -> Option<u8>,
) -> bool {
    use gs_field::skins::{AnimateKind, OverlaySpace};
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    let mut animated = false;
    // The art's alpha bbox mapped into the dest rect: what body-wrap
    // overlays cover, so canvas padding in the base costs nothing.
    let bbox = egui::Rect::from_min_max(
        egui::pos2(
            dest.left() + art.bbox[0] * dest.width(),
            dest.top() + art.bbox[1] * dest.height(),
        ),
        egui::pos2(
            dest.left() + art.bbox[2] * dest.width(),
            dest.top() + art.bbox[3] * dest.height(),
        ),
    );
    let anchor_pt = |name: &str| -> egui::Pos2 {
        // Manifest calibration wins; the image's sidecar anchors next;
        // the art's derived head/feet cover the common case; then the
        // built-in resting positions; centre is the never-crash
        // fallback.
        let frac = resolved
            .authored_anchor(name)
            .or_else(|| art.anchor(name))
            .or(match name {
                "head" => Some(art.head),
                "feet" => Some(art.feet),
                _ => None,
            })
            .or_else(|| gs_field::skins::default_creature_anchor(name))
            .unwrap_or([0.5, 0.5]);
        egui::pos2(
            dest.left() + frac[0] * dest.width(),
            dest.top() + frac[1] * dest.height(),
        )
    };
    for overlay in &resolved.overlays {
        let image = if overlay.image.contains("{severity}") {
            // Ranked art: severity comes from the live derived-effect
            // store, looked up by the effect name the overlay's own
            // condition tests. No live rank = nothing to draw.
            let mut ids = Vec::new();
            overlay.when.referenced_crtr_status_ids(&mut ids);
            let Some(sev) = ids.iter().find_map(|id| severity_of(id)) else {
                continue;
            };
            overlay.image.replace("{severity}", &sev.to_string())
        } else if overlay.image.contains('{') {
            continue; // per-creature placeholders are refused at load
        } else {
            overlay.image.clone()
        };
        let Some(texture) = cache.overlays.get(&image).cloned().flatten()/* >>>doll */ .map(|t| crate::creature_light::worn(cache, &image, t, art, overlay.anchor.as_deref(), matches!(overlay.space, OverlaySpace::Quad))) /* <<<doll */  else {
            continue;
        };
        let anim = overlay.animate.as_ref();
        animated |= anim.is_some();
        match overlay.space {
            OverlaySpace::Quad => {
                let rect = match overlay.anchor.as_deref() {
                    // Body-wrap: stretched over the sprite's alpha bbox.
                    None => bbox,
                    Some(name) => {
                        // Anchored: the calibrated overlay_scale (or
                        // the 1.0 identity) times the creature's drawn
                        // width — world-size scaling inherent, no
                        // built-in shrink factors.
                        let pt = anchor_pt(name);
                        let ts = texture.size_vec2();
                        let w = dest.width()
                            * cache.overlay_scale(&overlay.image).unwrap_or(1.0);
                        let h = w * ts.y / ts.x.max(1.0);
                        egui::Rect::from_center_size(pt, egui::vec2(w, h))
                    }
                };
                let alpha = match anim.map(|a| a.kind) {
                    Some(AnimateKind::Pulse) => {
                        let period = anim.map(|a| a.period_ms).unwrap_or(2400).max(1);
                        let ph = (now_ms / period as f64 * std::f64::consts::TAU).sin();
                        (170.0 + ph * 85.0) as u8
                    }
                    Some(AnimateKind::Flicker) => {
                        let period = anim.map(|a| a.period_ms).unwrap_or(2400).max(1) as f64;
                        if (now_ms / (period / 6.0)) as u64 % 3 == 0 {
                            120
                        } else {
                            255
                        }
                    }
                    _ => 255,
                };
                painter.image(texture.id(), rect, uv, Color32::from_white_alpha(alpha)/* >>>doll */ * crate::creature_light::wearer_tint() /* <<<doll */ );
            }
            OverlaySpace::Screen => {
                let pt = anchor_pt(overlay.anchor.as_deref().unwrap_or("head"));
                match anim.map(|a| a.kind) {
                    Some(AnimateKind::Orbit) => {
                        let a = anim.expect("kind implies spec");
                        let rx = dest.width() * a.rx;
                        let ry = dest.width() * a.ry;
                        let period = a.period_ms.max(1) as f64;
                        for k in 0..a.count.max(1) {
                            let ph = (now_ms / period * std::f64::consts::TAU) as f32
                                + k as f32 * std::f32::consts::TAU / a.count.max(1) as f32;
                            let depth = (ph.sin() + 1.0) / 2.0;
                            let size = dest.width() * (0.10 + 0.05 * depth);
                            let center = egui::pos2(
                                pt.x + ph.cos() * rx,
                                pt.y - ry * 1.5 + ph.sin() * ry,
                            );
                            painter.image(
                                texture.id(),
                                egui::Rect::from_center_size(center, egui::vec2(size, size)),
                                uv,
                                Color32::from_white_alpha(140 + (depth * 100.0) as u8),
                            );
                        }
                    }
                    _ => {
                        // Static screen layer: sits just above the
                        // anchor, sized like an anchored quad layer
                        // (calibrated overlay_scale, identity default).
                        let ts = texture.size_vec2();
                        let w = dest.width()
                            * cache.overlay_scale(&overlay.image).unwrap_or(1.0);
                        let h = w * ts.y / ts.x.max(1.0);
                        painter.image(
                            texture.id(),
                            egui::Rect::from_center_size(
                                egui::pos2(pt.x, pt.y - h * 0.8),
                                egui::vec2(w, h),
                            ),
                            uv,
                            Color32::WHITE,
                        );
                    }
                }
            }
        }
    }
    animated
}

/// Per-part wound sprites from the extended feed's injuries list: each
/// (part, rank) resolves art through the skin's part tables and draws
/// centred on that part's anchor — the creature-card analog of the
/// player injury doll's overlay pass. Parts without authored art (or
/// without an anchor beyond the fallbacks) simply don't draw.
fn paint_wound_overlays(
    painter: &egui::Painter,
    cache: &crate::creature_art::CreatureArtCache,
    resolved: &gs_field::cards::ResolvedCard<'_>,
    art: &crate::creature_art::CreatureArt,
    dest: egui::Rect,
    injuries: &[(String, u8)],
) {
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    for (part, rank) in injuries {
        // The AUTHORED anchor (calibrator/sidecar/skin) is the mode
        // switch — the tale is the anchor:
        //   no anchor  -> the art is canvas-matched to the base:
        //                 stretched 1:1 over the drawn base (perfect
        //                 alignment, world scaling inherited);
        //   anchor     -> the art is artist-cropped sprite art and
        //                 behaves exactly like the shared wounds pool:
        //                 calibrated overlay_scale (1.0 identity) times
        //                 the creature's drawn width, centred there.
        // Global/manifest art always draws anchored (it can't be
        // canvas-matched to any one creature), falling through to the
        // built-in default anchors.
        let authored = resolved
            .authored_anchor(part)
            .or_else(|| art.anchor(part));
        let frac = authored
            .or_else(|| gs_field::skins::default_creature_anchor(part))
            .or(match part.as_str() {
                "head" => Some(art.head),
                _ => None,
            })
            .or_else(|| gs_field::skins::default_doll_anchor(part))
            .unwrap_or([0.5, 0.5]);
        let pt = egui::pos2(
            dest.left() + frac[0] * dest.width(),
            dest.top() + frac[1] * dest.height(),
        );
        // Wound art, tier-locked: the creature's own tier overlays
        // ({token}_{loc}{rank}) when the tier ships ANY wound art —
        // never mixed with another source. The manifest's part
        // tables serve only tiers with no wound art of their own;
        // the procedural rank marker covers everything else, so
        // wounds stay visible on every skin.
        let tier_key = art
            .extra(&format!("{}{rank}", part.to_ascii_lowercase()))
            .map(|path| path.to_string_lossy().into_owned());
        let tier_texture = tier_key
            .as_deref()
            .and_then(|key| cache.overlays.get(key).cloned().flatten());
        if let Some(texture) = tier_texture {
            match authored {
                // Canvas-matched: full overlay over the drawn base.
                None => {
                    painter.image(texture.id(), dest, uv, Color32::WHITE/* >>>doll */ * crate::creature_light::wearer_tint() /* <<<doll */ );
                }
                // Artist-cropped sprite: calibrated scale × creature.
                Some(_) => {
                    let scale = tier_key
                        .as_deref()
                        .and_then(|key| cache.overlay_scale(key))
                        .unwrap_or(1.0);
                    let ts = texture.size_vec2();
                    let w = dest.width() * scale;
                    let h = w * ts.y / ts.x.max(1.0);
                    painter.image(
                        texture.id(),
                        egui::Rect::from_center_size(pt, egui::vec2(w, h)),
                        uv,
                        Color32::WHITE/* >>>doll */ * crate::creature_light::wearer_tint() /* <<<doll */ ,
                    );
                }
            }
            continue;
        }
        let global = if art.has_wound_extras() {
            None
        } else {
            resolved.part_overlay(part, *rank).and_then(|image| {
                cache
                    .overlays
                    .get(image)
                    .cloned()
                    .flatten()
                    /* >>>doll */ .map(|t| crate::creature_light::worn(cache, image, t, art, Some(part.as_str()), true)) /* <<<doll */ .map(|texture| (image, texture))
            })
        };
        match global {
            Some((image, texture)) => {
                // No invented sizes: the calibrated overlay_scale (or
                // the 1.0 identity) times the creature's drawn width,
                // so world-size scaling is inherent.
                let ts = texture.size_vec2();
                let w = dest.width() * cache.overlay_scale(image).unwrap_or(1.0);
                let h = w * ts.y / ts.x.max(1.0);
                painter.image(
                    texture.id(),
                    egui::Rect::from_center_size(pt, egui::vec2(w, h)),
                    uv,
                    Color32::WHITE/* >>>doll */ * crate::creature_light::wearer_tint() /* <<<doll */ ,
                );
            }
            None => paint_wound_marker(painter, pt, dest.width(), *rank),
        }
    }
}

/// Procedural wound marker: a rank-colored dot with a darker rim,
/// CreatureBar-style, for skins (and placeholder standees) without
/// authored per-part wound art. R1 yellow, R2 orange, R3 red.
fn paint_wound_marker(painter: &egui::Painter, pt: egui::Pos2, card_w: f32, rank: u8) {
    let color = match rank {
        1 => Color32::from_rgb(0xe0, 0xc0, 0x30),
        2 => Color32::from_rgb(0xe0, 0x7a, 0x20),
        _ => Color32::from_rgb(0xd2, 0x2b, 0x2b),
    };
    let r = (card_w * 0.055).clamp(2.5, 6.0);
    painter.circle_filled(pt, r, color);
    painter.circle_stroke(pt, r, Stroke::new(1.0, Color32::from_black_alpha(160)));
}

// >>>doll
/// A 1x1 transparent texture, created once per context. The overlay pass
/// takes a `CreatureArt` and reads its texture's SIZE for anchored layers
/// but never draws it; the puppet has no texture of its own, so it lends
/// this one. Square, so an anchored overlay keeps its authored aspect.
fn blank_texture(ctx: &egui::Context) -> egui::TextureHandle {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_insert_with::<egui::TextureHandle>(
            egui::Id::new("gs_field_egui::puppet_blank"),
            || {
                ctx.load_texture(
                    "puppet:blank",
                    egui::ColorImage::from_rgba_unmultiplied([1, 1], &[0, 0, 0, 0]),
                    egui::TextureOptions::NEAREST,
                )
            },
        )
        .clone()
    })
}

/// The creature PUPPET for a card with no sprite art (ADDITION, not in
/// VellumFE): resolve the family (`gs_field::puppets`, the doll cascade with
/// "has a baked puppet" as the test), pose the baked 3D figure for this
/// creature's state, and draw it. Returns the screen rect the figure fills —
/// the caller's `body` — or None when no puppet resolves or the bake does
/// not load, in which case the flat doll draws, and failing that the old
/// placeholder standee: three tiers, none of them lost.
///
/// Placement mirrors `paint_creature_doll`: the figure's ground point on the
/// card's foot line at the card's centre x, its STANDING height covering the
/// unit's standing height at the card's pixel scale — so a kneeling or
/// prone figure shrinks by exactly as much as the pose really does.
#[allow(clippy::too_many_arguments)]
fn paint_creature_puppet(
    painter: &egui::Painter,
    card: egui::Rect,
    unit: &Unit,
    creature: &gs_field::Creature,
    now_ms: f64,
    view: &CreatureFieldView<'_>,
    art_cache: Option<&crate::creature_art::CreatureArtCache>,
    resolved: Option<&gs_field::cards::ResolvedCard<'_>>,
    animated: &mut bool,
) -> Option<egui::Rect> {
    use crate::creature_puppet::{PoseChoice, PuppetCache, PuppetKey, PuppetPlacement};
    let flags = creature.flags.as_ref();
    let dead = creature.is_dead();
    let noun = creature.noun.as_deref().unwrap_or("creature");
    let injuries: &[(String, u8)] = flags.map(|f| f.injuries.as_slice()).unwrap_or_default();

    let ctx = painter.ctx();
    let shared = PuppetCache::shared(ctx);
    let mut puppets = shared.lock().unwrap_or_else(|e| e.into_inner());
    puppets.begin_frame(ctx.cumulative_frame_nr());
    puppets.set_props(crate::creature_puppet::puppet_props(ctx));
    let family = puppets.family_for(&creature.name, creature.noun.as_deref())?;

    // Stable per-creature variety, the doll's own bits: which way the
    // figure faces, and which way it falls (fallen-side vs supine, and
    // which of a body type's dead poses it takes).
    let (mirror, on_side) = crate::creature_doll::variety(&creature.id);
    let has = |name: &str| flags.is_some_and(|f| f.has_flag(name));
    let pose = PoseChoice::of(dead, &has, on_side, u8::from(on_side));
    // Died lying down: the dead pose nearest its lie, not a roll into another.
    let pose = puppets.dead_choice(&creature.id, &family, pose);
    // The card's pixel scale, exactly as the sprite and doll branches take
    // it: the reserved standing height in card pixels. Flying / hovering
    // already moved `card`, so the foot line follows the lift.
    let px_per_unit = card.height() / unit.size.h.max(0.01);
    let standing_px = unit.standing.h * px_per_unit;
    let foot = egui::pos2(card.center().x, card.bottom());
    let color = body_color(noun, dead);
    let key = PuppetKey::new(
        pose,
        injuries,
        dead,
        flags.is_some_and(|f| f.is_boss()),
        [color.r(), color.g(), color.b()],
        mirror,
        standing_px,
    );
    let key = match crate::creature_puppet::puppet_facing(painter.ctx(), &creature.id) {
        Some(deg) => key.with_facing(deg),
        None => key,
    };
    let key = key.with_effects(PuppetKey::effects_of(&has)).with_splats(crate::creature_puppet::puppet_splats(ctx), crate::creature_puppet::puppet_splat_params(ctx));
    // Alive, it breathes: the key steps round its family's idle cycle, and
    // the next frame is asked for when the step changes (not every frame).
    let breath = if crate::creature_puppet::puppet_breath(ctx) {
        puppets.breath(&family, &creature.id, pose, now_ms / 1000.0)
    } else {
        None
    };
    let key = match breath {
        Some((step, until_next)) => {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(until_next));
            key.with_breath(step)
        }
        None => key,
    };
    let skin = crate::creature_puppet::puppet_skin(ctx);
    let light = crate::creature_light::current_light(ctx);
    // On the GPU (`skin_gpu`, relit by `relight_gpu`) when asked, with the
    // v2 pass drawing and the scene lit: the figure goes over as a mesh.
    let bare = if skin && light.is_some() && crate::creature_puppet::puppet_gpu_skin(ctx) && crate::field_gpu_stage::with_current(|_| ()).is_some() {
        crate::creature_puppet::Bare::Mesh
    } else {
        crate::creature_puppet::Bare::No
    };
    puppets.set_light(light.clone());
    // Seen alive and just died: it falls (a procedural ragdoll) into its
    // dead pose. Any other change of pose eases from the old one into the
    // new. Both are drawn fresh every frame until they settle.
    // A hit (a wound rank rising) makes it flinch, drawn fresh each frame
    // until it settles; killed mid-flinch, it falls from that shape.
    let motion = puppets.motion(&creature.id, pose, now_ms / 1000.0);
    let flinch = puppets.flinch(&creature.id, &key.ranks, now_ms / 1000.0);
    let posed = match motion {
        crate::creature_puppet::Motion::Falling { from, t } => {
            *animated = true;
            ctx.request_repaint();
            let died_in = puppets.death_flinch(&creature.id);
            puppets.posed_falling_as(&family, &key, skin, from, t, died_in, bare)?
        }
        crate::creature_puppet::Motion::Changing { from, u } => {
            *animated = true;
            ctx.request_repaint();
            puppets.posed_changing_as(&family, &key, skin, from, u, flinch, bare)?
        }
        // A rank-3 nerve wound twitches every few seconds (as the doll's):
        // look again soon, and draw the twitch fresh while it lasts.
        crate::creature_puppet::Motion::Still => match flinch.or_else(|| {
            let twitch = crate::creature_puppet::nerve_twitch(&key, &creature.id, now_ms / 1000.0);
            if !key.dead && crate::creature_puppet::nerve_rank(&key) >= 3 {
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
            }
            twitch
        }) {
            Some(f) => {
                *animated = true;
                ctx.request_repaint();
                puppets.posed_flinching_as(&family, &key, skin, f, bare)?
            }
            None => puppets.posed_cached(&family, &key, skin, bare)?,
        },
    };
    // The parts this family's puppet actually carries, taken while the lock
    // is already held (the wound pass below needs it, and re-locking once
    // per creature per frame would be the one avoidable cost here).
    let drawn = puppets.parts(&family);
    drop(puppets);
    let place = PuppetPlacement::fit(&posed, foot, standing_px);
    // Lamps light the figure from where their flames are, and it casts
    // the shadows of those that cast them. The card's standing height is
    // the puppet's, so that fixes feet per floor unit.
    let ft_per_unit = posed.standing_ft / unit.standing.h.max(1e-3);
    let (lamps, casting, flicker) = crate::field_lights::puppet_lamps(ft_per_unit);
    // The figure's own contact shadow, under it: the card's generic ellipse
    // is already down, and this one is shaped like the pose.
    place.paint(painter, &posed.shadow);
    // The v2 pass lights the skin itself (lamps, the shadows it casts and
    // takes), when on and the surface was kept (scene lighting is up).
    let m_per_ft = unit.standing.h * crate::field_gpu_stage::METRES_PER_CARD_UNIT / posed.standing_ft.max(1e-3);
    let gpu_drawn = crate::field_gpu_stage::with_current(|g| {
        // a figure the GPU draws: its skin's mesh, or its splat cloud
        let figure = match (&posed.mesh, &posed.splats) {
            (Some((mesh, texture)), _) => Some((crate::skin_gpu::Figure::Skin { mesh: mesh.clone(), texture: texture.clone() }, mesh.origin, mesh.w, mesh.h)),
            (_, Some(f)) => Some((crate::skin_gpu::Figure::Splats(f.clone()), f.origin, f.w, f.h)),
            _ => None,
        };
        match (figure, light.as_deref(), posed.skin.as_ref(), posed.surface.as_ref()) {
            (Some((figure, origin, w, h)), Some(light), _, _) => {
                let frame = gs_puppet::Skin { origin, w, h, rgba: Vec::new() };
                g.paint_figure(painter, figure, light, place.skin_rect(&frame), view.field.ground_z(unit), m_per_ft)
            }
            (_, _, Some(skin), Some(surface)) if !skin.is_empty() => g.paint_skin(painter, skin, surface, place.skin_rect(skin), view.field.ground_z(unit), m_per_ft),
            _ => false,
        }
    })
    .unwrap_or(false);
    if !gpu_drawn {
        place.paint_figure_shadows(painter, &posed, &casting, flicker);
        place.paint_figure(painter, &posed);
        place.paint_figure_lamps(painter, &posed, &lamps, flicker);
    }

    // Skin status overlays ride the same code as sprite cards, over the
    // posed body rect. The puppet has no texture, so anchors come from the
    // drawn box: head at the top centre, feet at the ground point.
    if let (Some(cache), Some(resolved)) = (art_cache, resolved) {
        if !resolved.overlays.is_empty() {
            let body = place.body;
            let frac = |p: egui::Pos2| {
                [
                    (p.x - body.left()) / body.width().max(1e-3),
                    (p.y - body.top()) / body.height().max(1e-3),
                ]
            };
            let art = crate::creature_art::CreatureArt {
                texture: blank_texture(ctx),
                head: frac(egui::pos2(body.center().x, body.top())),
                feet: frac(foot),
                bbox: [0.0, 0.0, 1.0, 1.0],
                anchors: Default::default(),
                footprint: None,
                size: None,
                lift: None,
                alpha_px: 0,
                extras: Default::default(),
            };
            *animated |= paint_card_overlays(
                painter,
                cache,
                resolved,
                &art,
                body,
                now_ms,
                &|name| (view.effect_severity)(&creature.id, name),
            );
        }
    }

    // Wounds the puppet has no segment for (eyes, back and nsys on families
    // that do not carry them): the placeholder standee's treatment over the
    // posed body rect, so nothing a creature is carrying goes unshown.
    let unplaced: Vec<(String, u8)> = injuries
        .iter()
        .filter(|(part, rank)| *rank > 0 && !drawn.iter().any(|p| p.eq_ignore_ascii_case(part)))
        .cloned()
        .collect();
    if !unplaced.is_empty() {
        paint_placeholder_wounds(painter, place.body, &unplaced);
    }
    Some(place.body)
}

/// The creature doll for a card with no sprite art (ADDITION, not in
/// VellumFE): resolve the rig (`gs_field::dolls` cascade, cached), fetch the
/// composed texture for this creature's look (built only when the look
/// changes), and draw it posed. Returns the screen rect the doll fills — the
/// caller's `body` — or None when no rig resolves / decodes, in which case
/// the old placeholder standee draws.
///
/// Placement mirrors the sprite branch: ground row on the card's foot line,
/// content height = the unit's STANDING height at the card's pixel scale,
/// feet anchor on the card's centre x.
#[allow(clippy::too_many_arguments)]
fn paint_creature_doll(
    painter: &egui::Painter,
    card: egui::Rect,
    unit: &Unit,
    creature: &gs_field::Creature,
    now_ms: f64,
    view: &CreatureFieldView<'_>,
    art_cache: Option<&crate::creature_art::CreatureArtCache>,
    resolved: Option<&gs_field::cards::ResolvedCard<'_>>,
    animated: &mut bool,
) -> Option<egui::Rect> {
    use crate::creature_doll::{DollCache, DollLayout, DollLook, DollPose};
    let flags = creature.flags.as_ref();
    let dead = creature.is_dead();
    let noun = creature.noun.as_deref().unwrap_or("creature");
    let injuries: &[(String, u8)] = flags.map(|f| f.injuries.as_slice()).unwrap_or_default();

    let ctx = painter.ctx();
    let shared = DollCache::shared(ctx);
    let mut dolls = shared.lock().unwrap_or_else(|e| e.into_inner());
    dolls.begin_frame(ctx.cumulative_frame_nr());
    let family = dolls.family_for(&creature.name, creature.noun.as_deref())?;
    let color = body_color(noun, dead);
    let look = DollLook::new(
        [color.r(), color.g(), color.b()],
        injuries,
        dead,
        flags.is_some_and(|f| f.is_boss()),
    );
    let doll = dolls.texture(ctx, &family, &look)?;
    drop(dolls);
    let map = &doll.map;

    // Rigid poses: prone / dead lie down (turned 90°, to a side picked by
    // the creature's id); kneeling / sitting squash toward the ground like
    // the old standee's 0.45 rule. Flying / hovering already moved `card`.
    let (mirror, head_left) = crate::creature_doll::variety(&creature.id);
    let pose = if dead || flags.is_some_and(|f| f.has_flag("prone")) {
        DollPose::Lying { head_left }
    } else if flags.is_some_and(|f| f.has_flag("kneeling") || f.has_flag("sitting")) {
        DollPose::Squashed(0.55)
    } else {
        DollPose::Standing
    };
    let px_per_unit = card.height() / unit.size.h.max(0.01);
    let layout = DollLayout::fit(
        &map.meta,
        (map.width, map.height),
        card,
        unit.standing.h * px_per_unit,
        pose,
        mirror,
    );
    painter.add(egui::Shape::mesh(
        layout.mesh(doll.texture.id(), Color32::WHITE),
    ));

    // Skin status overlays ride the same code as sprite cards: the doll is
    // handed over as art whose anchors are the rig's, already carried
    // through the pose (mirror / turn), over the posed body rect.
    if let (Some(cache), Some(resolved)) = (art_cache, resolved) {
        if !resolved.overlays.is_empty() {
            let anchor = |name: &str| map.meta.anchor(name).map(|a| layout.frac(a));
            let head = anchor("head").unwrap_or([0.5, 0.0]);
            let feet = anchor("feet").unwrap_or([0.5, 1.0]);
            let art = crate::creature_art::CreatureArt {
                texture: doll.texture.clone(),
                head,
                feet,
                bbox: [0.0, 0.0, 1.0, 1.0],
                anchors: ["head", "feet", "mouth", "saddle"]
                    .into_iter()
                    .filter_map(|n| Some((n.to_string(), anchor(n)?)))
                    .collect(),
                footprint: None,
                size: None,
                lift: None,
                alpha_px: 0,
                extras: Default::default(),
            };
            *animated |= paint_card_overlays(
                painter,
                cache,
                resolved,
                &art,
                layout.body,
                now_ms,
                &|name| (view.effect_severity)(&creature.id, name),
            );
        }
    }

    // Wounds on parts this rig has no pixels for. Eyes: a marker dot at the
    // rig's [eyes] point. Everything else (back and nsys on a front view, or
    // any part a rig leaves out): the placeholder standee's treatment.
    let mut unplaced: Vec<(String, u8)> = Vec::new();
    for (part, rank) in injuries {
        if *rank == 0 || map.has_part(part) {
            continue;
        }
        match map.meta.eye(part) {
            Some(eye) => paint_wound_marker(painter, layout.pt(eye), layout.body.width(), *rank),
            None => unplaced.push((part.clone(), *rank)),
        }
    }
    if !unplaced.is_empty() {
        paint_placeholder_wounds(painter, layout.body, &unplaced);
    }
    Some(layout.body)
}

// <<<doll
/// Wound markers for the placeholder standee: no skin, no manifest —
/// parts place by their built-in default anchors over the body rect.
fn paint_placeholder_wounds(
    painter: &egui::Painter,
    body: egui::Rect,
    injuries: &[(String, u8)],
) {
    for (part, rank) in injuries {
        // Creature anchors only cover head/mouth/feet/saddle; body-part
        // wounds fall through to the humanoid doll anchors so limbs,
        // eyes, neck, chest, and abdomen each get their own spot on the
        // standee instead of stacking on one centre dot.
        let frac = gs_field::skins::default_creature_anchor(part)
            .or_else(|| gs_field::skins::default_doll_anchor(part))
            .unwrap_or([0.5, 0.5]);
        let pt = egui::pos2(
            body.left() + frac[0] * body.width(),
            body.top() + frac[1] * body.height(),
        );
        paint_wound_marker(painter, pt, body.width(), *rank);
    }
}

fn paint_target_order(
    painter: &egui::Painter,
    map: &StageMap,
    field: &gs_field::solver::CreatureField,
    current_target: &str,
) {
    let order = field.target_order();
    if order.is_empty() {
        return;
    }
    let y_bar = STAGE_H - 16.0;
    let teal = Color32::from_rgb(0x4f, 0xd1, 0xc5);
    for (k, &i) in order.iter().enumerate() {
        let unit = &field.units()[i];
        let (fx, _) = field.foot(unit);
        let is_sel = unit
            .members
            .iter()
            .any(|m| bare_id(m) == current_target);
        painter.circle_filled(
            map.pt(fx, y_bar),
            if is_sel { 5.0 } else { 3.4 } * map.scale.max(0.5),
            if is_sel {
                Color32::from_rgb(0xc9, 0xa2, 0x27)
            } else {
                teal.gamma_multiply(0.85)
            },
        );
        painter.text(
            map.pt(fx, y_bar - 8.0),
            egui::Align2::CENTER_BOTTOM,
            format!("{}", k + 1),
            egui::FontId::monospace(9.0),
            teal,
        );
    }
}

#[cfg(test)]
mod stage_map_tests {
    use super::*;

    // The three reference viewports from the proposal's acceptance tests:
    // tall (the worked example), wide, and stage-aspect.
    const TALL: (f32, f32) = (440.0, 470.0);
    const WIDE: (f32, f32) = (1760.0, 470.0);
    const MATCH: (f32, f32) = (880.0, 470.0);

    fn map_for(size: (f32, f32)) -> StageMap {
        StageMap::fit(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(size.0, size.1),
        ))
    }

    /// Where a backdrop pixel lands on screen: through the backdrop's
    /// stage placement, then the shared StageMap — the ONE transform.
    fn backdrop_point_on_screen(
        map: &StageMap,
        tex: (f32, f32),
        px: f32,
        py: f32,
    ) -> egui::Pos2 {
        let (x0, y0, x1, y1) = backdrop_stage_rect(tex.0, tex.1).unwrap();
        map.pt(
            x0 + px / tex.0 * (x1 - x0),
            y0 + py / tex.1 * (y1 - y0),
        )
    }

    /// The proposal's worked example: at 440x470, stage Y=100 previously
    /// mapped to 167.5 while the same point in an 880x470 backdrop mapped
    /// to 100. Now both go through StageMap and agree exactly.
    #[test]
    fn worked_example_backdrop_and_stage_agree() {
        let map = map_for(TALL);
        let stage_pt = map.pt(440.0, 100.0);
        // Old contain-fit math still holds for stage points: scale 0.5,
        // vertical centring offset (470 - 235) / 2 = 117.5.
        assert!((stage_pt.y - 167.5).abs() < 1e-4, "stage y = {}", stage_pt.y);
        // The same point on an 880x470 backdrop lands in the SAME place.
        let bd_pt = backdrop_point_on_screen(&map, (880.0, 470.0), 440.0, 100.0);
        assert!((bd_pt.x - stage_pt.x).abs() < 1e-4);
        assert!((bd_pt.y - stage_pt.y).abs() < 1e-4, "backdrop y = {}", bd_pt.y);
    }

    /// A stage-aspect backdrop fills the stage exactly, at every window
    /// aspect; every backdrop point coincides with its stage point.
    #[test]
    fn stage_aspect_backdrop_matches_stage_everywhere() {
        for size in [TALL, WIDE, MATCH] {
            let map = map_for(size);
            for (sx, sy) in [(0.0, 0.0), (880.0, 470.0), (220.0, 400.0), (700.0, 35.0)] {
                let a = map.pt(sx, sy);
                let b = backdrop_point_on_screen(&map, (880.0, 470.0), sx, sy);
                assert!((a.x - b.x).abs() < 1e-3 && (a.y - b.y).abs() < 1e-3,
                    "{size:?} stage ({sx},{sy}): stage {a:?} vs backdrop {b:?}");
            }
        }
    }

    /// Forward -> inverse round-trips at all three aspects (pointer hit
    /// testing inverts through stage_pos; it must recover stage coords).
    #[test]
    fn forward_inverse_round_trip() {
        for size in [TALL, WIDE, MATCH] {
            let map = map_for(size);
            for (sx, sy) in [(0.0, 0.0), (880.0, 470.0), (123.4, 456.7), (879.0, 1.0)] {
                let p = map.pt(sx, sy);
                let (rx, ry) = map.stage_pos(p);
                assert!((rx - sx).abs() < 1e-3 && (ry - sy).abs() < 1e-3,
                    "{size:?}: ({sx},{sy}) -> {p:?} -> ({rx},{ry})");
            }
        }
    }

    /// Backdrop placement in stage coordinates depends only on the texture
    /// aspect, never on the window: a wide backdrop overflows the stage
    /// horizontally (clipped at draw time), a tall one vertically, and the
    /// placement is symmetric about the stage centre.
    #[test]
    fn backdrop_placement_is_window_independent() {
        // 2:1 backdrop against the 880:470 stage: cover on height.
        let (x0, y0, x1, y1) = backdrop_stage_rect(1000.0, 500.0).unwrap();
        assert!((y0 - 0.0).abs() < 1e-3 && (y1 - 470.0).abs() < 1e-3);
        let w = 470.0 * 1000.0 / 500.0; // 940 > 880: symmetric overflow
        assert!((x0 - (880.0 - w) / 2.0).abs() < 1e-3);
        assert!((x1 - (880.0 + w) / 2.0).abs() < 1e-3);
        // Tall backdrop: cover on width, vertical overflow.
        let (x0, y0, x1, y1) = backdrop_stage_rect(880.0, 940.0).unwrap();
        assert!((x0 - 0.0).abs() < 1e-3 && (x1 - 880.0).abs() < 1e-3);
        assert!(y0 < 0.0 && y1 > 470.0 && ((y0 + y1) / 2.0 - 235.0).abs() < 1e-3);
        // Degenerate textures place nothing.
        assert!(backdrop_stage_rect(0.0, 500.0).is_none());
        assert!(backdrop_stage_rect(500.0, -1.0).is_none());
    }

    /// The logical viewport (stage_rect) is contained in the widget and
    /// centred — the neutral-fill margins are symmetric.
    #[test]
    fn stage_rect_centred_in_widget() {
        for size in [TALL, WIDE, MATCH] {
            let map = map_for(size);
            let sr = map.stage_rect();
            assert!(sr.width() <= size.0 + 1e-3 && sr.height() <= size.1 + 1e-3);
            assert!((sr.center().x - size.0 / 2.0).abs() < 1e-3);
            assert!((sr.center().y - size.1 / 2.0).abs() < 1e-3);
            // At matching aspect the stage fills the widget exactly.
            if size == MATCH {
                assert!((sr.width() - 880.0).abs() < 1e-3);
                assert!((sr.height() - 470.0).abs() < 1e-3);
            }
        }
    }
}

/// Points approximating an ellipse (convex polygon for the shadow).
fn ellipse_points(center: egui::Pos2, rx: f32, ry: f32) -> Vec<egui::Pos2> {
    (0..24)
        .map(|i| {
            let a = i as f32 / 24.0 * std::f32::consts::TAU;
            egui::pos2(center.x + a.cos() * rx, center.y + a.sin() * ry)
        })
        .collect()
}
