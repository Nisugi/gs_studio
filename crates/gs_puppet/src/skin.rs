//! The smooth skin: the same stations as [`Posed::draw_list`], rendered as
//! ONE surface into an image instead of a list of flat shapes.
//!
//! The shape list sorts whole groups (trunk, head, each limb) by their
//! average depth and paints each station pair as its own convex hull. Two
//! things follow, and both show while a figure turns: a limb and the trunk
//! swap order in one jump when their averages cross, and every hull brings
//! its own corners and contour, so joints read as bumps and notches.
//!
//! The skin fixes both at the root:
//!
//! * every station ring becomes a band of triangles, rasterised with a depth
//!   test per pixel, so what is in front is decided where it is in front;
//! * normals and colours are then averaged over a small window, but only
//!   between pixels whose depths agree, so a shoulder or a hip blends into
//!   the trunk while an arm held in front of the chest stays separate;
//! * contours come from the depth buffer: round the whole figure, and inside
//!   it only where a DIFFERENT segment sits clearly nearer -- and not one it
//!   is JOINED to ([`joined`]): the tubes a trunk is built from (spine,
//!   brisket, underline) are one body, so the valley where two of them meet
//!   is neither outlined nor shaded as a crease, and neither is the knee
//!   between a thigh and a shank.
//!
//! Shading is the shape list's cel look (a light fixed to the view, a lit
//! and a shaded band), softened at the band edges. Everything is egui-free;
//! the result is straight RGBA for any host to upload as a texture.
//!
//! Lights standing round the figure -- a campfire, a torch -- are not in
//! that look: [`Posed::skin_surface`] also keeps what every pixel shows
//! (where its surface point is, which way it faces, its colour), and
//! [`SkinSurface::lamp_light`] lights that from any [`Lamp`]s, as often as
//! they move, without drawing the figure again.

use std::sync::Arc;

use crate::draw::{surfaces, view_basis, Look, BOSS_OUTLINE, BODY_DEAD, DARK_BELOW, DEPTH_SHADE, EYE, LIGHT_ABOVE};
use crate::math::{add, cross, dot, len, mul_mv, norm_or, rot_y, scale, sub, V3};
use crate::pose::Posed;
use crate::uv::{Texture, UvLayout};

/// Samples per pixel along each axis while rasterising.
const SS: usize = 2;
/// No skin is rendered larger than this on a side.
const MAX_SIDE: usize = 4096;
/// Normals and colours are averaged over a window this fraction of the
/// figure's height across.
const SMOOTH_OF_HEIGHT: f32 = 0.035;
/// How far a normal sheet's deviation leans the surface normal (1 = as the
/// map was painted).
const NORMAL_MAP_STRENGTH: f32 = 1.0;
/// How strongly the relief shades: tone per unit change in the light's
/// incidence the normal sheet makes over the geometry's own.
const RELIEF_GAIN: f32 = 0.7;
/// Neighbours further apart in depth than this (fraction of height) are
/// different surfaces and never blend.
const BLEND_DEPTH_OF_HEIGHT: f32 = 0.04;
/// A different segment nearer by more than this (fraction of height) draws
/// a contour on the far one.
const CONTOUR_DEPTH_OF_HEIGHT: f32 = 0.035;
/// Joined segments blend across a depth step this many times the ordinary
/// tolerance: the valley between two trunk tubes shades as one surface.
const JOINED_BLEND: f32 = 3.0;

/// Which segments are parts of one continuous surface, as a `n * n` table:
/// a segment and the next along its chain (parent and child); and, inside
/// the trunk and the head, segments of the same bone, of a bone and its
/// parent, or of two sibling bones -- except that a tail joins only its
/// parent, so a tail laid across the flank keeps its line.
pub(crate) fn joined(puppet: &crate::Puppet) -> Vec<bool> {
    let segs = &puppet.segments;
    let n = segs.len();
    let bone_parent = |i: usize| -> Option<&str> {
        // The bone this segment's CHAIN hangs from: walk to its first
        // segment, then that segment's parent's bone.
        let mut at = i;
        while segs[at].chain_index > 0 {
            at = segs[at].parent?;
        }
        segs[at].parent.map(|p| segs[p].bone.as_str())
    };
    let body = |g: usize| puppet.groups.get(g).is_some_and(|name| name == "trunk" || name == "head");
    let tail = |i: usize| segs[i].bone.contains("tail");
    let mut out = vec![false; n * n];
    for i in 0..n {
        for j in 0..n {
            let (a, b) = (&segs[i], &segs[j]);
            // A prop or a garment is not flesh: it keeps its line against
            // the body it hangs from.
            let layer = |k: crate::Kind| (k.is_prop(), k.is_garment());
            // ...except a hem where it leaves the waist: no seam line there.
            let waist = |g: &crate::Segment, body: usize| {
                g.chain_index == 0 && g.parent == Some(body) && matches!(g.kind, crate::Kind::Garment { hang: crate::Hang::Toward { .. } })
            };
            let chain = i == j
                || (a.parent == Some(j) || b.parent == Some(i)) && layer(a.kind) == layer(b.kind)
                || waist(a, j)
                || waist(b, i);
            let related = a.group == b.group && body(a.group) && {
                let (pa, pb) = (bone_parent(i), bone_parent(j));
                a.bone == b.bone
                    || pa == Some(b.bone.as_str())
                    || pb == Some(a.bone.as_str())
                    || (pa.is_some() && pa == pb && !tail(i) && !tail(j))
            };
            out[i * n + j] = chain || related;
        }
    }
    out
}
/// Half-width of the softened step between tone bands, in `dot(n, light)`.
const BAND_SOFT: f32 = 0.09;
const LIT_TONE: f32 = 1.13;
const DARK_TONE: f32 = 0.8;
/// Strength of the continuous light term under the bands.
const LAMBERT: f32 = 0.08;
/// Extra width of the boss contour past the ordinary one, pixels.
const BOSS_EXTRA_PX: f32 = 2.5;

/// Things a status does to the body itself, drawn onto the skin.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Effects {
    /// Wrapped in web: strands round every limb, following the surface
    /// through any pose, and threads bridging limb to body.
    pub webbed: bool,
    /// Iced over: a cold tint with glints.
    pub frozen: bool,
}

impl Effects {
    pub fn any(&self) -> bool {
        self.webbed || self.frozen
    }
}

const WEB_RGB: [f32; 3] = [236.0, 236.0, 226.0];
const FROST_RGB: [f32; 3] = [138.0, 190.0, 240.0];
/// Strand spacing on the surface, feet.
const WEB_SPACING_FT: f32 = 0.42;
/// Threads bridging limb to body on a webbed figure.
const BIND_THREADS: u32 = 7;

fn hash(a: i32, b: u32) -> f32 {
    let mut x = (a as u32).wrapping_mul(0x9E37_79B9) ^ b.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

/// How much web covers a surface point (surface feet, from a 1 px-per-foot
/// layout): three families of wavering strands, some missing, crossing.
fn web_alpha(at: [f32; 2], half_width: f32) -> f32 {
    let mut a = 0.0f32;
    for (k, (dx, dy, sp)) in [(0.8f32, 0.6f32, 1.0f32), (-0.6, 0.8, 1.13), (0.0, 1.0, 1.7)].into_iter().enumerate() {
        let sp = sp * WEB_SPACING_FT;
        let across = (at[0] * dx + at[1] * dy) / sp;
        let along = (-at[0] * dy + at[1] * dx) / sp;
        let cell = across.floor();
        if hash(cell as i32, k as u32 + 11) < 0.22 {
            continue;
        }
        let wobble = 0.18 * (along * 1.9 + hash(cell as i32, k as u32) * std::f32::consts::TAU).sin();
        let d = (across - cell - 0.5 - wobble).abs() * sp;
        a = a.max(1.0 - smoothstep(half_width * 0.5, half_width, d));
    }
    a
}

/// A glint on frost: sparse bright cells.
fn frost_glint(at: [f32; 2]) -> bool {
    let (cx, cy) = ((at[0] / 0.07).floor() as i32, (at[1] / 0.07).floor() as i32);
    hash(cx.wrapping_mul(7919) ^ cy, 3) > 0.965
}

/// What the rasteriser paints a sample with, beyond the body colour.
struct Paint<'a> {
    tex: Option<&'a Texture>,
    /// A rest-space direction on segment `seg`, as posed and turned: what
    /// the normal sheet's deviations go through.
    rot: &'a (dyn Fn(usize, V3) -> V3 + Sync),
    /// A 1 px-per-foot layout of the surface, for effects measured in feet.
    surface: Option<&'a UvLayout>,
    effects: Effects,
    /// Half a web strand's width, feet.
    strand_ft: f32,
    /// Per segment: its wound and scar marks ([`crate::wounds`]), and
    /// them baked onto the surface layout, which the samples read.
    marks: &'a [crate::wounds::Marks],
    marks_sheet: Option<&'a crate::wounds::MarksSheet>,
}

