//! The creature doll: what a creature with NO sprite art is drawn as — its
//! family's baked label map (`gs_field::dolls`), filled with the creature's
//! placeholder body colour and tinted per body part by wound rank.
//!
//! NOT a port — nothing here exists in VellumFE @60c84a79 (see PORTING.md,
//! "Additions since the port").
//!
//! Three layers, cheapest last:
//!   * [`DollMap`] — one rig's label map, decoded ONCE per rig generation.
//!     Only the PNG's R (label) and A (inside) channels are read; G and B
//!     are reserved by the bake format.
//!   * [`compose_rgba`] — pure CPU: labels + a [`DollLook`] -> RGBA. Tints,
//!     severed limbs, stumps and the silhouette outline all happen here.
//!   * [`DollCache`] — per-`egui::Context` LRU of composed textures keyed by
//!     (rig, look). A texture is rebuilt only when its key changes; a room
//!     of identical healthy kobolds shares one texture.
//!
//! [`DollLayout`] is the rigid posing: standing, squashed (kneeling /
//! sitting), or lying (prone / dead: the same picture turned 90°), each
//! optionally mirrored. It maps map fractions to screen points, so anchors,
//! eye markers and the textured quad all go through one transform.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use gs_field::dolls::{DollMeta, DollRigs};

/// Number of doll parts (labels are 1..=PARTS).
pub const PARTS: usize = 14;

/// How far a wounded part's colour moves from the body colour toward the
/// rank colour. High on purpose: cards are small.
pub const TINT_STRENGTH: f32 = 0.85;

/// Composed textures kept per egui context.
pub const TEXTURE_CACHE_CAP: usize = 64;

/// The boss outline (the old placeholder's gold stroke).
pub const BOSS_GOLD: [u8; 3] = [0xc9, 0xa2, 0x27];

/// One rig's decoded label map.
#[derive(Debug, Clone)]
pub struct DollMap {
    pub width: usize,
    pub height: usize,
    /// Row-major labels: 0 outside, else 1..=14.
    pub labels: Vec<u8>,
    /// `present[label]`: the map has pixels with this label.
    pub present: [bool; PARTS + 1],
    pub meta: DollMeta,
}

impl DollMap {
    /// Decode a baked label-map PNG. R is the label where A says "inside";
    /// every other channel is ignored. Out-of-range labels count as outside.
    pub fn decode(png: &[u8], meta: DollMeta) -> Option<Self> {
        let rgba = image::load_from_memory_with_format(png, image::ImageFormat::Png)
            .ok()?
            .into_rgba8();
        let (width, height) = (rgba.width() as usize, rgba.height() as usize);
        if width == 0 || height == 0 {
            return None;
        }
        let labels = rgba
            .pixels()
            .map(|p| if p[3] >= 128 && p[0] as usize <= PARTS { p[0] } else { 0 })
            .collect();
        Some(Self::from_labels(width, height, labels, meta))
    }

    pub fn from_labels(width: usize, height: usize, labels: Vec<u8>, meta: DollMeta) -> Self {
        assert_eq!(labels.len(), width * height);
        let mut present = [false; PARTS + 1];
        for &l in &labels {
            present[(l as usize).min(PARTS)] |= l != 0;
        }
        present[0] = false;
        Self {
            width,
            height,
            labels,
            present,
            meta,
        }
    }

    /// Does this rig have labelled pixels for `part`?
    pub fn has_part(&self, part: &str) -> bool {
        gs_field::dolls::part_label(part).is_some_and(|l| self.present[l as usize])
    }
}

/// Everything that decides a doll texture's pixels (besides the rig).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DollLook {
    pub body: [u8; 3],
    /// Wound rank per label (index = label - 1), 0 = healthy, clamped 1..=3.
    /// The severed set is a function of this (rank 3 on a limb), so it needs
    /// no key of its own.
    pub ranks: [u8; PARTS],
    pub dead: bool,
    pub boss: bool,
}

impl DollLook {
    pub fn new(body: [u8; 3], injuries: &[(String, u8)], dead: bool, boss: bool) -> Self {
        let mut ranks = [0u8; PARTS];
        for (part, rank) in injuries {
            if let Some(label) = gs_field::dolls::part_label(part) {
                let slot = &mut ranks[label as usize - 1];
                *slot = (*slot).max((*rank).min(3));
            }
        }
        Self {
            body,
            ranks,
            dead,
            boss,
        }
    }

