//! ADDITION (not in VellumFE): the painted backdrop standing IN FRONT of
//! creatures -- a rock, a cactus, a tree trunk.
//!
//! The artist paints white over those things in a mask beside the backdrop,
//! `scenes/<name>.occluders.png` (any size: it is read in the backdrop's
//! own 0..1 coordinates; white or opaque = occluder, black or transparent =
//! not). Each connected blob becomes one occluder, and its DEPTH is where
//! its lowest pixel meets the floor (`CreatureField::ground_from_screen`):
//! a rock whose base sits at depth 3 covers every creature standing behind
//! depth 3 and none in front of it. No depth painting, no numbers.
//!
//! An occluder is drawn by drawing the BACKDROP again, cut to the blob's
//! shape (row spans as a textured mesh), in its turn among the cards --
//! the same way scenery props slot into the painter's order. It uses
//! whatever texture the backdrop slot holds, so a lit backdrop's occluders
//! are lit with it.
//!
//! Hooks in `creature_field.rs` (the marked regions that also draw lamps):
//! [`Prepared::new`] after the backdrop, [`paint_behind`] before each card
//! or prop and after the last, and [`label_painter`] for the part of a card
//! that labels it.

use std::cell::RefCell;

use egui::{pos2, Color32, Pos2, Rect};

/// Blobs smaller than this many mask pixels are specks, not occluders.
const MIN_BLOB_PX: usize = 40;

/// One occluder, in backdrop 0..1 coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct Blob {
    /// Where it stands: the middle of its lowest row.
    pub foot: [f32; 2],
    /// Its shape as `[u0, v0, u1, v1]` rectangles (row runs, merged down
    /// the rows while they match).
    pub rects: Vec<[f32; 4]>,
    /// Mask pixels it covers.
    pub area: usize,
}

/// Every occluder a mask holds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Occluders {
    pub blobs: Vec<Blob>,
}

/// `scenes/desert.jpg` -> `scenes/desert.occluders.png`.
pub fn mask_path(background: &str) -> String {
    let stem = background.rsplit_once('.').map_or(background, |(s, _)| s);
    format!("{stem}.occluders.png")
}