/// Every segment's wound marks: its own part's wound and scar; on a form
/// with no `back` part, its chest and abdomen split front from back, the
/// back's wound on their rear, and the back's blood (rank 2 and up) run down
/// their sides and the backs of the legs.
fn segment_marks(puppet: &crate::puppet::Puppet, surfs: &[crate::draw::Surface], look: &Look) -> Vec<crate::wounds::Marks> {
    let mut marks = vec![crate::wounds::Marks::default(); puppet.segments.len()];
    let has_back = puppet.segments.iter().any(|s| s.part.as_deref() == Some("back"));
    let rank_of = |list: &[(&str, u8)]| list.iter().filter(|(p, _)| *p == "back").map(|(_, r)| (*r).min(3)).max().unwrap_or(0);
    let (back, back_scar) = if has_back { (0, 0) } else { (rank_of(look.wounds), rank_of(look.scars)) };
    let nerve = look.wounds.iter().filter(|(p, _)| *p == "nsys").map(|(_, r)| (*r).min(3)).max().unwrap_or(0);
    for surf in surfs {
        let Some(m) = marks.get_mut(surf.index) else { continue };
        let part = puppet.segments[surf.index].part.as_deref().unwrap_or("");
        let split = !has_back && matches!(part, "chest" | "abdomen");
        let leg = part.ends_with("Leg");
        *m = crate::wounds::Marks {
            rank: surf.rank,
            scar: surf.scar,
            split,
            back: if split { back } else { 0 },
            back_scar: if split { back_scar } else { 0 },
            run: if back >= 2 && (split || leg) { back } else { 0 },
            leg,
            nerve: if puppet.segments[surf.index].kind.is_prop() { 0 } else { nerve },
        };
    }
    marks
}

/// `marks` baked onto `layout` (the 1 px-per-foot surface layout), at a
/// resolution for lines no thinner than `min_half` feet -- cached, so a
/// figure bakes once per set of wounds and size, not per frame.
fn marks_sheet(
    puppet: &crate::puppet::Puppet,
    layout: &UvLayout,
    surfs: &[crate::draw::Surface],
    marks: &[crate::wounds::Marks],
    min_half: f32,
) -> std::sync::Arc<crate::wounds::MarksSheet> {
    use std::hash::{Hash, Hasher};
    // sizes in half-octave steps, so a figure growing a pixel does not rebake
    let half = 2f32.powf((min_half.max(1e-4).log2() * 2.0).round() / 2.0);
    let per_ft = (0.9 / half).clamp(24.0, 128.0);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (&puppet.family, puppet.segments.len(), puppet.height.to_bits(), layout.w, layout.h, marks, per_ft.to_bits(), half.to_bits()).hash(&mut h);
    let key = h.finish();
    crate::wounds::sheet(key, || {
        // the edge fade per ring, per segment, as the mesh has them
        let mut tints: Vec<Vec<f32>> = vec![Vec::new(); puppet.segments.len()];
        for surf in surfs {
            if let Some(t) = tints.get_mut(surf.index) {
                *t = surf.rings.iter().map(|r| r.tint).collect();
            }
        }
        let address = |at: [f32; 2]| {
            let (seg, station, theta) = layout.address(at)?;
            let s = puppet.segments.get(seg)?;
            let ring_tints = tints.get(seg).filter(|t| !t.is_empty())?;
            let at_station = |f: &dyn Fn(usize) -> f32, n: usize| {
                let k = (station.max(0.0).floor() as usize).min(n - 1);
                let k1 = (k + 1).min(n - 1);
                let t = (station - k as f32).clamp(0.0, 1.0);
                f(k) + (f(k1) - f(k)) * t
            };
            let edge = at_station(&|k| ring_tints[k.min(ring_tints.len() - 1)], ring_tints.len());
            let n = s.stations.len().max(1);
            let (w, d) = (at_station(&|k| s.stations[k.min(n - 1)].w, n), at_station(&|k| s.stations[k.min(n - 1)].d, n));
            let (sn, cs) = theta.sin_cos();
            let rest = add(scale(s.axis_w, cs * w), scale(s.axis_d, sn * d));
            let front = if len(rest) > 1e-5 { rest[2] / len(rest) } else { 0.0 };
            Some((seg, front, edge))
        };
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(16);
        crate::wounds::MarksSheet::bake(layout.w, layout.h, per_ft, marks, &address, half, threads)
    })
}

thread_local! {
    /// A test's thread count, over [`raster_threads`].
    pub(crate) static THREADS_FOR_TEST: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Fill `out` (rows of `w`) row by row with `row(y, the row)`, the rows
/// split across [`raster_threads`] threads.
fn par_rows<T: Send>(out: &mut [T], w: usize, row: &(dyn Fn(usize, &mut [T]) + Sync)) {
    let h = out.len() / w.max(1);
    let threads = raster_threads().min(h / 8).max(1);
    let rows = h.div_ceil(threads).max(1);
    let run = |band: usize, chunk: &mut [T]| {
        for (k, r) in chunk.chunks_mut(w).enumerate() {
            row(band * rows + k, r);
        }
    };
    if threads == 1 {
        run(0, out);
        return;
    }
    std::thread::scope(|scope| {
        for (band, chunk) in out.chunks_mut(rows * w).enumerate() {
            scope.spawn(move || run(band, chunk));
        }
    });
}

/// Threads a skin is rasterised on: `GS_PUPPET_THREADS` (1 = one), else
/// the machine's cores, at most 16.
fn raster_threads() -> usize {
    if let Some(n) = THREADS_FOR_TEST.with(|t| t.get()) {
        return n;
    }
    static THREADS: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *THREADS.get_or_init(|| {
        std::env::var("GS_PUPPET_THREADS")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()))
            .clamp(1, 16)
    })
}

/// One corner of a triangle of a skin's mesh, for a host that rasterises
/// the skin itself (the GPU skin, [`Posed::skin_mesh`]): three per triangle,
/// in order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkinCorner {
    /// Supersampled pixels ([`SKIN_SS`] per pixel) in the skin's frame.
    pub p: [f32; 2],
    /// Depth, the projection's (larger = farther).
    pub z: f32,
    pub n: V3,
    /// The surface point, world feet.
    pub pos: V3,
    /// Its colour (0..255) with no paint, and a wound's colour and how
    /// strongly it shows over paint.
    pub col: [f32; 3],
    pub wound: [f32; 4],
    pub seg: u32,
    /// Where on its segment's surface: station and ring angle, the angle
    /// unwrapped across the triangle; meaningless unless `addressed`.
    pub addr: [f32; 2],
    pub addressed: bool,
}

/// The skin's shading constants, for a host that shades it itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkinShading {
    pub light_above: f32,
    pub dark_below: f32,
    pub band_soft: f32,
    pub lit_tone: f32,
    pub dark_tone: f32,
    pub lambert: f32,
    pub relief_gain: f32,
    pub normal_map_strength: f32,
    pub depth_shade: f32,
    pub joined_blend: f32,
    pub boss_extra_px: f32,
    pub boss_rgb: [f32; 3],
}

/// Samples per pixel along each axis the skin is rasterised at.
pub const SKIN_SS: usize = SS;

/// Everything a host needs to rasterise and shade a skin itself -- the
/// same figure [`Posed::skin_surface`] draws, on the GPU (ADDITION, for
/// the doll): the mesh, how its paint and marks are laid out, and how it
/// is shaded.
#[derive(Debug, Clone)]
pub struct SkinMesh {
    /// Where the skin's top-left corner sits in the projection's pixels,
    /// and its size, as [`Skin`]'s.
    pub origin: [f32; 2],
    pub w: usize,
    pub h: usize,
    pub corners: Vec<SkinCorner>,
    /// Per segment: its posed and turned rotation, as columns (what a
    /// normal sheet's rest-space deviations go through).
    pub seg_rot: Vec<[V3; 3]>,
    /// Per pair of segments (`a * segments + b`): joined, so their seam is
    /// blended and draws no contour.
    pub joined: Vec<bool>,
    pub segments: usize,
    /// Toward the light and toward the viewer, in the skin's frame.
    pub light: V3,
    pub to_cam: V3,
    /// The contour's colour (0..255) and width, pixels; a boss's gold round it.
    pub line: [f32; 3],
    pub outline_px: f32,
    pub boss: bool,
    /// The normals' smoothing: radius (pixels) and depth tolerance; the
    /// inner contour's depth step; depth darkening's near and range.
    pub smooth_radius: usize,
    pub blend_tol: f32,
    pub contour_tol: f32,
    pub near: f32,
    pub range: f32,
    /// The paint's layout, [`UvLayout::packed`].
    pub paint_layout: Vec<f32>,
    /// Wound, scar and nerve marks: their sheet, the 1 px-per-foot surface
    /// layout it is laid on (packed), and which segments carry any.
    pub marks: Option<(Arc<crate::wounds::MarksSheet>, Vec<f32>, Vec<bool>)>,
    pub shading: SkinShading,
    /// Identifies this figure's pixels.
    pub key: u64,
}

/// What [`Posed::build_mesh`] makes: the mesh and what it was made with.
struct Mesh<'t> {
    texture: Option<&'t Texture>,
    line: [f32; 3],
    view: crate::draw::View,
    height: f32,
    verts: Vec<Vert>,
    tris: Vec<[u32; 3]>,
    surfs: Vec<crate::draw::Surface>,
    origin: [f32; 2],
    w: usize,
    h: usize,
}

/// A rendered figure.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Skin {
    /// Where the image's top-left corner sits in the pixel space of the
    /// projection it was rendered through.
    pub origin: [f32; 2],
    pub w: usize,
    pub h: usize,
    /// Straight (not premultiplied) RGBA8, row-major, transparent outside
    /// the figure and its contour.
    pub rgba: Vec<u8>,
}