    /// `removed[label]`: this label's pixels are gone (a severed limb, and
    /// the hand that went with an arm).
    pub fn removed(&self) -> [bool; PARTS + 1] {
        let mut removed = [false; PARTS + 1];
        for (i, &rank) in self.ranks.iter().enumerate() {
            let Some(part) = gs_field::dolls::label_part(i as u8 + 1) else {
                continue;
            };
            for gone in gs_field::dolls::severed_parts(part, rank) {
                if let Some(l) = gs_field::dolls::part_label(gone) {
                    removed[l as usize] = true;
                }
            }
        }
        removed
    }
}

fn blend(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    std::array::from_fn(|i| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t).round() as u8)
}

/// Grow `seed` (dist 0) through `open` pixels by 4-neighbour steps; returns
/// each pixel's step distance, 255 = further than `reach` (or not open).
/// With `border`, the image edge counts as a seed (dist 0 just outside).
fn distances(
    w: usize,
    h: usize,
    seed: &dyn Fn(usize) -> bool,
    open: &dyn Fn(usize) -> bool,
    reach: u8,
    border: bool,
) -> Vec<u8> {
    let mut dist = vec![255u8; w * h];
    let mut frontier: Vec<usize> = Vec::new();
    // Level 1 starts with the open pixels on the image edge (their missing
    // neighbour is "outside"); the seeds' own neighbours join them below.
    let mut next: Vec<usize> = Vec::new();
    for (i, d) in dist.iter_mut().enumerate() {
        if seed(i) {
            *d = 0;
            frontier.push(i);
        } else if border && reach >= 1 && open(i) {
            let (x, y) = (i % w, i / w);
            if x == 0 || y == 0 || x == w - 1 || y == h - 1 {
                *d = 1;
                next.push(i);
            }
        }
    }
    // Strict level order: `frontier` holds exactly one distance at a time.
    while !frontier.is_empty() || !next.is_empty() {
        for &i in &frontier {
            let d = dist[i];
            if d >= reach {
                continue;
            }
            let (x, y) = (i % w, i / w);
            let mut visit = |j: usize| {
                if dist[j] == 255 && open(j) {
                    dist[j] = d + 1;
                    next.push(j);
                }
            };
            if x > 0 {
                visit(i - 1);
            }
            if x + 1 < w {
                visit(i + 1);
            }
            if y > 0 {
                visit(i - w);
            }
            if y + 1 < h {
                visit(i + w);
            }
        }
        frontier.clear();
        std::mem::swap(&mut frontier, &mut next);
    }
    dist
}

/// Outline thickness in map texels: ~1 screen px at typical card sizes
/// (a 512-texel doll is drawn ~100-200 px tall). Bosses get a heavier edge.
fn outline_texels(map_h: usize, boss: bool) -> u8 {
    let t = (map_h as f32 / 128.0).round().max(1.0);
    (if boss { t * 2.0 } else { t }).min(40.0) as u8
}

/// Stump band thickness in map texels, measured from the cut. It reaches
/// past the outline (which is drawn over it) so the same width of red shows
/// on a boss's heavier edge as on anyone else's.
fn stump_texels(map_h: usize, boss: bool) -> u8 {
    let band = (map_h as f32 / 48.0).round().clamp(2.0, 40.0) as u8;
    band + outline_texels(map_h, boss)
}