impl Occluders {
    /// Find the blobs in a straight-alpha RGBA mask.
    pub fn from_mask(w: usize, h: usize, rgba: &[u8]) -> Occluders {
        let on: Vec<bool> = rgba.chunks_exact(4).map(|p| p[3] > 127 && p[0].max(p[1]).max(p[2]) > 127).collect();
        let mut label = vec![u32::MAX; w * h];
        let mut blobs = Vec::new();
        for start in 0..w * h {
            if !on[start] || label[start] != u32::MAX {
                continue;
            }
            // Flood fill, 8-connected.
            let id = blobs.len() as u32;
            let mut stack = vec![start];
            let mut pixels = Vec::new();
            label[start] = id;
            while let Some(i) = stack.pop() {
                pixels.push(i);
                let (x, y) = ((i % w) as i64, (i / w) as i64);
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let (nx, ny) = (x + dx, y + dy);
                        if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                            continue;
                        }
                        let j = ny as usize * w + nx as usize;
                        if on[j] && label[j] == u32::MAX {
                            label[j] = id;
                            stack.push(j);
                        }
                    }
                }
            }
            blobs.push(pixels);
        }
        let (fw, fh) = (w as f32, h as f32);
        let blobs = blobs
            .into_iter()
            .enumerate()
            .filter(|(_, p)| p.len() >= MIN_BLOB_PX)
            .map(|(id, pixels)| {
                let id = id as u32;
                let (mut x0, mut x1, mut y0, mut y1) = (usize::MAX, 0, usize::MAX, 0);
                for &i in &pixels {
                    let (x, y) = (i % w, i / w);
                    (x0, x1, y0, y1) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
                }
                // Row runs, merged with the run directly above when equal.
                let mut rects: Vec<[usize; 4]> = Vec::new();
                let mut open: Vec<usize> = Vec::new(); // rects ending on the previous row
                for y in y0..=y1 {
                    let mut runs = Vec::new();
                    let mut x = x0;
                    while x <= x1 {
                        if label[y * w + x] == id {
                            let s = x;
                            while x <= x1 && label[y * w + x] == id {
                                x += 1;
                            }
                            runs.push((s, x));
                        } else {
                            x += 1;
                        }
                    }
                    let mut next_open = Vec::new();
                    for (s, e) in runs {
                        match open.iter().find(|&&k| rects[k][0] == s && rects[k][2] == e) {
                            Some(&k) => {
                                rects[k][3] = y + 1;
                                next_open.push(k);
                            }
                            None => {
                                rects.push([s, y, e, y + 1]);
                                next_open.push(rects.len() - 1);
                            }
                        }
                    }
                    open = next_open;
                }
                let bottom: Vec<usize> = pixels.iter().filter(|&&i| i / w == y1).map(|&i| i % w).collect();
                let foot_x = bottom.iter().sum::<usize>() as f32 / bottom.len().max(1) as f32 + 0.5;
                Blob {
                    foot: [foot_x / fw, (y1 as f32 + 1.0) / fh],
                    rects: rects.iter().map(|r| [r[0] as f32 / fw, r[1] as f32 / fh, r[2] as f32 / fw, r[3] as f32 / fh]).collect(),
                    area: pixels.len(),
                }
            })
            .collect();
        Occluders { blobs }
    }

    /// Decode a mask file's bytes.
    pub fn from_png(bytes: &[u8]) -> Result<Occluders, String> {
        let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?.to_rgba8();
        Ok(Occluders::from_mask(img.width() as usize, img.height() as usize, img.as_raw()))
    }
}

/// This frame's occluders, placed: the backdrop texture, and each blob's
/// depth and stage-space rectangles.
pub struct Prepared {
    texture: egui::TextureId,
    blobs: Vec<(f32, Vec<([f32; 4], [f32; 4])>)>,
}

thread_local! {
    /// Which of this frame's occluders are drawn already.
    static DRAWN: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
    /// Show every occluder tinted (the Studio's check).
    static SHOW: RefCell<bool> = const { RefCell::new(false) };
    /// This frame's scene has occluders (card labels then go on top).
    static ANY: RefCell<bool> = const { RefCell::new(false) };
}

/// Where a card paints its labels -- HP bar, name, status badges and icons,
/// stun stars, target marker. When the scene has occluders: a layer drawn
/// directly above the field's, so an occluder never hides who is behind it
/// (labels then also sit above nearer creatures' bodies). Otherwise the
/// field's own painter, exactly as VellumFE draws.
pub fn label_painter(painter: &egui::Painter) -> egui::Painter {
    if !ANY.with(|a| *a.borrow()) {
        return painter.clone();
    }
    let parent = painter.layer_id();
    let layer = egui::LayerId::new(parent.order, parent.id.with("gs_field_egui::card_labels"));
    painter.ctx().set_sublayer(parent, layer);
    painter.clone().with_layer_id(layer)
}

fn show_id() -> egui::Id {
    egui::Id::new("gs_field_egui::show_occluders")
}

/// Tint the occluders so an artist can see what the mask made (a host
/// setting, off by default).
pub fn set_show_occluders(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(show_id(), on));
}

