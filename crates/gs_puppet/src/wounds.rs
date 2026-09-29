//! Wounds and scars drawn onto the smooth skin: procedural marks laid out
//! on the body's own surface (in feet, from a 1 px-per-foot `UvLayout`, as
//! web and frost are), so they keep to the body through any pose and turn.
//!
//! Rank 1 is bruising; rank 2 bruising and cuts, blood round them; rank 3
//! deep gashes in wet blood. A scar is a healed cut: a pale, slightly raised
//! line, wider with its rank. Every mark is fixed by its segment, so a
//! wound does not crawl from frame to frame, and each part's marks differ.
//! The flat shape list (small cards) keeps the part tint instead.
//!
//! A back wound on a form with no `back` part of its own (a biped: its
//! trunk is chest and abdomen) goes on the REAR of its chest and abdomen,
//! theirs on the front. From the front it shows by the blood it runs: down
//! the sides and the backs of the legs (rank 2 and 3), and by the guarding
//! posture the pose files give it (`[guard.back]`).

use crate::skin::smoothstep;

fn hash(a: i32, b: u32) -> f32 {
    let mut x = (a as u32).wrapping_mul(0x9E37_79B9) ^ b.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

/// Value noise over the surface, 0..1, smooth between whole cells.
fn noise(p: [f32; 2], seed: u32) -> f32 {
    let (fx, fy) = (p[0].floor(), p[1].floor());
    let (ux, uy) = (p[0] - fx, p[1] - fy);
    let (ux, uy) = (ux * ux * (3.0 - 2.0 * ux), uy * uy * (3.0 - 2.0 * uy));
    let at = |x: f32, y: f32| hash((x as i32).wrapping_mul(7919) ^ (y as i32).wrapping_mul(104_729), seed);
    let (a, b, c, d) = (at(fx, fy), at(fx + 1.0, fy), at(fx, fy + 1.0), at(fx + 1.0, fy + 1.0));
    let top = a + (b - a) * ux;
    let bottom = c + (d - c) * ux;
    top + (bottom - top) * uy
}

fn fbm(p: [f32; 2], seed: u32) -> f32 {
    0.65 * noise(p, seed) + 0.35 * noise([p[0] * 2.3 + 5.1, p[1] * 2.3 - 1.7], seed ^ 0x55)
}

fn mix(c: [f32; 3], to: [f32; 3], a: f32) -> [f32; 3] {
    let a = a.clamp(0.0, 1.0);
    [0, 1, 2].map(|k| c[k] + (to[k] - c[k]) * a)
}

const BRUISE_RGB: [f32; 3] = [88.0, 50.0, 72.0];
const FRESH_BRUISE_RGB: [f32; 3] = [128.0, 44.0, 48.0];
const BLOOD_RGB: [f32; 3] = [132.0, 18.0, 20.0];
const WET_BLOOD_RGB: [f32; 3] = [96.0, 8.0, 10.0];
const CUT_CORE_RGB: [f32; 3] = [58.0, 6.0, 8.0];
const SCAR_RGB: [f32; 3] = [236.0, 196.0, 190.0];

/// A family of wavering line marks (cuts, or scars): how much of one covers
/// `at`, 0..1, and how near its middle (1 = on the line). `spacing` feet
/// between lines, `keep` the share of lines drawn, `half` a line's half
/// width, `length` how long a run of line is before a gap, all in feet.
fn lines(at: [f32; 2], seed: u32, spacing: f32, keep: f32, half: f32, length: f32) -> (f32, f32) {
    let angle = hash(seed as i32, 3) * std::f32::consts::PI;
    let (s, c) = angle.sin_cos();
    let across = (at[0] * c + at[1] * s) / spacing;
    let along = -at[0] * s + at[1] * c;
    let cell = across.floor();
    let id = cell as i32 ^ (seed as i32).wrapping_mul(31);
    if hash(id, seed ^ 7) > keep {
        return (0.0, 0.0);
    }
    // a run of line, then a gap, tapering at its ends
    let run = noise([along / length + hash(id, 11) * 17.0, cell * 0.37], seed ^ 13);
    let taper = smoothstep(0.4, 0.55, run);
    if taper <= 0.0 {
        return (0.0, 0.0);
    }
    let wobble = 0.16 * (along * 3.1 / length + hash(id, 5) * std::f32::consts::TAU).sin();
    let d = (across - cell - 0.5 - wobble).abs() * spacing;
    let w = half * (0.45 + 0.55 * taper);
    let cover = 1.0 - smoothstep(w * 0.55, w, d);
    (cover, (1.0 - d / w.max(1e-4)).clamp(0.0, 1.0) * taper)
}

/// What one segment's skin carries: its own wound and scar, the back's
/// when it is split front from back, and the back's blood run over it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Marks {
    pub rank: u8,
    pub scar: u8,
    /// Its own marks on the front only, the back's on the rear.
    pub split: bool,
    pub back: u8,
    pub back_scar: u8,
    /// The back wound's rank whose blood runs down this segment (0 = none);
    /// `leg`: down its back, else down its sides and back.
    pub run: u8,
    pub leg: bool,
}