impl Skin {
    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }
}

#[derive(Clone, Copy)]
struct Vert {
    /// Supersampled pixels, relative to the image.
    p: [f32; 2],
    z: f32,
    n: V3,
    /// The surface point, world feet.
    w: V3,
    col: [f32; 3],
    seg: u32,
    /// Surface address for a texture: (station, ring angle); NaN = none
    /// (a fan centre, a stump's raw end).
    addr: [f32; 2],
    /// A wound's colour and how strongly it shows over a texture here.
    wound: [f32; 4],
}

/// Samples round a ring for a figure whose largest ring is `px` across.
fn around(px: f32) -> usize {
    ((px * 0.9) as usize).clamp(10, 36)
}

/// The supersampled buffers the triangles are rasterised into.
struct Samples {
    w: usize,
    h: usize,
    z: Vec<f32>,
    n: Vec<V3>,
    col: Vec<[f32; 3]>,
    seg: Vec<u32>,
    /// Surface points, world feet; empty unless a [`SkinSurface`] is kept.
    pos: Vec<V3>,
    /// The normal sheet's deviation here, posed (zero without one).
    dev: Vec<V3>,
    /// How much it glows (nerve marks), 0..1.
    glow: Vec<f32>,
}

impl Samples {
    fn new(w: usize, h: usize, keep_pos: bool) -> Samples {
        let n = w * h;
        Samples {
            w,
            h,
            z: vec![f32::INFINITY; n],
            n: vec![[0.0; 3]; n],
            col: vec![[0.0; 3]; n],
            seg: vec![u32::MAX; n],
            pos: if keep_pos { vec![[0.0; 3]; n] } else { Vec::new() },
            dev: vec![[0.0; 3]; n],
            glow: vec![0.0; n],
        }
    }

    /// Rasterise `tris` over `threads` bands of rows, each thread running
    /// every triangle clipped to its own rows, in order -- the same result
    /// as one thread, sooner.
    fn rasterise(&mut self, verts: &[Vert], tris: &[[u32; 3]], paint: &Paint, threads: usize) {
        let w = self.w;
        let rows = self.h.div_ceil(threads.max(1)).max(8);
        let keep_pos = !self.pos.is_empty();
        let chunk = rows * w;
        let mut pos = self.pos.chunks_mut(chunk.max(1));
        let bands: Vec<Band> = self
            .z
            .chunks_mut(chunk)
            .zip(self.n.chunks_mut(chunk))
            .zip(self.col.chunks_mut(chunk))
            .zip(self.seg.chunks_mut(chunk))
            .zip(self.dev.chunks_mut(chunk))
            .zip(self.glow.chunks_mut(chunk))
            .enumerate()
            .map(|(k, (((((z, n), col), seg), dev), glow))| Band {
                y0: k * rows,
                w,
                z,
                n,
                col,
                seg,
                pos: if keep_pos { pos.next().unwrap_or_default() } else { &mut [] },
                dev,
                glow,
            })
            .collect();
        let run = |mut band: Band| {
            for t in tris {
                let v = t.map(|i| &verts[i as usize]);
                if v.iter().all(|c| c.p[0].is_finite() && c.p[1].is_finite() && c.z.is_finite()) {
                    band.tri(v, paint);
                }
            }
        };
        if bands.len() <= 1 {
            bands.into_iter().for_each(run);
        } else {
            std::thread::scope(|scope| {
                for band in bands {
                    scope.spawn(|| run(band));
                }
            });
        }
    }
}

/// One band of sample rows, from `y0`, for one rasterising thread.
struct Band<'a> {
    y0: usize,
    w: usize,
    z: &'a mut [f32],
    n: &'a mut [V3],
    col: &'a mut [[f32; 3]],
    seg: &'a mut [u32],
    pos: &'a mut [V3],
    dev: &'a mut [V3],
    glow: &'a mut [f32],
}

impl Band<'_> {
    fn tri(&mut self, v: [&Vert; 3], paint: &Paint) {
        // A textured triangle interpolates its corners' ADDRESSES and looks
        // each sample up on the sheet (effects do the same on a feet-scaled
        // layout). Angles are unwrapped first, so a triangle straddling
        // angle 0 does not sweep the whole ring.
        let addressed = v.iter().all(|c| c.addr[0].is_finite() && c.addr[1].is_finite());
        let tex = paint.tex.filter(|_| addressed);
        let surface = paint.surface.filter(|_| addressed);
        let mut theta = [v[0].addr[1], v[1].addr[1], v[2].addr[1]];
        if tex.is_some() || surface.is_some() {
            let lo = theta.iter().copied().fold(f32::MAX, f32::min);
            for t in &mut theta {
                if *t - lo > std::f32::consts::PI {
                    *t -= std::f32::consts::TAU;
                }
            }
        }
        let p = [v[0].p, v[1].p, v[2].p];
        let area = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[1][1] - p[0][1]) * (p[2][0] - p[0][0]);
        if area.abs() < 1e-6 || !area.is_finite() {
            return;
        }
        let min = [p[0][0].min(p[1][0]).min(p[2][0]), p[0][1].min(p[1][1]).min(p[2][1])];
        let max = [p[0][0].max(p[1][0]).max(p[2][0]), p[0][1].max(p[1][1]).max(p[2][1])];
        let x0 = (min[0] - 0.5).ceil().max(0.0) as usize;
        let y0 = (min[1] - 0.5).ceil().max(0.0) as usize;
        let rows = self.z.len() / self.w.max(1);
        let x1 = ((max[0] - 0.5).floor() as isize).min(self.w as isize - 1);
        let y0 = y0.max(self.y0);
        let y1 = ((max[1] - 0.5).floor() as isize).min((self.y0 + rows) as isize - 1);
        if x1 < x0 as isize || y1 < y0 as isize {
            return;
        }
        let inv = 1.0 / area;
        let edge = |a: [f32; 2], b: [f32; 2], q: [f32; 2]| (b[0] - a[0]) * (q[1] - a[1]) - (b[1] - a[1]) * (q[0] - a[0]);
        for y in y0..=y1 as usize {
            let row = (y - self.y0) * self.w;
            for x in x0..=x1 as usize {
                let q = [x as f32 + 0.5, y as f32 + 0.5];
                let w0 = edge(p[1], p[2], q) * inv;
                let w1 = edge(p[2], p[0], q) * inv;
                let w2 = 1.0 - w0 - w1;
                if w0 < -1e-5 || w1 < -1e-5 || w2 < -1e-5 {
                    continue;
                }
                let z = w0 * v[0].z + w1 * v[1].z + w2 * v[2].z;
                let i = row + x;
                if z >= self.z[i] {
                    continue;
                }
                self.z[i] = z;
                self.n[i] = add(add(scale(v[0].n, w0), scale(v[1].n, w1)), scale(v[2].n, w2));
                if !self.pos.is_empty() {
                    self.pos[i] = add(add(scale(v[0].w, w0), scale(v[1].w, w1)), scale(v[2].w, w2));
                }
                let lerp = |f: fn(&Vert) -> f32| w0 * f(v[0]) + w1 * f(v[1]) + w2 * f(v[2]);
                let station = lerp(|c| c.addr[0]);
                let angle = w0 * theta[0] + w1 * theta[1] + w2 * theta[2];
                let seg = v[0].seg as usize;
                let painted = tex.and_then(|t| t.sample(t.layout.locate(seg, station, angle)?));
                self.dev[i] = tex
                    .filter(|t| t.normal.is_some())
                    .and_then(|t| t.sample_normal(t.layout.locate(seg, station, angle)?))
                    .map_or([0.0; 3], |d| (paint.rot)(seg, d));
                let mut col = match painted {
                    Some(rgb) => {
                        let wound = [lerp(|c| c.wound[0]), lerp(|c| c.wound[1]), lerp(|c| c.wound[2])];
                        let a = lerp(|c| c.wound[3]);
                        [0, 1, 2].map(|k| rgb[k] + (wound[k] - rgb[k]) * a)
                    }
                    None => [0, 1, 2].map(|k| w0 * v[0].col[k] + w1 * v[1].col[k] + w2 * v[2].col[k]),
                };
                let feet = surface.and_then(|l| l.locate(seg, station, angle));
                let mut glow = 0.0;
                if let (Some(at), Some(sheet), Some(m)) = (feet, paint.marks_sheet, paint.marks.get(seg)) {
                    if m.any() {
                        (col, glow) = sheet.apply(col, at);
                    }
                }
                self.glow[i] = glow;
                if paint.effects.frozen {
                    col = [0, 1, 2].map(|k| col[k] + (FROST_RGB[k] - col[k]) * 0.62);
                    if feet.is_some_and(frost_glint) {
                        col = [0, 1, 2].map(|k| col[k] + (255.0 - col[k]) * 0.85);
                    }
                }
                if paint.effects.webbed {
                    if let Some(at) = feet {
                        let a = web_alpha(at, paint.strand_ft) * 0.92;
                        col = [0, 1, 2].map(|k| col[k] + (WEB_RGB[k] - col[k]) * a);
                    }
                }
                self.col[i] = col;
                // The segment of the corner this sample is closest to.
                self.seg[i] = if w0 >= w1 && w0 >= w2 { v[0].seg } else if w1 >= w2 { v[1].seg } else { v[2].seg };
            }
        }
    }
}