/// Labels + look -> straight (unmultiplied) RGBA, row-major.
///
/// * inside pixels take the body colour; a wounded part blends
///   [`TINT_STRENGTH`] of the way to its rank colour (the player injury
///   doll's palette, `gs_field::dolls::DEFAULT_INJURY_PALETTE` 1-3);
/// * a severed limb's pixels (and an arm's hand) become transparent, and the
///   band of the adjoining part that touches them — chest for an arm,
///   abdomen for a leg, else whatever is there — takes the rank-3 colour;
/// * the silhouette's edge (computed AFTER removal, so a stump is outlined
///   too) is darkened, or gold for a boss, so tints read on any backdrop.
pub fn compose_rgba(map: &DollMap, look: &DollLook) -> Vec<u8> {
    let (w, h) = (map.width, map.height);
    let labels = &map.labels;
    let removed = look.removed();
    let visible = |i: usize| labels[i] != 0 && !removed[labels[i] as usize];

    let mut colors = [look.body; PARTS + 1];
    for (i, &rank) in look.ranks.iter().enumerate() {
        if rank > 0 {
            colors[i + 1] = blend(
                look.body,
                gs_field::dolls::injury_rank_rgb(rank),
                TINT_STRENGTH,
            );
        }
    }

    // Stumps: only when something is actually severed (the common case
    // skips this pass entirely).
    let stump = if removed.iter().any(|r| *r) {
        let label_of = |part: &str| gs_field::dolls::part_label(part).unwrap_or(0) as usize;
        // Which labels may carry a stump: the anatomical neighbour when the
        // rig has it (so a hanging arm does not bloody the hip it brushes).
        let mut hosts = [false; PARTS + 1];
        let mut any_host = false;
        for (limb, host) in [
            ("leftArm", "chest"),
            ("rightArm", "chest"),
            ("leftLeg", "abdomen"),
            ("rightLeg", "abdomen"),
        ] {
            if removed[label_of(limb)] && map.present[label_of(host)] {
                hosts[label_of(host)] = true;
                any_host = true;
            }
        }
        if !any_host {
            hosts = [true; PARTS + 1];
        }
        Some(distances(
            w,
            h,
            &|i| labels[i] != 0 && removed[labels[i] as usize],
            &|i| visible(i) && hosts[labels[i] as usize],
            stump_texels(h, look.boss),
            false,
        ))
    } else {
        None
    };
    let stump_color = gs_field::dolls::injury_rank_rgb(3);

    let edge = distances(w, h, &|i| !visible(i), &visible, outline_texels(h, look.boss), true);

    let mut out = vec![0u8; w * h * 4];
    for i in 0..w * h {
        if !visible(i) {
            continue;
        }
        let mut c = colors[labels[i] as usize];
        if stump.as_ref().is_some_and(|d| d[i] != 255) {
            c = stump_color;
        }
        if edge[i] != 255 {
            c = if look.boss {
                BOSS_GOLD
            } else {
                blend(c, [0, 0, 0], 0.55)
            };
        }
        out[i * 4..i * 4 + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
    }
    out
}

// ---- posing -------------------------------------------------------------

/// The rigid poses (no articulation yet).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DollPose {
    Standing,
    /// Kneeling / sitting: squashed toward the ground to this fraction of
    /// the standing height (the old placeholder's 0.45 rule -> 0.55).
    Squashed(f32),
    /// Prone / dead: the picture turned 90°, lying along the ground.
    Lying { head_left: bool },
}

/// Where a posed doll sits on screen. `body` is the screen rect the (posed)
/// picture fills; [`DollLayout::pt`] carries map fractions into it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DollLayout {
    pub body: egui::Rect,
    pub pose: DollPose,
    /// Mirrored left-right (facing variety). Labels travel with the pixels.
    pub mirror: bool,
}

impl DollLayout {
    /// Fit a doll the way the sprite branch fits art: the map's ground row
    /// lands on the card's foot line, the content (top..ground) spans the
    /// unit's STANDING height in pixels, and the feet anchor's x sits on the
    /// card centre. A lying doll is the same picture turned 90°, resting on
    /// the foot line and centred on the card.
    pub fn fit(
        meta: &DollMeta,
        map_size: (usize, usize),
        card: egui::Rect,
        standing_px: f32,
        pose: DollPose,
        mirror: bool,
    ) -> Self {
        let ground = meta.ground_frac.clamp(0.05, 1.0);
        let draw_h = standing_px / ground;
        let draw_w = draw_h * map_size.0 as f32 / (map_size.1 as f32).max(1.0);
        let feet = meta.feet();
        let feet_x = if mirror { 1.0 - feet[0] } else { feet[0] };
        let body = match pose {
            DollPose::Standing | DollPose::Squashed(_) => {
                let k = match pose {
                    DollPose::Squashed(k) => k.clamp(0.05, 1.0),
                    _ => 1.0,
                };
                let h = draw_h * k;
                egui::Rect::from_min_size(
                    egui::pos2(card.center().x - feet_x * draw_w, card.bottom() - feet[1] * h),
                    egui::vec2(draw_w, h),
                )
            }
            DollPose::Lying { .. } => egui::Rect::from_min_size(
                egui::pos2(card.center().x - draw_h / 2.0, card.bottom() - draw_w),
                egui::vec2(draw_h, draw_w),
            ),
        };
        Self { body, pose, mirror }
    }