impl Marks {
    pub fn any(&self) -> bool {
        self.rank > 0 || self.scar > 0 || self.back > 0 || self.back_scar > 0 || self.run > 0
    }
}

/// Blood run down a surface from a back wound of `rank`: streaks along the
/// segment (the layout's y), some reaching further than others.
fn run_of_blood(at: [f32; 2], seg: u32, rank: u8, min_half: f32) -> f32 {
    let lane = 0.16;
    let cell = (at[0] / lane).floor();
    let id = cell as i32 ^ (seg as i32).wrapping_mul(131);
    if hash(id, 21) > if rank >= 3 { 0.6 } else { 0.35 } {
        return 0.0;
    }
    let wobble = 0.2 * (at[1] * 2.7 + hash(id, 23) * 6.0).sin();
    let d = (at[0] / lane - cell - 0.5 - wobble).abs() * lane;
    let half = (if rank >= 3 { 0.022 } else { 0.014 } as f32).max(min_half) * (0.6 + 0.8 * hash(id, 25));
    let reach = noise([at[1] / if rank >= 3 { 1.4 } else { 0.8 }, cell * 0.61], seg ^ 0x2B1);
    (1.0 - smoothstep(half * 0.5, half, d)) * smoothstep(0.42, 0.5, reach)
}

/// `col` with segment `seg`'s marks drawn on, at surface point `at` (feet),
/// facing `front` (-1 straight back .. 1 straight forward, rest space).
/// `edge` fades wounds where the part meets another (the ring's wound
/// tint); `min_half` is the least half width a line may have (a pixel).
pub(crate) fn marks(col: [f32; 3], at: [f32; 2], seg: u32, m: &Marks, front: f32, edge: f32, min_half: f32) -> [f32; 3] {
    let fore = if m.split { smoothstep(-0.25, 0.25, front) } else { 1.0 };
    let mut c = mark(col, at, seg, m.rank, m.scar, edge * fore, min_half);
    if m.split && (m.back > 0 || m.back_scar > 0) {
        c = mark(c, at, seg ^ 0xBAC, m.back, m.back_scar, edge * (1.0 - fore), min_half);
    }
    if m.run > 0 {
        let side = if m.leg { 1.0 - smoothstep(-0.3, 0.3, front) } else { 1.0 - smoothstep(0.35, 0.75, front) };
        let a = run_of_blood(at, seg, m.run, min_half) * side;
        c = mix(c, if m.run >= 3 { WET_BLOOD_RGB } else { BLOOD_RGB }, a * 0.9);
    }
    c
}