/// The samples averaged down to pixels.
struct Pixels {
    w: usize,
    h: usize,
    /// Fraction of the pixel's samples the figure covers.
    cov: Vec<f32>,
    /// Nearest sample's depth and segment (INFINITY / MAX when uncovered).
    z: Vec<f32>,
    seg: Vec<u32>,
    n: Vec<V3>,
    col: Vec<[f32; 3]>,
    /// The nearest sample's surface point; empty unless the samples kept them.
    pos: Vec<V3>,
    /// The normal sheet's deviation, averaged over the samples.
    dev: Vec<V3>,
    /// How much it glows, averaged over the samples.
    glow: Vec<f32>,
}

impl Pixels {
    fn resolve(s: &Samples, w: usize, h: usize) -> Pixels {
        /// One pixel's resolve: cov, z, seg, n, col, pos, dev, glow.
        type Px = (f32, f32, u32, V3, [f32; 3], V3, V3, f32);
        let empty: Px = (0.0, f32::INFINITY, u32::MAX, [0.0; 3], [0.0; 3], [0.0; 3], [0.0; 3], 0.0);
        let mut px: Vec<Px> = vec![empty; w * h];
        let keep_pos = !s.pos.is_empty();
        par_rows(&mut px, w, &|y, row| {
            for (x, out) in row.iter_mut().enumerate() {
                let mut hits = 0;
                let (mut n, mut col, mut dev, mut glow) = ([0.0f32; 3], [0.0f32; 3], [0.0f32; 3], 0.0f32);
                for sy in 0..SS {
                    for sx in 0..SS {
                        let j = (y * SS + sy) * s.w + x * SS + sx;
                        if !s.z[j].is_finite() {
                            continue;
                        }
                        hits += 1;
                        n = add(n, norm_or(s.n[j], [0.0, 0.0, 1.0]));
                        dev = add(dev, s.dev[j]);
                        glow += s.glow[j];
                        for k in 0..3 {
                            col[k] += s.col[j][k];
                        }
                        if s.z[j] < out.1 {
                            out.1 = s.z[j];
                            out.2 = s.seg[j];
                            if keep_pos {
                                out.5 = s.pos[j];
                            }
                        }
                    }
                }
                if hits > 0 {
                    out.0 = hits as f32 / (SS * SS) as f32;
                    out.3 = norm_or(n, [0.0, 0.0, 1.0]);
                    out.4 = col.map(|c| c / hits as f32);
                    out.6 = scale(dev, 1.0 / hits as f32);
                    out.7 = glow / hits as f32;
                }
            }
        });
        Pixels {
            w,
            h,
            cov: px.iter().map(|p| p.0).collect(),
            z: px.iter().map(|p| p.1).collect(),
            seg: px.iter().map(|p| p.2).collect(),
            n: px.iter().map(|p| p.3).collect(),
            col: px.iter().map(|p| p.4).collect(),
            pos: if keep_pos { px.iter().map(|p| p.5).collect() } else { Vec::new() },
            dev: px.iter().map(|p| p.6).collect(),
            glow: px.iter().map(|p| p.7).collect(),
        }
    }

    /// Average normals (and, with `colours`, colours) over a `2r + 1`
    /// window (one axis per pass), only between pixels whose depths are
    /// within `tol`. A texture's colours are left sharp: blending is for
    /// the shape, and for softening flat wound tints.
    fn smooth(&mut self, r: usize, tol: f32, colours: bool, joined: &(dyn Fn(u32, u32) -> bool + Sync)) {
        if r == 0 {
            return;
        }
        for (dx, dy) in [(1isize, 0isize), (0, 1)] {
            let mut out: Vec<(V3, [f32; 3])> = self.n.iter().copied().zip(self.col.iter().copied()).collect();
            let this = &*self;
            par_rows(&mut out, this.w, &|y, row| {
                for (x, slot) in row.iter_mut().enumerate() {
                    let i = y * this.w + x;
                    let (z0, s0) = (this.z[i], this.seg[i]);
                    if !z0.is_finite() {
                        continue;
                    }
                    let (mut n, mut col, mut k) = ([0.0f32; 3], [0.0f32; 3], 0.0f32);
                    for t in -(r as isize)..=r as isize {
                        let (qx, qy) = (x as isize + dx * t, y as isize + dy * t);
                        if qx < 0 || qy < 0 || qx >= this.w as isize || qy >= this.h as isize {
                            continue;
                        }
                        let j = qy as usize * this.w + qx as usize;
                        let dz = (this.z[j] - z0).abs();
                        if dz > tol && !(dz <= tol * JOINED_BLEND && joined(s0, this.seg[j])) {
                            continue;
                        }
                        n = add(n, this.n[j]);
                        for c in 0..3 {
                            col[c] += this.col[j][c];
                        }
                        k += 1.0;
                    }
                    slot.0 = norm_or(n, this.n[i]);
                    if colours {
                        slot.1 = col.map(|c| c / k.max(1.0));
                    }
                }
            });
            for (i, (n, c)) in out.into_iter().enumerate() {
                self.n[i] = n;
                self.col[i] = c;
            }
        }
    }

    /// Distance (pixels, chamfer 1 / sqrt 2) from every pixel to the nearest
    /// pixel the figure mostly covers; 0 on those.
    fn outside_distance(&self) -> Vec<f32> {
        let (w, h) = (self.w, self.h);
        let mut d: Vec<f32> = self.cov.iter().map(|c| if *c >= 0.5 { 0.0 } else { f32::INFINITY }).collect();
        let diag = std::f32::consts::SQRT_2;
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let mut v = d[i];
                if x > 0 {
                    v = v.min(d[i - 1] + 1.0);
                }
                if y > 0 {
                    v = v.min(d[i - w] + 1.0);
                    if x > 0 {
                        v = v.min(d[i - w - 1] + diag);
                    }
                    if x + 1 < w {
                        v = v.min(d[i - w + 1] + diag);
                    }
                }
                d[i] = v;
            }
        }
        for y in (0..h).rev() {
            for x in (0..w).rev() {
                let i = y * w + x;
                let mut v = d[i];
                if x + 1 < w {
                    v = v.min(d[i + 1] + 1.0);
                }
                if y + 1 < h {
                    v = v.min(d[i + w] + 1.0);
                    if x + 1 < w {
                        v = v.min(d[i + w + 1] + diag);
                    }
                    if x > 0 {
                        v = v.min(d[i + w - 1] + diag);
                    }
                }
                d[i] = v;
            }
        }
        d
    }

    /// Strength (0..=1) of an interior contour at every pixel: how close a
    /// pixel of ANOTHER segment is that sits nearer by more than `tol`.
    fn inner_contour(&self, width: f32, tol: f32, joined: &(dyn Fn(u32, u32) -> bool + Sync)) -> Vec<f32> {
        let mut out = vec![0.0; self.w * self.h];
        if width <= 0.0 {
            return out;
        }
        let r = (width + 1.0).ceil() as isize;
        par_rows(&mut out, self.w, &|y, row| {
            for (x, slot) in row.iter_mut().enumerate() {
                let i = y * self.w + x;
                let (z0, s0) = (self.z[i], self.seg[i]);
                if !z0.is_finite() {
                    continue;
                }
                let mut best = 0.0f32;
                for dy in -r..=r {
                    for dx in -r..=r {
                        let (qx, qy) = (x as isize + dx, y as isize + dy);
                        if qx < 0 || qy < 0 || qx >= self.w as isize || qy >= self.h as isize {
                            continue;
                        }
                        let j = qy as usize * self.w + qx as usize;
                        if self.seg[j] == s0 || self.z[j] >= z0 - tol || joined(s0, self.seg[j]) {
                            continue;
                        }
                        // Fade in with the step, so a limb growing out of
                        // the body starts its contour where it parts from it.
                        let step = smoothstep(tol, 2.0 * tol, z0 - self.z[j]);
                        let near = (width - ((dx * dx + dy * dy) as f32).sqrt() + 1.0).clamp(0.0, 1.0);
                        best = best.max(step * near);
                    }
                }
                *slot = best;
            }
        });
        out
    }
}

pub(crate) fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `top` over `under`, both straight RGB with an alpha.
fn over(top: ([f32; 3], f32), under: ([f32; 3], f32)) -> ([f32; 3], f32) {
    let a = top.1 + under.1 * (1.0 - top.1);
    if a <= 1e-6 {
        return ([0.0; 3], 0.0);
    }
    let c = [0, 1, 2].map(|k| (top.0[k] * top.1 + under.0[k] * under.1 * (1.0 - top.1)) / a);
    (c, a)
}