    /// Map fraction (x right, y down) -> fraction of `body`.
    pub fn frac(&self, p: [f32; 2]) -> [f32; 2] {
        let u = if self.mirror { 1.0 - p[0] } else { p[0] };
        let v = p[1];
        match self.pose {
            DollPose::Standing | DollPose::Squashed(_) => [u, v],
            // Turned counter-clockwise: up -> left, right -> up.
            DollPose::Lying { head_left: true } => [v, 1.0 - u],
            // Turned clockwise: up -> right, right -> down.
            DollPose::Lying { head_left: false } => [1.0 - v, u],
        }
    }

    /// Map fraction -> screen point.
    pub fn pt(&self, p: [f32; 2]) -> egui::Pos2 {
        let f = self.frac(p);
        egui::pos2(
            self.body.left() + f[0] * self.body.width(),
            self.body.top() + f[1] * self.body.height(),
        )
    }

    /// The textured quad for this layout.
    pub fn mesh(&self, texture: egui::TextureId, tint: egui::Color32) -> egui::Mesh {
        let mut mesh = egui::Mesh::with_texture(texture);
        for uv in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] {
            mesh.vertices.push(egui::epaint::Vertex {
                pos: self.pt(uv),
                uv: egui::pos2(uv[0], uv[1]),
                color: tint,
            });
        }
        mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
        mesh
    }
}

/// Stable per-creature variety bits from the exist id (FNV-1a, avalanche
/// finish so sequential ids do not alternate in lockstep): `(mirror,
/// falls_head_left)`.
pub fn variety(id: &str) -> (bool, bool) {
    let mut h: u32 = 2166136261;
    for b in id.trim().trim_start_matches('#').bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    ((h >> 5) & 1 == 1, (h >> 9) & 1 == 1)
}

// ---- the cache ----------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TextureKey {
    family: Arc<str>,
    look: DollLook,
}

/// A doll ready to draw.
#[derive(Clone)]
pub struct DollTexture {
    pub texture: egui::TextureHandle,
    pub map: Arc<DollMap>,
}

/// Per-context doll state: decoded maps, the creature -> rig resolutions,
/// and the LRU of composed textures. Everything is dropped when the
/// published rig set changes generation (`gs_field::dolls::invalidate_cache`
/// or a moved base dir).
#[derive(Default)]
pub struct DollCache {
    generation: u64,
    frame: Option<u64>,
    rigs: Option<Arc<DollRigs>>,
    maps: HashMap<Arc<str>, Option<Arc<DollMap>>>,
    resolved: HashMap<u64, Option<Arc<str>>>,
    textures: HashMap<TextureKey, (egui::TextureHandle, u64)>,
    tick: u64,
    /// Textures composed since this cache was created (tests, diagnostics).
    pub builds: u64,
}

pub type SharedDollCache = Arc<Mutex<DollCache>>;

impl DollCache {
    /// The cache that lives in this egui context (created on first use).
    pub fn shared(ctx: &egui::Context) -> SharedDollCache {
        ctx.data_mut(|d| {
            d.get_temp_mut_or_default::<SharedDollCache>(egui::Id::new("gs_field_egui::dolls"))
                .clone()
        })
    }

    /// Pick up the current rig set, dropping everything derived from an
    /// older one. Does the work once per `frame` (pass
    /// `ctx.cumulative_frame_nr()`); further calls that frame are a compare.
    pub fn begin_frame(&mut self, frame: u64) {
        if self.rigs.is_some() && self.frame == Some(frame) {
            return;
        }
        self.frame = Some(frame);
        let rigs = gs_field::dolls::rigs();
        if self.rigs.is_none() || self.generation != rigs.generation() {
            self.generation = rigs.generation();
            self.maps.clear();
            self.resolved.clear();
            self.textures.clear();
        }
        self.rigs = Some(rigs);
    }

    /// The rig family a creature draws with (cached cascade result).
    pub fn family_for(&mut self, name: &str, noun: Option<&str>) -> Option<Arc<str>> {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (name, noun).hash(&mut hasher);
        let key = hasher.finish();
        if let Some(hit) = self.resolved.get(&key) {
            return hit.clone();
        }
        let rigs = self.rigs.clone().unwrap_or_else(gs_field::dolls::rigs);
        let family = rigs
            .doll_for(name, noun)
            .map(|d| Arc::<str>::from(d.family.as_str()));
        self.resolved.insert(key, family.clone());
        family
    }