impl Prepared {
    /// After the backdrop is drawn: place this scene's occluders, if its
    /// backdrop has a mask. `backdrop` is the backdrop's stage rectangle
    /// `(x0, y0, x1, y1)`.
    pub fn new(
        ctx: &egui::Context,
        scene: Option<&gs_field::scenes::StageScene>,
        art_cache: Option<&crate::creature_art::CreatureArtCache>,
        backdrop: &dyn Fn(f32, f32) -> Option<(f32, f32, f32, f32)>,
        field: &gs_field::solver::CreatureField,
    ) -> Option<Prepared> {
        DRAWN.with(|d| d.borrow_mut().clear());
        ANY.with(|a| *a.borrow_mut() = false);
        SHOW.with(|s| *s.borrow_mut() = ctx.data(|d| d.get_temp::<bool>(show_id())).unwrap_or(false));
        let bg = scene?.background.as_deref()?;
        let cache = art_cache?;
        let occluders = cache.occluders(bg)?;
        let texture = cache.scene_background(bg)?;
        let ts = texture.size_vec2();
        let (x0, y0, x1, y1) = backdrop(ts.x, ts.y)?;
        let at = |u: f32, v: f32| (x0 + u * (x1 - x0), y0 + v * (y1 - y0));
        let blobs: Vec<_> = occluders
            .blobs
            .iter()
            .map(|b| {
                let (fx, fy) = at(b.foot[0], b.foot[1]);
                let (_, z) = field.ground_from_screen(fx, fy);
                let rects = b
                    .rects
                    .iter()
                    .map(|r| {
                        let (a, c) = (at(r[0], r[1]), at(r[2], r[3]));
                        ([a.0, a.1, c.0, c.1], *r)
                    })
                    .collect();
                (z, rects)
            })
            .collect();
        DRAWN.with(|d| d.borrow_mut().resize(blobs.len(), false));
        ANY.with(|a| *a.borrow_mut() = !blobs.is_empty());
        Some(Prepared { texture: texture.id(), blobs })
    }

    /// The occluders not yet drawn standing at least as far away as `z`,
    /// far to near, marked drawn.
    fn due(&self, z: f32) -> Vec<usize> {
        DRAWN.with(|d| {
            let mut d = d.borrow_mut();
            d.resize(self.blobs.len(), false);
            let mut due: Vec<usize> = (0..self.blobs.len()).filter(|&k| !d[k] && self.blobs[k].0 >= z).collect();
            due.sort_by(|&a, &b| self.blobs[b].0.total_cmp(&self.blobs[a].0));
            for &k in &due {
                d[k] = true;
            }
            due
        })
    }

    fn paint(&self, painter: &egui::Painter, to_screen: &dyn Fn(f32, f32) -> Pos2, k: usize) {
        let mut mesh = egui::Mesh::with_texture(self.texture);
        for (s, uv) in &self.blobs[k].1 {
            mesh.add_rect_with_uv(
                Rect::from_min_max(to_screen(s[0], s[1]), to_screen(s[2], s[3])),
                Rect::from_min_max(pos2(uv[0], uv[1]), pos2(uv[2], uv[3])),
                Color32::WHITE,
            );
        }
        painter.add(egui::Shape::mesh(mesh));
        if SHOW.with(|s| *s.borrow()) {
            for (s, _) in &self.blobs[k].1 {
                painter.rect_filled(Rect::from_min_max(to_screen(s[0], s[1]), to_screen(s[2], s[3])), 0.0, Color32::from_rgba_unmultiplied(255, 60, 60, 70));
            }
        }
    }
}