impl Posed<'_> {
    /// Render the puppet as one smooth surface through `project` (world
    /// feet -> screen pixels and a depth in feet, larger = farther). Uses
    /// `look` as [`Posed::draw_list`] does, except `min_station_px`: the skin
    /// keeps every station.
    pub fn skin(&self, project: &dyn Fn(V3) -> ([f32; 2], f32), look: &Look) -> Skin {
        self.skin_textured(project, look, None)
    }

    /// [`Posed::skin`] wearing a painted sheet (see [`crate::uv`]): each
    /// pixel takes its colour from where its surface point lies on the
    /// sheet; unpainted parts of the sheet fall back to the body colour.
    /// Wounds tint over the paint; a dead body's paint is greyed.
    pub fn skin_textured(
        &self,
        project: &dyn Fn(V3) -> ([f32; 2], f32),
        look: &Look,
        texture: Option<&Texture>,
    ) -> Skin {
        self.skin_with(project, look, texture, Effects::default())
    }

    /// [`Posed::skin_textured`] with status [`Effects`] drawn onto the body.
    pub fn skin_with(
        &self,
        project: &dyn Fn(V3) -> ([f32; 2], f32),
        look: &Look,
        texture: Option<&Texture>,
        effects: Effects,
    ) -> Skin {
        self.render(project, look, texture, effects, false).0
    }

    /// [`Posed::skin_with`], and what each of its pixels shows, for lamps
    /// to light ([`SkinSurface::lamp_light`]).
    pub fn skin_surface(
        &self,
        project: &dyn Fn(V3) -> ([f32; 2], f32),
        look: &Look,
        texture: Option<&Texture>,
        effects: Effects,
    ) -> (Skin, SkinSurface) {
        let (skin, surface) = self.render(project, look, texture, effects, true);
        (skin, surface.unwrap_or_default())
    }

    /// The mesh a skin is rasterised from: every visible segment's rings
    /// as triangles, in supersampled pixels of the skin's own frame (its
    /// top-left at `origin` in the projection's pixels), and what the rest
    /// of the render needs of it. None when nothing shows.
    fn build_mesh<'t>(&self, project: &dyn Fn(V3) -> ([f32; 2], f32), look: &Look, texture: Option<&'t Texture>) -> Option<Mesh<'t>> {
        // A sheet laid out for another puppet would put paint anywhere.
        // Held props come after the body and are never on the sheet.
        let body_segments = self.puppet.segments.iter().take_while(|s| !s.kind.is_prop()).count();
        let texture = texture.filter(|t| t.layout.patches.len() == body_segments);
        let puppet = self.puppet;
        let body = if look.dead { BODY_DEAD } else { look.body };
        let line = body.map(|c| c as f32 * 0.3);
        let view = view_basis(project, [0.0, puppet.height * 0.5, 0.0]);
        let height = puppet.height.max(0.1);

        // ---- the mesh, in unshifted supersampled pixels
        let ss = SS as f32;
        let mut verts: Vec<Vert> = Vec::new();
        let mut tris: Vec<[u32; 3]> = Vec::new();
        let surfs = surfaces(self, look);
        for surf in &surfs {
            let n = surf.rings.len();
            if n < 2 {
                continue;
            }
            let widest = surf
                .rings
                .iter()
                .map(|r| {
                    let (p, _) = project(r.c);
                    let e = |v: V3| {
                        let (q, _) = project(add(r.c, v));
                        (q[0] - p[0]).hypot(q[1] - p[1])
                    };
                    e(r.a).max(e(r.b))
                })
                .filter(|v| v.is_finite())
                .fold(0.0f32, f32::max);
            let m = around(widest * 2.0);
            let angles: Vec<(f32, f32)> =
                (0..m).map(|j| (j as f32 / m as f32 * std::f32::consts::TAU).sin_cos()).map(|(s, c)| (c, s)).collect();
            let pts: Vec<Vec<V3>> = surf
                .rings
                .iter()
                .map(|r| angles.iter().map(|(c, s)| add(r.c, add(scale(r.a, *c), scale(r.b, *s)))).collect())
                .collect();
            let middle = scale(surf.rings.iter().fold([0.0; 3], |acc, r| add(acc, r.c)), 1.0 / n as f32);
            let base = verts.len() as u32;
            let measured = puppet.segments[surf.index].stations.len();
            for (k, ring) in surf.rings.iter().enumerate() {
                // A wounded part keeps its own colour: its marks are the
                // wound (`wounds::mark`), not a tint over the whole part.
                let base = if surf.rank > 0 { body } else { surf.base };
                let col = ring.color(body, base).map(|c| c as f32);
                let wound = if ring.red {
                    [col[0], col[1], col[2], 1.0]
                } else if look.dead {
                    [BODY_DEAD[0] as f32, BODY_DEAD[1] as f32, BODY_DEAD[2] as f32, 0.7]
                } else if base != body {
                    [base[0] as f32, base[1] as f32, base[2] as f32, ring.tint * 0.75]
                } else {
                    [0.0; 4]
                };
                let addressed = !ring.red && k < measured && surf.index < body_segments;
                let (prev, next) = (k.saturating_sub(2), (k + 2).min(n - 1));
                for j in 0..m {
                    let p = pts[k][j];
                    let around_t = sub(pts[k][(j + 1) % m], pts[k][(j + m - 1) % m]);
                    let along = sub(pts[next][j], pts[prev][j]);
                    let radial = sub(p, ring.c);
                    let outward = if len(radial) > 1e-4 { radial } else { sub(p, middle) };
                    let mut nrm = norm_or(cross(around_t, along), norm_or(outward, view.to_cam));
                    if dot(nrm, outward) < 0.0 {
                        nrm = scale(nrm, -1.0);
                    }
                    let (sp, z) = project(p);
                    let theta = j as f32 / m as f32 * std::f32::consts::TAU;
                    let addr = if addressed { [k as f32, theta] } else { [f32::NAN; 2] };
                    verts.push(Vert { p: [sp[0] * ss, sp[1] * ss], z, n: nrm, w: p, col, seg: surf.index as u32, addr, wound });
                }
            }
            let at = |k: usize, j: usize| base + (k * m + j % m) as u32;
            for k in 0..n - 1 {
                for j in 0..m {
                    tris.push([at(k, j), at(k, j + 1), at(k + 1, j + 1)]);
                    tris.push([at(k, j), at(k + 1, j + 1), at(k + 1, j)]);
                }
            }
            // Close both ends with a fan, so a cap's last ring leaves no hole.
            for (k, toward) in [(0, 1), (n - 1, n - 2)] {
                let ring = &surf.rings[k];
                let (sp, z) = project(ring.c);
                let out = norm_or(sub(ring.c, surf.rings[toward].c), view.to_cam);
                let centre = verts.len() as u32;
                verts.push(Vert {
                    p: [sp[0] * ss, sp[1] * ss],
                    z,
                    n: out,
                    w: ring.c,
                    col: ring.color(body, if surf.rank > 0 { body } else { surf.base }).map(|c| c as f32),
                    seg: surf.index as u32,
                    addr: [f32::NAN; 2],
                    wound: [0.0; 4],
                });
                for j in 0..m {
                    tris.push([centre, at(k, j), at(k, j + 1)]);
                }
            }
        }

        // ---- frame: the mesh's box plus room for the contours
        let margin = (look.outline_px + if look.boss_outline { BOSS_EXTRA_PX } else { 0.0 }).ceil() + 2.0;
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for v in verts.iter().filter(|v| v.p[0].is_finite() && v.p[1].is_finite() && v.z.is_finite()) {
            for k in 0..2 {
                lo[k] = lo[k].min(v.p[k] / ss);
                hi[k] = hi[k].max(v.p[k] / ss);
            }
        }
        if lo[0] > hi[0] {
            return None;
        }
        let origin = [(lo[0] - margin).floor(), (lo[1] - margin).floor()];
        let w = ((hi[0] + margin).ceil() - origin[0]) as usize;
        let h = ((hi[1] + margin).ceil() - origin[1]) as usize;
        if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
            return None;
        }
        for v in &mut verts {
            v.p = [v.p[0] - origin[0] * ss, v.p[1] - origin[1] * ss];
        }
        Some(Mesh { texture, line, view, height, verts, tris, surfs, origin, w, h })
    }

    /// This figure as a mesh and what shades it, for a host that
    /// rasterises the skin itself (the GPU skin): the same figure
    /// [`Posed::skin_surface`] draws with `texture`. None when nothing
    /// shows, or for what the host does not draw yet -- an unpainted skin
    /// (its dotted eyes) and status effects -- which it draws on the CPU.
    pub fn skin_mesh(&self, project: &dyn Fn(V3) -> ([f32; 2], f32), look: &Look, texture: Option<&Texture>, effects: Effects) -> Option<SkinMesh> {
        use std::hash::{Hash, Hasher};
        if effects.any() || texture.is_none() {
            return None;
        }
        let Mesh { texture, line, view, height, verts, tris, surfs, origin, w, h } = self.build_mesh(project, look, texture)?;
        let texture = texture?;
        let puppet = self.puppet;
        let px_per_ft = {
            let mid = [0.0, height * 0.5, 0.0];
            let (a, _) = project(mid);
            let (b, _) = project(add(mid, [0.0, 1.0, 0.0]));
            (a[0] - b[0]).hypot(a[1] - b[1])
        };
        let marks = if look.dead { vec![crate::wounds::Marks::default(); puppet.segments.len()] } else { segment_marks(puppet, &surfs, look) };
        let marked = marks.iter().any(|m| m.any());
        let strand_ft = (0.9 / px_per_ft.max(1e-3)).max(0.018);
        let marks = marked.then(|| {
            let layout = UvLayout::new(puppet, 1.0);
            let sheet = marks_sheet(puppet, &layout, &surfs, &marks, strand_ft * 0.8);
            (sheet, layout.packed(), marks.iter().map(|m| m.any()).collect())
        });
        let mut corners = Vec::with_capacity(tris.len() * 3);
        for t in &tris {
            let v = t.map(|i| &verts[i as usize]);
            if !v.iter().all(|c| c.p[0].is_finite() && c.p[1].is_finite() && c.z.is_finite()) {
                continue;
            }
            // as `Band::tri`: angles unwrapped, so a triangle straddling
            // angle 0 does not sweep the whole ring
            let addressed = v.iter().all(|c| c.addr[0].is_finite() && c.addr[1].is_finite());
            let mut theta = [v[0].addr[1], v[1].addr[1], v[2].addr[1]];
            if addressed {
                let lo = theta.iter().copied().fold(f32::MAX, f32::min);
                for t in &mut theta {
                    if *t - lo > std::f32::consts::PI {
                        *t -= std::f32::consts::TAU;
                    }
                }
            }
            for (c, th) in v.iter().zip(theta) {
                corners.push(SkinCorner {
                    p: c.p,
                    z: c.z,
                    n: c.n,
                    pos: c.w,
                    col: c.col,
                    wound: c.wound,
                    seg: c.seg,
                    addr: if addressed { [c.addr[0], th] } else { [0.0; 2] },
                    addressed,
                });
            }
        }
        let (near, far) = corners.iter().fold((f32::MAX, f32::MIN), |(n, f), c| (n.min(c.z), f.max(c.z)));
        let turn = rot_y(look.yaw);
        let seg_rot = (0..puppet.segments.len()).map(|seg| [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]].map(|e| mul_mv(&turn, self.xf[seg].dir(e)))).collect();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (w, h, origin.map(f32::to_bits), corners.len()).hash(&mut hasher);
        for c in corners.iter().step_by(61) {
            (c.p.map(f32::to_bits), c.z.to_bits(), c.col.map(f32::to_bits), c.seg).hash(&mut hasher);
        }
        (texture.rgba.len(), texture.rgba.as_ptr() as usize, look.outline_px.to_bits(), look.boss_outline).hash(&mut hasher);
        if let Some((sheet, _, _)) = &marks {
            (Arc::as_ptr(sheet) as usize).hash(&mut hasher);
        }
        Some(SkinMesh {
            origin,
            w,
            h,
            corners,
            seg_rot,
            joined: joined(puppet),
            segments: puppet.segments.len(),
            light: view.light,
            to_cam: view.to_cam,
            line,
            outline_px: look.outline_px,
            boss: look.boss_outline,
            smooth_radius: (SMOOTH_OF_HEIGHT * height * px_per_ft).round().clamp(0.0, 16.0) as usize,
            blend_tol: BLEND_DEPTH_OF_HEIGHT * height,
            contour_tol: CONTOUR_DEPTH_OF_HEIGHT * height,
            near,
            range: (far - near).max(2.0),
            paint_layout: texture.layout.packed(),
            marks,
            shading: SkinShading {
                light_above: LIGHT_ABOVE,
                dark_below: DARK_BELOW,
                band_soft: BAND_SOFT,
                lit_tone: LIT_TONE,
                dark_tone: DARK_TONE,
                lambert: LAMBERT,
                relief_gain: RELIEF_GAIN,
                normal_map_strength: NORMAL_MAP_STRENGTH,
                depth_shade: DEPTH_SHADE,
                joined_blend: JOINED_BLEND,
                boss_extra_px: BOSS_EXTRA_PX,
                boss_rgb: BOSS_OUTLINE.map(|c| c as f32),
            },
            key: hasher.finish(),
        })
    }

    fn render(
        &self,
        project: &dyn Fn(V3) -> ([f32; 2], f32),
        look: &Look,
        texture: Option<&Texture>,
        effects: Effects,
        keep_surface: bool,
    ) -> (Skin, Option<SkinSurface>) {
        let Some(Mesh { texture, line, view, height, verts, tris, surfs, origin, w, h }) = self.build_mesh(project, look, texture) else {
            return (Skin::default(), None);
        };
        let puppet = self.puppet;

        // ---- rasterise, resolve, smooth
        let px_per_ft = {
            let mid = [0.0, height * 0.5, 0.0];
            let (a, _) = project(mid);
            let (b, _) = project(add(mid, [0.0, 1.0, 0.0]));
            (a[0] - b[0]).hypot(a[1] - b[1])
        };
        let marks = if look.dead { vec![crate::wounds::Marks::default(); puppet.segments.len()] } else { segment_marks(puppet, &surfs, look) };
        let marked = marks.iter().any(|m| m.any());
        let surface = (effects.any() || marked).then(|| UvLayout::new(puppet, 1.0));
        let turn = rot_y(look.yaw);
        let rot = |seg: usize, v: V3| mul_mv(&turn, self.xf[seg].dir(v));
        let strand_ft = (0.9 / px_per_ft.max(1e-3)).max(0.018);
        let marks_sheet = surface.as_ref().filter(|_| marked).map(|layout| marks_sheet(puppet, layout, &surfs, &marks, strand_ft * 0.8));
        let paint = Paint {
            tex: texture,
            rot: &rot,
            surface: surface.as_ref(),
            effects,
            // At least about a pixel wide, however small the figure.
            strand_ft,
            marks: &marks,
            marks_sheet: marks_sheet.as_deref(),
        };
        let mut samples = Samples::new(w * SS, h * SS, keep_surface);
        samples.rasterise(&verts, &tris, &paint, raster_threads());
        let mut px = Pixels::resolve(&samples, w, h);
        drop(samples);
        let radius = (SMOOTH_OF_HEIGHT * height * px_per_ft).round().clamp(0.0, 16.0) as usize;
        let table = joined(puppet);
        let count = puppet.segments.len();
        let is_joined = |a: u32, b: u32| {
            let (a, b) = (a as usize, b as usize);
            a < count && b < count && table[a * count + b]
        };
        px.smooth(radius, BLEND_DEPTH_OF_HEIGHT * height, texture.is_none() && !effects.any() && !marked, &is_joined);
        // The normal sheet's relief goes on AFTER the smoothing (which is
        // for the geometry's seams, and would blur a rib away), leaning the
        // surface's own normal by the painted deviation: the cel bands and
        // every lamp then see the ribs, folds and cracks the paint has.
        // The geometry's own normals are kept: the cel bands are theirs, and
        // the relief shows as the CHANGE in the light's incidence the map
        // makes on top (the bands have a tiny continuous term, and a rib
        // vanished into them).
        let relief = texture.is_some_and(|t| t.normal.is_some()).then(|| {
            let geo = px.n.clone();
            for i in 0..w * h {
                if px.dev[i] != [0.0; 3] {
                    px.n[i] = norm_or(add(px.n[i], scale(px.dev[i], NORMAL_MAP_STRENGTH)), px.n[i]);
                }
            }
            geo
        });

        // ---- shade and composite
        let (near, far) =
            px.z.iter().filter(|z| z.is_finite()).fold((f32::MAX, f32::MIN), |(n, f), z| (n.min(*z), f.max(*z)));
        let range = (far - near).max(2.0);
        let outside = px.outside_distance();
        let inner = px.inner_contour(look.outline_px, CONTOUR_DEPTH_OF_HEIGHT * height, &is_joined);
        let gold = BOSS_OUTLINE.map(|c| c as f32);
        let mut rgba = vec![0u8; w * h * 4];
        let mut kept = keep_surface.then(|| SkinSurface::blank(w, h, view.to_cam));
        for i in 0..w * h {
            let mut under = ([0.0; 3], 0.0);
            if look.boss_outline {
                let a = (look.outline_px + BOSS_EXTRA_PX - outside[i] + 1.0).clamp(0.0, 1.0);
                under = over((gold, a), under);
            }
            if look.outline_px > 0.0 {
                let a = (look.outline_px - outside[i] + 1.0).clamp(0.0, 1.0);
                under = over((line, a), under);
            }
            let mut top = ([0.0; 3], 0.0);
            if px.cov[i] > 0.0 {
                let n = px.n[i];
                let g = dot(n, view.light);
                let (gb, dg) = match &relief {
                    Some(geo) => {
                        let gb = dot(geo[i], view.light);
                        (gb, g - gb)
                    }
                    None => (g, 0.0),
                };
                let lit = smoothstep(LIGHT_ABOVE - BAND_SOFT, LIGHT_ABOVE + BAND_SOFT, gb);
                let dark = 1.0 - smoothstep(DARK_BELOW - BAND_SOFT, DARK_BELOW + BAND_SOFT, gb);
                // The cel bands, plus a gentle continuous term so a form still
                // turns between them, and the normal sheet's relief on top.
                let tone = (1.0 + (LIT_TONE - 1.0) * lit - (1.0 - DARK_TONE) * dark) * (1.0 + LAMBERT * gb) * (1.0 + RELIEF_GAIN * dg);
                let dim = 1.0 - DEPTH_SHADE * ((px.z[i] - near) / range).clamp(0.0, 1.0);
                let mut c = px.col[i].map(|v| (v * tone * dim).clamp(0.0, 255.0));
                if px.glow[i] > 0.0 {
                    // a glow is its own light: the bands do not darken it
                    c = [0, 1, 2].map(|k| c[k] + (px.col[i][k].max(c[k]) - c[k]) * px.glow[i]);
                }
                if inner[i] > 0.0 {
                    c = [0, 1, 2].map(|k| c[k] + (line[k] - c[k]) * inner[i]);
                }
                top = (c, px.cov[i]);
                if let Some(s) = kept.as_mut() {
                    // Its own colour, unshaded: what a lamp's light shows.
                    let albedo = [0, 1, 2].map(|k| px.col[i][k] + (line[k] - px.col[i][k]) * inner[i]);
                    s.set(i, px.pos[i], px.n[i], albedo, px.cov[i], px.glow[i]);
                }
            }
            let (c, a) = over(top, under);
            let o = i * 4;
            rgba[o..o + 3].copy_from_slice(&c.map(|v| v.round().clamp(0.0, 255.0) as u8));
            rgba[o + 3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
        }

        if effects.webbed {
            bind_threads(&mut rgba, &px, &puppet.family, (px_per_ft * 0.012).max(0.7));
        }

        // ---- eyes, where the surface they sit on is the one that shows --
        // unless a painted skin supplies the face (its own eyes, and dots
        // on top of them would give it four)
        if !look.dead && texture.is_none() {
            let turn = rot_y(look.yaw);
            for eye in &puppet.eyes {
                if !surfs.iter().any(|s| s.index == eye.segment) {
                    continue;
                }
                let x = &self.xf[eye.segment];
                let pos = mul_mv(&turn, x.point(eye.pos));
                if dot(mul_mv(&turn, x.dir(eye.normal)), view.to_cam) <= 0.25 {
                    continue;
                }
                let (at, z) = project(pos);
                let (edge, _) = project(add(pos, scale(mul_mv(&turn, x.dir([0.0, 1.0, 0.0])), eye.radius)));
                let r = (edge[0] - at[0]).hypot(edge[1] - at[1]).max(0.9);
                let c = [at[0] - origin[0], at[1] - origin[1]];
                if !(c[0].is_finite() && c[1].is_finite() && r.is_finite()) {
                    continue;
                }
                let (cx, cy) = (c[0].floor() as isize, c[1].floor() as isize);
                if cx < 0 || cy < 0 || cx >= w as isize || cy >= h as isize {
                    continue;
                }
                let at_px = cy as usize * w + cx as usize;
                if px.z[at_px] < z - CONTOUR_DEPTH_OF_HEIGHT * height {
                    continue;
                }
                // Under a garment (a hood's rim) it is covered, however near.
                if puppet.segments.get(px.seg[at_px] as usize).is_some_and(|s| s.kind.is_garment()) {
                    continue;
                }
                let eye_rgb = [EYE[0], EYE[1], EYE[2]].map(|v| v as f32);
                let reach = (r + 1.0).ceil() as isize;
                for y in (cy - reach).max(0)..=(cy + reach).min(h as isize - 1) {
                    for x in (cx - reach).max(0)..=(cx + reach).min(w as isize - 1) {
                        let d = (x as f32 + 0.5 - c[0]).hypot(y as f32 + 0.5 - c[1]);
                        let a = (r - d + 0.5).clamp(0.0, 1.0);
                        if a <= 0.0 {
                            continue;
                        }
                        let o = (y as usize * w + x as usize) * 4;
                        let under = ([0, 1, 2].map(|k| rgba[o + k] as f32), rgba[o + 3] as f32 / 255.0);
                        let (col, alpha) = over((eye_rgb, a), under);
                        rgba[o..o + 3].copy_from_slice(&col.map(|v| v.round().clamp(0.0, 255.0) as u8));
                        rgba[o + 3] = (alpha * 255.0).round() as u8;
                    }
                }
            }
        }
        (Skin { origin, w, h, rgba }, kept)
    }
}