    /// The decoded label map for a family (decoded once; misses cached).
    pub fn map(&mut self, family: &Arc<str>) -> Option<Arc<DollMap>> {
        if let Some(hit) = self.maps.get(family) {
            return hit.clone();
        }
        let rigs = self.rigs.clone().unwrap_or_else(gs_field::dolls::rigs);
        let map = rigs.get(family).and_then(|rig| {
            let map = DollMap::decode(&rig.png, rig.meta.clone());
            if map.is_none() {
                tracing::warn!("doll rig '{family}': label map did not decode");
            }
            map.map(Arc::new)
        });
        self.maps.insert(family.clone(), map.clone());
        map
    }

    /// The composed texture for (family, look); built only on a key miss.
    pub fn texture(
        &mut self,
        ctx: &egui::Context,
        family: &Arc<str>,
        look: &DollLook,
    ) -> Option<DollTexture> {
        let map = self.map(family)?;
        self.tick += 1;
        let tick = self.tick;
        let key = TextureKey {
            family: family.clone(),
            look: *look,
        };
        if let Some((texture, used)) = self.textures.get_mut(&key) {
            *used = tick;
            return Some(DollTexture {
                texture: texture.clone(),
                map,
            });
        }
        let rgba = compose_rgba(&map, look);
        let image = egui::ColorImage::from_rgba_unmultiplied([map.width, map.height], &rgba);
        // Dolls are drawn far below map resolution: mipmaps keep the thin
        // outline from shimmering.
        let options = egui::TextureOptions {
            mipmap_mode: Some(egui::TextureFilter::Linear),
            ..egui::TextureOptions::LINEAR
        };
        let texture = ctx.load_texture(format!("doll:{family}"), image, options);
        self.builds += 1;
        if self.textures.len() >= TEXTURE_CACHE_CAP {
            if let Some(oldest) = self
                .textures
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| k.clone())
            {
                self.textures.remove(&oldest);
            }
        }
        self.textures.insert(key, (texture.clone(), tick));
        Some(DollTexture { texture, map })
    }

    pub fn texture_count(&self) -> usize {
        self.textures.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn humanoid() -> DollMap {
        let rigs = DollRigs::from_rigs(gs_field::dolls::embedded().map(|(family, png, meta)| {
            gs_field::dolls::DollRig {
                family: family.to_string(),
                meta,
                png: png.into(),
                source: gs_field::dolls::DollSource::Embedded,
            }
        }));
        let rig = rigs.get("humanoid").expect("humanoid embedded");
        DollMap::decode(&rig.png, rig.meta.clone()).expect("decodes")
    }

    fn look(injuries: &[(&str, u8)]) -> DollLook {
        let injuries: Vec<(String, u8)> =
            injuries.iter().map(|(p, r)| (p.to_string(), *r)).collect();
        DollLook::new([100, 120, 140], &injuries, false, false)
    }

    fn px(rgba: &[u8], i: usize) -> [u8; 4] {
        [rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2], rgba[i * 4 + 3]]
    }

    /// Interior pixels (not outline, not stump) of one label.
    fn interior(map: &DollMap, rgba: &[u8], part: &str, body_or_tint: [u8; 3]) -> usize {
        let label = gs_field::dolls::part_label(part).unwrap();
        (0..map.labels.len())
            .filter(|&i| map.labels[i] == label && px(rgba, i) == [body_or_tint[0], body_or_tint[1], body_or_tint[2], 255])
            .count()
    }

    #[test]
    fn humanoid_map_decodes_with_the_parts_the_toml_lists() {
        let map = humanoid();
        assert_eq!((map.width, map.height), (182, 512));
        for part in &map.meta.parts {
            assert!(map.has_part(part), "{part} has pixels");
        }
        // Eye discs are baked onto the head; a front view has no back, and
        // nsys never has a region.
        for part in ["leftEye", "rightEye"] {
            assert!(map.has_part(part), "{part} is baked onto the head");
        }
        for part in ["back", "nsys"] {
            assert!(!map.has_part(part), "{part} has no region on this rig");
        }
        assert!(!map.has_part("tail"));
    }

    /// The G channel is reserved (a bone index in newer bakes): a map whose
    /// G/B are garbage decodes to the same labels.
    #[test]
    fn only_r_and_a_are_read() {
        let base = humanoid();
        let mut img = image::RgbaImage::new(base.width as u32, base.height as u32);
        for (i, p) in img.pixels_mut().enumerate() {
            let l = base.labels[i];
            *p = image::Rgba([l, (i % 251) as u8, 77, if l == 0 { 0 } else { 255 }]);
        }
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let again = DollMap::decode(&png, base.meta.clone()).unwrap();
        assert_eq!(again.labels, base.labels);
    }

    #[test]
    fn healthy_doll_is_body_colour_inside_an_outline() {
        let map = humanoid();
        let rgba = compose_rgba(&map, &look(&[]));
        let mut body = 0;
        let mut outline = 0;
        for i in 0..map.labels.len() {
            let p = px(&rgba, i);
            if map.labels[i] == 0 {
                assert_eq!(p, [0, 0, 0, 0]);
            } else if p == [100, 120, 140, 255] {
                body += 1;
            } else {
                assert_eq!(p, [45, 54, 63, 255], "darkened body = outline");
                outline += 1;
            }
        }
        assert!(outline > 500 && body > outline * 3, "body {body} outline {outline}");
    }

    #[test]
    fn a_wound_tints_its_part_only_with_the_player_palette() {
        let map = humanoid();
        let rgba = compose_rgba(&map, &look(&[("chest", 2)]));
        let tint = blend([100, 120, 140], [0xff, 0x88, 0x00], TINT_STRENGTH);
        assert!(interior(&map, &rgba, "chest", tint) > 500);
        assert_eq!(interior(&map, &rgba, "chest", [100, 120, 140]), 0);
        assert_eq!(interior(&map, &rgba, "abdomen", tint), 0);
        assert!(interior(&map, &rgba, "abdomen", [100, 120, 140]) > 500);
        // Rank 3 on the head tints; nothing is removed.
        let rgba = compose_rgba(&map, &look(&[("head", 3)]));
        let red = blend([100, 120, 140], [0xff, 0, 0], TINT_STRENGTH);
        assert!(interior(&map, &rgba, "head", red) > 500);
        // The worst rank wins; ranks clamp to 3.
        assert_eq!(look(&[("head", 1), ("HEAD", 9)]).ranks[0], 3);
    }

    #[test]
    fn a_severed_arm_takes_its_hand_and_leaves_a_red_stump_on_the_chest() {
        let map = humanoid();
        let l = look(&[("rightArm", 3)]);
        let rgba = compose_rgba(&map, &l);
        let label = |p| gs_field::dolls::part_label(p).unwrap();
        let (arm, hand, chest, other_arm) =
            (label("rightArm"), label("rightHand"), label("chest"), label("leftArm"));
        let mut stump = 0;
        for i in 0..map.labels.len() {
            let p = px(&rgba, i);
            let lab = map.labels[i];
            if lab == arm || lab == hand {
                assert_eq!(p[3], 0, "severed pixels are transparent");
            } else if lab != 0 {
                assert_eq!(p[3], 255);
                if [p[0], p[1], p[2]] == [0xff, 0, 0] {
                    assert_eq!(lab, chest, "the stump sits on the chest only");
                    stump += 1;
                }
            }
            if lab == other_arm {
                assert_eq!(p[3], 255, "the other arm stays");
            }
        }
        assert!(stump > 20, "stump band painted ({stump} px)");
        // Rank 2 on the same arm: nothing removed, just tinted.
        let rgba = compose_rgba(&map, &look(&[("rightArm", 2)]));
        assert!((0..map.labels.len()).all(|i| (map.labels[i] == 0) == (px(&rgba, i)[3] == 0)));
    }

    #[test]
    fn a_boss_gets_a_gold_silhouette_edge() {
        let map = humanoid();
        let mut l = look(&[]);
        l.boss = true;
        let rgba = compose_rgba(&map, &l);
        let gold = (0..map.labels.len())
            .filter(|&i| px(&rgba, i) == [BOSS_GOLD[0], BOSS_GOLD[1], BOSS_GOLD[2], 255])
            .count();
        let plain = compose_rgba(&map, &look(&[]));
        let dark = (0..map.labels.len())
            .filter(|&i| px(&plain, i) == [45, 54, 63, 255])
            .count();
        assert!(gold > dark, "heavier than the normal edge: {gold} vs {dark}");
    }

    fn card() -> egui::Rect {
        egui::Rect::from_min_max(egui::pos2(100.0, 50.0), egui::pos2(160.0, 250.0))
    }

    #[test]
    fn standing_layout_puts_feet_on_the_foot_line_at_standing_height() {
        let map = humanoid();
        let size = (map.width, map.height);
        for mirror in [false, true] {
            let lay = DollLayout::fit(&map.meta, size, card(), 200.0, DollPose::Standing, mirror);
            let feet = lay.pt(map.meta.feet());
            assert!((feet.x - 130.0).abs() < 1e-3 && (feet.y - 250.0).abs() < 1e-3, "{feet:?}");
            // Content top..ground = the standing height.
            assert!((250.0 - lay.body.top() - 200.0).abs() < 0.5);
            let head = lay.pt(map.meta.anchor("head").unwrap());
            assert!(head.y < 60.0 && (head.x - 130.0).abs() < 1.0);
            // An off-centre point mirrors about the feet.
            let eye = lay.pt(map.meta.eye("leftEye").unwrap());
            assert_eq!(eye.x > 130.0, !mirror);
        }
    }

    #[test]
    fn squashed_layout_keeps_the_ground_and_lying_turns_the_picture() {
        let map = humanoid();
        let size = (map.width, map.height);
        let stand = DollLayout::fit(&map.meta, size, card(), 200.0, DollPose::Standing, false);
        let kneel = DollLayout::fit(&map.meta, size, card(), 200.0, DollPose::Squashed(0.55), false);
        assert!((kneel.body.height() - stand.body.height() * 0.55).abs() < 1e-3);
        assert!((kneel.pt(map.meta.feet()).y - 250.0).abs() < 1e-3);
        assert_eq!(kneel.body.width(), stand.body.width());

        for head_left in [true, false] {
            let lie = DollLayout::fit(&map.meta, size, card(), 200.0, DollPose::Lying { head_left }, false);
            // Length along the ground, thickness up; resting on the foot line.
            assert!((lie.body.width() - stand.body.height()).abs() < 1e-3);
            assert!((lie.body.height() - stand.body.width()).abs() < 1e-3);
            assert!((lie.body.bottom() - 250.0).abs() < 1e-3);
            assert!((lie.body.center().x - 130.0).abs() < 1e-3);
            let head = lie.pt(map.meta.anchor("head").unwrap());
            let feet = lie.pt(map.meta.feet());
            assert_eq!(head.x < feet.x, head_left);
            // The quad's corners are the body rect's corners.
            let mesh = lie.mesh(egui::TextureId::default(), egui::Color32::WHITE);
            for v in &mesh.vertices {
                assert!(lie.body.expand(1e-3).contains(v.pos));
            }
        }
    }

    #[test]
    fn variety_is_stable_and_mixed() {
        assert_eq!(variety("#1001"), variety("1001"));
        let ids: Vec<String> = (1000..1064).map(|n| n.to_string()).collect();
        let mirrored = ids.iter().filter(|id| variety(id).0).count();
        let left = ids.iter().filter(|id| variety(id).1).count();
        assert!((16..=48).contains(&mirrored), "mirror split {mirrored}/64");
        assert!((16..=48).contains(&left), "fall split {left}/64");
    }

    /// The LRU: same key = same texture, never rebuilt; a changed look
    /// builds once; the cap evicts the least recently used.
    #[test]
    fn texture_cache_builds_once_per_key_and_is_capped() {
        // Other tests re-point VELLUM_FE_DIR, which reloads the rig set.
        let _guard = gs_field::data_dirs::VELLUM_FE_DIR_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let ctx = egui::Context::default();
        let shared = DollCache::shared(&ctx);
        assert!(Arc::ptr_eq(&shared, &DollCache::shared(&ctx)), "one per context");
        let mut cache = shared.lock().unwrap();
        cache.begin_frame(0);
        let family = cache.family_for("mongrel kobold", Some("kobold")).expect("humanoid exists");
        assert_eq!(&*family, "humanoid");

        let healthy = look(&[]);
        let a = cache.texture(&ctx, &family, &healthy).unwrap();
        for frame in 1..=50 {
            cache.begin_frame(frame);
            let b = cache.texture(&ctx, &family, &healthy).unwrap();
            assert_eq!(a.texture.id(), b.texture.id());
        }
        assert_eq!(cache.builds, 1);
        cache.texture(&ctx, &family, &look(&[("head", 1)])).unwrap();
        assert_eq!(cache.builds, 2);

        for n in 0..(TEXTURE_CACHE_CAP as u8 + 10) {
            let mut l = look(&[]);
            l.body = [n, 0, 0];
            cache.texture(&ctx, &family, &l).unwrap();
            // Keep the healthy one hot so it survives eviction.
            cache.texture(&ctx, &family, &healthy).unwrap();
        }
        assert_eq!(cache.texture_count(), TEXTURE_CACHE_CAP);
        let builds = cache.builds;
        cache.texture(&ctx, &family, &healthy).unwrap();
        assert_eq!(cache.builds, builds, "the hot entry was never evicted");
    }

    /// Port-fidelity guard. `creature_field.rs` is a port VellumFE will
    /// adopt; the doll may only ADD to it, inside the `>>>doll` / `<<<doll`
    /// markers (the extra `else if` arm before the placeholder standee, and
    /// `paint_creature_doll`). With the marked regions removed the file must
    /// be byte-identical (line endings aside) to the ported file as of
    /// gs_studio 129091f — so the sprite branch, the placeholder standee and
    /// everything they call are provably untouched.
    #[test]
    fn creature_field_is_the_port_plus_marked_additions_only() {
        let src = include_str!("creature_field.rs").replace("\r\n", "\n");
        let mut stripped = String::with_capacity(src.len());
        let mut rest = src.as_str();
        let mut regions = 0;
        loop {
            let inline = rest.find("/* >>>doll */");
            let block = rest.find("// >>>doll\n");
            let (start, end_marker) = match (inline, block) {
                (Some(a), Some(b)) if a < b => (a, "/* <<<doll */ "),
                (Some(a), None) => (a, "/* <<<doll */ "),
                (_, Some(b)) => (b, "// <<<doll\n"),
                (None, None) => break,
            };
            stripped.push_str(&rest[..start]);
            let after = &rest[start..];
            let end = after.find(end_marker).expect("unterminated doll region");
            rest = &after[end + end_marker.len()..];
            regions += 1;
        }
        stripped.push_str(rest);
        // Eleven regions: the host's forced-puppet filter on the active art
        // (which also names the wearer for lighting), the draw-branch arms
        // (puppet, then doll, then the untouched standee), the block
        // holding both paint_creature_puppet and paint_creature_doll, six
        // inline lighting additions -- the wearer-lit copy of a shared card
        // overlay and part overlay, and the corpse tint on the quad overlay
        // and the three wound draws -- and the status icons: the text
        // badges skip iconed statuses, and a block draws the icons -- and
        // the weather (and point lights), behind the creatures and in front
        // of them -- and the firelight laid over a sprite's base art and a
        // prop's; lamps drawn in depth order between the cards; card labels
        // lifted over occluders -- and the v2 GPU lighting pass
        // (field_gpu_stage): its frame started before the backdrop, its
        // backdrop over the image, and its lit body in place of the
        // sprite's image -- and the floor grid drawn over the floor's own
        // rows and columns (deep growth adds rows), and scenery props
        // clipped to the stage like the backdrop.
        assert_eq!(regions, 22, "exactly the twenty-two documented additions");
        let mut h: u64 = 0xcbf29ce484222325;
        for b in stripped.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        assert_eq!(stripped.len(), 53919, "ported text length");
        assert_eq!(h, 0x90b9f65f4e317a19, "ported text (FNV-1a 64) changed outside the doll markers");
    }

    /// Cost of one texture build (compose only; the GPU upload is egui's).
    /// `cargo test -p gs_field_egui --release compose_cost -- --nocapture`.
    #[test]
    fn compose_cost() {
        let map = humanoid();
        let looks = [look(&[]), look(&[("chest", 2), ("leftLeg", 3)])];
        for l in &looks {
            let t = std::time::Instant::now();
            let n = 20;
            for _ in 0..n {
                std::hint::black_box(compose_rgba(&map, l));
            }
            println!(
                "compose {}x{} ({} wounds): {:.2} ms",
                map.width,
                map.height,
                l.ranks.iter().filter(|r| **r > 0).count(),
                t.elapsed().as_secs_f64() * 1000.0 / n as f64
            );
        }
        let t = std::time::Instant::now();
        let rig_png: Vec<u8> = gs_field::dolls::embedded().next().unwrap().1.to_vec();
        std::hint::black_box(DollMap::decode(&rig_png, map.meta.clone()));
        println!("decode label png: {:.2} ms", t.elapsed().as_secs_f64() * 1000.0);
    }
}