/// Before a card or prop at depth `z` (and with `z` = -inf after the last):
/// the occluders and lamps standing at least as far away, far to near, so
/// the card covers them and they cover whatever stands behind them.
#[allow(clippy::too_many_arguments)]
pub fn paint_behind(
    painter: &egui::Painter,
    to_screen: &dyn Fn(f32, f32) -> Pos2,
    scale: f32,
    prepared: Option<&Prepared>,
    field: &gs_field::solver::CreatureField,
    scene: Option<&gs_field::scenes::StageScene>,
    z: f32,
    ctx: &egui::Context,
) {
    let mut due: Vec<(f32, Option<usize>, Option<usize>)> = Vec::new();
    if let Some(p) = prepared {
        due.extend(p.due(z).into_iter().map(|k| (p.blobs[k].0, Some(k), None)));
    }
    let lights = scene.map(|s| s.lights.as_slice()).unwrap_or_default();
    if !lights.is_empty() {
        due.extend(crate::field_lights::lamps_due(lights, z).into_iter().map(|k| (lights[k].z, None, Some(k))));
    }
    // Far first; on a tie the occluder, so a lamp at a rock's foot shows.
    due.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.is_some().cmp(&a.1.is_some())));
    let t = ctx.input(|i| i.time) as f32;
    for (_, occluder, lamp) in due {
        if let (Some(k), Some(p)) = (occluder, prepared) {
            p.paint(painter, to_screen, k);
        }
        if let Some(k) = lamp {
            crate::field_lights::paint_lamp(painter, field, to_screen, scale, &lights[k], k, t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mask(w: usize, h: usize, on: &dyn Fn(usize, usize) -> bool) -> Vec<u8> {
        (0..w * h).flat_map(|i| if on(i % w, i / w) { [255u8; 4] } else { [0, 0, 0, 255] }).collect()
    }

    #[test]
    fn blobs_are_found_with_their_feet_and_specks_are_dropped() {
        // A 10x20 block standing on row 39, a triangle-ish blob, a 2px speck.
        let (w, h) = (100, 50);
        let rgba = mask(w, h, &|x, y| {
            (10..20).contains(&x) && (20..40).contains(&y) || (y >= 30 && y < 45 && x >= 60 && x < 60 + (y - 29)) || (x == 90 && y < 2)
        });
        let occ = Occluders::from_mask(w, h, &rgba);
        assert_eq!(occ.blobs.len(), 2, "{:?}", occ.blobs.iter().map(|b| b.area).collect::<Vec<_>>());
        let block = occ.blobs.iter().find(|b| b.area == 200).unwrap();
        assert_eq!(block.rects.len(), 1, "equal rows merge into one rect");
        assert!((block.foot[0] - 0.15).abs() < 1e-4 && (block.foot[1] - 0.8).abs() < 1e-4, "{:?}", block.foot);
        let tri = occ.blobs.iter().find(|b| b.area != 200).unwrap();
        assert_eq!(tri.rects.len(), 15, "one rect per row of a widening shape");
        // The spans cover exactly the blob.
        let covered: f32 = tri.rects.iter().map(|r| (r[2] - r[0]) * w as f32 * (r[3] - r[1]) * h as f32).sum();
        assert!((covered - tri.area as f32).abs() < 1e-2);
        assert_eq!(mask_path("scenes/desert.jpg"), "scenes/desert.occluders.png");
    }

    #[test]
    fn an_occluder_slots_between_the_cards_by_its_foot() {
        let field = gs_field::solver::CreatureField::default();
        let (z_near, z_far) = field.depth_range();
        let mid = (z_near + z_far) * 0.5;
        // A blob whose foot sits on the floor at depth `mid`.
        let (sx, sy) = field.project_ground(440.0, mid).0;
        let backdrop = |_: f32, _: f32| Some((0.0, 0.0, 880.0, 470.0));
        let blob = Blob { foot: [sx / 880.0, sy / 470.0], rects: vec![[0.4, 0.2, 0.6, sy / 470.0]], area: 1 };
        let (_, z) = field.ground_from_screen(sx, sy);
        assert!((z - mid).abs() < 0.05, "{z} vs {mid}");
        let p = Prepared { texture: egui::TextureId::default(), blobs: vec![(z, vec![([0.0; 4], blob.rects[0])])] };
        let _ = backdrop;
        DRAWN.with(|d| d.borrow_mut().clear());
        assert!(p.due(mid + 1.0).is_empty(), "a card farther away is drawn first, then covered");
        assert_eq!(p.due(mid - 1.0), vec![0], "before a nearer card");
        assert!(p.due(f32::NEG_INFINITY).is_empty(), "once");
    }
}