/// A light standing near a figure -- a campfire, a torch -- in the frame
/// its skin was drawn in ([`SkinSurface::lamp_light`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lamp {
    /// Where its flame is, world feet.
    pub pos: V3,
    /// Its colour, sRGB.
    pub rgb: [u8; 3],
    /// How far it reaches, feet: it fades smoothly to nothing there.
    pub reach: f32,
    /// 1 = an ordinary fire.
    pub strength: f32,
    /// Shines one way only; None = all round.
    pub beam: Option<Beam>,
}

/// A lamp's beam, for one that shines one way ([`Lamp::beam`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Beam {
    /// Which way it points across the floor (world; y is ignored).
    pub dir: V3,
    /// Its full width, and the width of its fading edge, degrees.
    pub spread: f32,
    pub soft: f32,
}

impl Beam {
    /// How much of the beam reaches a point `off` from the lamp, 0..=1, by
    /// its bearing across the floor: 1 inside, fading out over `soft` at
    /// the edge, and 1 at the lamp itself.
    pub fn at(&self, off: V3) -> f32 {
        let (l, dl) = (off[0].hypot(off[2]), self.dir[0].hypot(self.dir[2]));
        if l < 1e-4 || dl < 1e-6 {
            return 1.0;
        }
        let cos = ((off[0] * self.dir[0] + off[2] * self.dir[2]) / (l * dl)).clamp(-1.0, 1.0);
        let t = ((self.spread * 0.5 + self.soft.max(1e-3) * 0.5 - cos.acos().to_degrees()) / self.soft.max(1e-3)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }
}

/// The scene's own light on a figure: a directed light (the sun, the moon)
/// and the soft fill light from everywhere else (the sky, the room), for
/// [`SkinSurface::scene_lit`]. PROTOTYPE: the shading comes from the scene
/// instead of the skin's fixed cel light.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneLight {
    /// Toward the directed light, world (need not be unit length).
    pub toward: V3,
    /// Its colour, sRGB, and how strong it is (1 = full sun).
    pub rgb: [u8; 3],
    pub strength: f32,
    /// The fill light's colour, sRGB, and strength.
    pub fill_rgb: [u8; 3],
    pub fill: f32,
}