/// `col` (0..255) with a wound of `rank` and scar of `scar` at surface
/// point `at` (feet) drawn on, laid out by `seg`; `edge` fades them.
fn mark(col: [f32; 3], at: [f32; 2], seg: u32, rank: u8, scar: u8, edge: f32, min_half: f32) -> [f32; 3] {
    let edge = edge.clamp(0.0, 1.0);
    if edge <= 0.0 {
        return col;
    }
    let mut c = col;
    if scar > 0 {
        let r = scar.min(3) as usize - 1;
        let seed = seg.wrapping_mul(977) ^ 0x5CA2;
        let half = ([0.018, 0.028, 0.045][r] as f32).max(min_half);
        let (cover, mid) = lines(at, seed, [0.55, 0.5, 0.42][r], [0.55, 0.65, 0.7][r], half, [0.45, 0.6, 0.8][r]);
        // pale in the middle, a faint darker seam at its edges
        c = mix(c, c.map(|v| v * 0.62), cover * (1.0 - mid) * 0.8 * edge);
        c = mix(c, SCAR_RGB, cover * smoothstep(0.2, 0.6, mid) * 0.9 * edge);
    }
    if rank == 0 {
        return c;
    }
    let r = rank.min(3) as usize - 1;
    let seed = seg.wrapping_mul(7717) ^ 0xB10D;
    // bruising: soft blotches, more and redder with rank
    let blot = fbm([at[0] / 0.32, at[1] / 0.32], seed);
    let bruise = smoothstep([0.6, 0.52, 0.46][r], [0.78, 0.72, 0.66][r], blot) * edge;
    let tone = mix(BRUISE_RGB, FRESH_BRUISE_RGB, r as f32 * 0.5);
    c = mix(c, tone, bruise * [0.45, 0.5, 0.55][r]);
    if rank >= 2 {
        // cuts, with blood run into the skin round them
        let half = ([0.0, 0.03, 0.05][r] as f32).max(min_half);
        let (cover, mid) = lines(at, seed ^ 0xC07, [0.0, 0.5, 0.38][r], [0.0, 0.55, 0.75][r], half, [0.0, 0.45, 0.6][r]);
        let (halo, _) = lines(at, seed ^ 0xC07, [0.0, 0.5, 0.38][r], [0.0, 0.55, 0.75][r], half * 3.5, [0.0, 0.45, 0.6][r]);
        c = mix(c, BLOOD_RGB, halo * 0.35 * edge);
        c = mix(c, BLOOD_RGB, cover * edge);
        c = mix(c, CUT_CORE_RGB, cover * smoothstep(0.35, 0.8, mid) * edge);
    }
    if rank >= 3 {
        // wet blood pooled over the worst of it
        let pool = smoothstep(0.62, 0.74, fbm([at[0] / 0.22 + 3.3, at[1] / 0.22], seed ^ 0xB100)) * edge;
        c = mix(c, WET_BLOOD_RGB, pool * 0.8);
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn over(rank: u8, scar: u8) -> (f32, f32) {
        // a 3 x 3 ft patch of plain skin: how much changed, and how red
        let skin = [200.0, 170.0, 150.0];
        let (mut changed, mut red, mut n) = (0.0, 0.0, 0.0);
        for y in 0..120 {
            for x in 0..120 {
                let at = [x as f32 * 0.025, y as f32 * 0.025];
                let c = mark(skin, at, 4, rank, scar, 1.0, 0.01);
                let d = (0..3).map(|k| (c[k] - skin[k]).abs()).sum::<f32>();
                changed += (d > 12.0) as u32 as f32;
                red += (c[0] > c[1] * 1.8 && c[0] > 60.0) as u32 as f32;
                n += 1.0;
            }
        }
        (changed / n, red / n)
    }

    #[test]
    fn worse_wounds_mark_more_and_bleed() {
        let (a1, r1) = over(1, 0);
        let (a2, r2) = over(2, 0);
        let (a3, r3) = over(3, 0);
        assert!(a1 > 0.03 && a1 < a2 && a2 < a3, "{a1} {a2} {a3}");
        assert!(r1 < 0.01 && r2 > r1 && r3 > r2, "{r1} {r2} {r3}");
        assert!(a3 < 0.8, "a wound is marks on the skin, not a wash: {a3}");
    }

    #[test]
    fn a_scar_is_pale_and_a_healthy_part_is_untouched() {
        assert_eq!(over(0, 0), (0.0, 0.0));
        let (s, red) = over(0, 2);
        assert!(s > 0.005 && red == 0.0, "{s} {red}");
    }

    /// Split front from back: the front shows only its own wound, the back
    /// only the back's; blood runs down the back of a leg, not its front.
    #[test]
    fn a_split_trunk_keeps_front_and_back_apart_and_blood_runs_down_the_back() {
        let skin = [200.0, 170.0, 150.0];
        let m = Marks { split: true, back: 3, ..Marks::default() };
        let changed = |front: f32, m: &Marks| {
            (0..3600).filter(|i| marks(skin, [(i % 60) as f32 * 0.05, (i / 60) as f32 * 0.05], 4, m, front, 1.0, 0.01) != skin).count()
        };
        assert_eq!(changed(1.0, &m), 0, "the chest's front is clean");
        assert!(changed(-1.0, &m) > 200, "the back is wounded");
        let run = Marks { run: 3, leg: true, ..Marks::default() };
        assert_eq!(changed(1.0, &run), 0, "the front of the leg is clean");
        assert!(changed(-1.0, &run) > 100, "blood runs down its back: {}", changed(-1.0, &run));
    }

    #[test]
    fn marks_fade_where_the_part_ends_and_differ_by_part() {
        let skin = [200.0, 170.0, 150.0];
        let at = |i: u32| [(i % 40) as f32 * 0.07, (i / 40) as f32 * 0.07];
        assert!((0..1600).all(|i| mark(skin, at(i), 4, 3, 3, 0.0, 0.01) == skin));
        let differ = (0..1600).filter(|&i| mark(skin, at(i), 4, 2, 0, 1.0, 0.01) != mark(skin, at(i), 5, 2, 0, 1.0, 0.01)).count();
        assert!(differ > 50, "{differ}");
    }
}