/// How far round a surface the directed light wraps past edge-on, as for
/// a lamp: the terminator is soft, as on skin and fur.
const SCENE_WRAP: f32 = 0.25;
/// The fill light is brighter on surfaces facing up (the sky) than down
/// (the ground): this much of it is lost facing straight down.
const FILL_GROUND: f32 = 0.45;
/// The rim a directed light draws along the edges that face it, seen
/// edge-on: what keeps a figure against a low sun readable. Its strength,
/// and how tightly it hugs the edge.
const SCENE_RIM: f32 = 0.9;
const SCENE_RIM_POWER: i32 = 3;

/// How far round a surface a lamp's light wraps past the line where it
/// meets the surface edge-on (0 = a hard terminator).
const LAMP_WRAP: f32 = 0.3;
/// A lamp's light on a surface facing it squarely, at the flame, as a
/// multiple of the surface's own colour.
const LAMP_GAIN: f32 = 2.4;
/// The sheen a lamp leaves where the surface mirrors it toward the viewer.
const LAMP_SHEEN: f32 = 0.16;
const LAMP_SHINE: f32 = 18.0;

/// What each pixel of a [`Skin`] shows, from [`Posed::skin_surface`]: the
/// surface point (world feet, the frame the skin was projected from), which
/// way it faces, its own colour before shading, and how much of the pixel
/// the body covers (0 = none of it).
#[derive(Debug, Clone, Default)]
pub struct SkinSurface {
    pub w: usize,
    pub h: usize,
    /// Toward the viewer, world.
    to_cam: V3,
    pos: Vec<V3>,
    /// Unit normals, packed to a byte per axis.
    normal: Vec<[i8; 3]>,
    albedo: Vec<[u8; 3]>,
    cov: Vec<u8>,
    /// How much each pixel glows (nerve marks), 0..255.
    glow: Vec<u8>,
}

impl SkinSurface {
    fn blank(w: usize, h: usize, to_cam: V3) -> SkinSurface {
        let n = w * h;
        SkinSurface { w, h, to_cam, pos: vec![[0.0; 3]; n], normal: vec![[0; 3]; n], albedo: vec![[0; 3]; n], cov: vec![0; n], glow: vec![0; n] }
    }

    /// The unit normal at pixel `i` (row-major), or None where the body
    /// does not cover it.
    pub fn normal_at(&self, i: usize) -> Option<V3> {
        (self.cov.get(i).copied().unwrap_or(0) > 0).then(|| self.normal[i].map(|v| v as f32 / 127.0))
    }

    /// Toward the viewer, in the skin's frame.
    pub fn to_cam(&self) -> V3 {
        self.to_cam
    }

    /// Everything pixel `i` (row-major) shows, for a host that lights the
    /// skin itself (the v2 field pass): its surface point (world feet, the
    /// skin's frame), unit normal, colour before shading, and how much of
    /// the pixel the body covers (0..255). None where it covers nothing.
    pub fn texel(&self, i: usize) -> Option<(V3, V3, [u8; 3], u8)> {
        let cov = self.cov.get(i).copied().unwrap_or(0);
        (cov > 0).then(|| (self.pos[i], self.normal[i].map(|v| v as f32 / 127.0), self.albedo[i], cov))
    }

    /// How much each pixel glows, 0..255 (row-major): what a lit host
    /// keeps showing where no light reaches -- nerve marks.
    pub fn glow(&self) -> &[u8] {
        &self.glow
    }

    fn set(&mut self, i: usize, pos: V3, n: V3, albedo: [f32; 3], cov: f32, glow: f32) {
        self.glow[i] = (glow * 255.0).round().clamp(0.0, 255.0) as u8;
        self.pos[i] = pos;
        self.normal[i] = n.map(|v| (v.clamp(-1.0, 1.0) * 127.0).round() as i8);
        self.albedo[i] = albedo.map(|v| v.round().clamp(0.0, 255.0) as u8);
        self.cov[i] = (cov * 255.0).round().clamp(0.0, 255.0) as u8;
    }

    /// The figure lit by the scene alone (PROTOTYPE): each pixel's own
    /// colour -- the paint, flat, with no cel bands, roundness term or
    /// depth darkening -- under `light`'s fill and directed light, from
    /// the way that pixel faces (the normal sheet's relief included).
    /// Straight RGBA8 the size of `skin`, which supplies everything the
    /// surface does not cover (the outline round the figure).
    pub fn scene_lit(&self, light: &SceneLight, skin: &Skin) -> Vec<u8> {
        let mut out = skin.rgba.clone();
        if skin.w != self.w || skin.h != self.h {
            return out;
        }
        let decode = srgb_lut();
        let sun = light.rgb.map(|c| decode[c as usize] * light.strength);
        let fill = light.fill_rgb.map(|c| decode[c as usize] * light.fill);
        let toward = norm_or(light.toward, [0.0, 1.0, 0.0]);
        for i in 0..self.w * self.h {
            if self.cov[i] == 0 {
                continue;
            }
            let n = norm_or(self.normal[i].map(|v| v as f32 / 127.0), self.to_cam);
            let albedo = self.albedo[i].map(|c| decode[c as usize]);
            let facing_light = dot(n, toward);
            let edge = (1.0 - dot(n, self.to_cam).max(0.0)).powi(SCENE_RIM_POWER);
            let diffuse = ((facing_light + SCENE_WRAP) / (1.0 + SCENE_WRAP)).max(0.0) + SCENE_RIM * edge * facing_light.max(0.0);
            let sky = 1.0 - FILL_GROUND * (1.0 - n[1]) * 0.5;
            let o = i * 4;
            let cov = self.cov[i] as f32 / 255.0;
            for k in 0..3 {
                let lit = linear_to_srgb8(albedo[k] * (fill[k] * sky + sun[k] * diffuse)) as f32;
                out[o + k] = (lit * cov + out[o + k] as f32 * (1.0 - cov)).round() as u8;
            }
        }
        out
    }

    /// The light `lamps` add to `skin` (the skin this surface came with, as
    /// it is drawn -- lit by the scene or not): brightest where a surface
    /// faces a flame and is near it, fading to nothing at a lamp's reach,
    /// with a sheen where the surface mirrors a flame toward the viewer.
    ///
    /// RGBA8, PREMULTIPLIED, with alpha 0 everywhere: each pixel's colour is
    /// how far the lamps lift the skin's there (sRGB steps, so that laid
    /// over it by addition it shows `skin + light` in linear light), scaled
    /// by the body's coverage. A premultiplied-alpha compositor draws alpha 0
    /// as pure addition; scaling it (a flicker) dims the lamps' light alone.
    pub fn lamp_light(&self, lamps: &[Lamp], skin: &Skin) -> Vec<u8> {
        let mut out = vec![0u8; self.w * self.h * 4];
        if skin.w != self.w || skin.h != self.h || lamps.is_empty() {
            return out;
        }
        let decode = srgb_lut();
        let lamps: Vec<(Lamp, [f32; 3])> =
            lamps.iter().filter(|l| l.reach > 0.0 && l.strength > 0.0).map(|l| (*l, l.rgb.map(|c| decode[c as usize]))).collect();
        for i in 0..self.w * self.h {
            if self.cov[i] == 0 {
                continue;
            }
            let (p, n) = (self.pos[i], self.normal[i].map(|v| v as f32 / 127.0));
            let n = norm_or(n, self.to_cam);
            let albedo = self.albedo[i].map(|c| decode[c as usize]);
            let mut light = [0.0f32; 3];
            for (lamp, rgb) in &lamps {
                let to = sub(lamp.pos, p);
                let d = len(to);
                if d >= lamp.reach {
                    continue;
                }
                let u = d / lamp.reach;
                let reach = (1.0 - u * u).powi(2) * lamp.strength * lamp.beam.map_or(1.0, |b| b.at(sub(p, lamp.pos)));
                let l = if d > 1e-4 { scale(to, 1.0 / d) } else { n };
                let facing = dot(n, l);
                let diffuse = ((facing + LAMP_WRAP) / (1.0 + LAMP_WRAP)).max(0.0);
                let sheen = if facing > 0.0 {
                    dot(n, norm_or(add(l, self.to_cam), n)).max(0.0).powf(LAMP_SHINE) * LAMP_SHEEN
                } else {
                    0.0
                };
                for k in 0..3 {
                    light[k] += rgb[k] * reach * (albedo[k] * diffuse * LAMP_GAIN + sheen);
                }
            }
            if light.iter().all(|v| *v <= 1e-5) {
                continue;
            }
            let o = i * 4;
            let cov = self.cov[i] as f32 / 255.0;
            for k in 0..3 {
                let under = skin.rgba[o + k];
                let lifted = linear_to_srgb8(decode[under as usize] + light[k]);
                out[o + k] = ((lifted.saturating_sub(under)) as f32 * cov).round() as u8;
            }
        }
        out
    }
}

/// sRGB8 -> linear, per level.
fn srgb_lut() -> [f32; 256] {
    std::array::from_fn(|i| {
        let c = i as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    })
}

fn linear_to_srgb8(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Threads across a webbed figure, each spanning from where it first meets
/// the body to where it last leaves it: over the gaps between an arm and
/// the chest, or between the legs, they bind limb to body. Placement is
/// fixed per family, so the web does not crawl between frames.
fn bind_threads(rgba: &mut [u8], px: &Pixels, family: &str, half_width: f32) {
    let (w, h) = (px.w, px.h);
    let seed = family.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for y in 0..h {
        for x in 0..w {
            if px.cov[y * w + x] >= 0.5 {
                lo = [lo[0].min(x as f32), lo[1].min(y as f32)];
                hi = [hi[0].max(x as f32), hi[1].max(y as f32)];
            }
        }
    }
    if lo[0] > hi[0] {
        return;
    }
    let (cx, cy) = ((lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5);
    let reach = (hi[0] - lo[0]).hypot(hi[1] - lo[1]);
    for k in 0..BIND_THREADS {
        let angle = hash(k as i32, seed) * std::f32::consts::PI;
        let offset = (hash(k as i32, seed ^ 0xA5A5) - 0.5) * 0.7;
        let (dx, dy) = angle.sin_cos();
        // A point on the line: the box centre, shifted across the line.
        let (ox, oy) = (cx - dy * offset * (hi[0] - lo[0]), cy + dx * offset * (hi[1] - lo[1]));
        // Walk the line; keep the first and last covered pixel.
        let steps = (reach * 2.0) as i32;
        let (mut first, mut last, mut covered) = (None, None, 0u32);
        for s in -steps..=steps {
            let t = s as f32 * 0.5;
            let (x, y) = (ox + dx * t, oy + dy * t);
            if x < 0.0 || y < 0.0 || x >= w as f32 || y >= h as f32 {
                continue;
            }
            if px.cov[y as usize * w + x as usize] >= 0.5 {
                first.get_or_insert(t);
                last = Some(t);
                covered += 1;
            }
        }
        let (Some(t0), Some(t1)) = (first, last) else { continue };
        // A thread that is mostly air (tail tip to snout over the back) reads
        // as a tripwire, not a binding: only short bridges over gaps.
        if t1 - t0 < 4.0 || (covered as f32 * 0.5) < (t1 - t0) * 0.6 {
            continue;
        }
        let (ax, ay, bx, by) = (ox + dx * t0, oy + dy * t0, ox + dx * t1, oy + dy * t1);
        let x0 = (ax.min(bx) - half_width - 1.0).max(0.0) as usize;
        let x1 = ((ax.max(bx) + half_width + 1.0) as usize).min(w - 1);
        let y0 = (ay.min(by) - half_width - 1.0).max(0.0) as usize;
        let y1 = ((ay.max(by) + half_width + 1.0) as usize).min(h - 1);
        let len2 = ((bx - ax).powi(2) + (by - ay).powi(2)).max(1e-6);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (qx, qy) = (x as f32 + 0.5, y as f32 + 0.5);
                let t = (((qx - ax) * (bx - ax) + (qy - ay) * (by - ay)) / len2).clamp(0.0, 1.0);
                let d = (qx - (ax + (bx - ax) * t)).hypot(qy - (ay + (by - ay) * t));
                let a = (half_width + 0.5 - d).clamp(0.0, 1.0) * 0.8;
                if a <= 0.0 {
                    continue;
                }
                let o = (y * w + x) * 4;
                let under = ([0, 1, 2].map(|c| rgba[o + c] as f32), rgba[o + 3] as f32 / 255.0);
                let (col, alpha) = over((WEB_RGB, a), under);
                rgba[o..o + 3].copy_from_slice(&col.map(|v| v.round().clamp(0.0, 255.0) as u8));
                rgba[o + 3] = (alpha * 255.0).round() as u8;
            }
        }
    }
}
