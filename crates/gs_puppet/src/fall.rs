//! Falling down dead: a procedural ragdoll from the pose a creature was in
//! to the dead pose it ends in.
//!
//! Not a rigid-body solver -- a pose is angles, and everything that draws a
//! puppet (skin, texture, wounds, grounding) wants angles, so the fall is
//! made of angles too, each driven by a small piece of physics:
//!
//! * The whole figure TOPPLES like a falling pole: its root turns from the
//!   old pose to the dead one with the pole's acceleration (slow, then
//!   fast), hits the ground at [`IMPACT_SECS`] and bounces once, a little.
//!   Its long axis only ever sinks on the way ([`turn_low`]): a body lying
//!   face down whose dead pose lies face up turns over on the ground rather
//!   than through standing.
//! * Every bone follows on its own SPRING (under-damped), released a moment
//!   after the bone it hangs from -- the legs buckle first, the trunk goes,
//!   and the arms, head and tail trail it, swing past where they come to
//!   rest and settle back. The impact kicks them once more.
//!
//! Angles move angle by angle (`BonePose::lerp`), so an overshoot past the
//! dead pose is still a set of joint angles, and a renderer's
//! `PoseSet::clamp` holds it inside the body type's joint limits.
//!
//! Grounding still runs per frame (`Puppet::pose` drops the lowest surface
//! onto y = 0), so the figure never sinks through the floor. The motion is
//! a pure function of time and a seed: the same death plays the same way,
//! and any frame can be drawn without the ones before it.

use std::collections::BTreeMap;

use crate::math::{axis_angle, cross, dot, len, mul_mm, mul_mv, norm_or, quat_from_m3, quat_to_m3, scale, Quat, IDENTITY, M3, V3};
use crate::pose::Pose;
use crate::puppet::Puppet;

/// When the body hits the ground.
pub const IMPACT_SECS: f32 = 0.55;
/// When the fall has settled into the dead pose.
pub const FALL_SECS: f32 = 1.7;
/// A bone that, moved alone to its dead angle, lifts the figure by more than
/// this share of its height is holding it down (see [`Fall::new`]).
const LIFTS: f32 = 0.03;

/// One creature's fall.
#[derive(Debug, Clone)]
pub struct Fall {
    from: Pose,
    to: Pose,
    /// Pose key -> (release delay s, spring angular frequency rad/s,
    /// how hard the impact kicks it).
    springs: BTreeMap<String, (f32, f32, f32)>,
    /// Pose keys that give way no faster than the body goes down.
    held: std::collections::BTreeSet<String>,
    /// It was lying already: nothing to fall into, so its limbs settle
    /// into their dead angles without the spring's overshoot or the
    /// impact's whip, which flung a lying body's arms up through the air.
    lying: bool,
    /// The body's long axis at rest (its tallest extent: up for a biped,
    /// nose to tail for a quadruped).
    long: V3,
}

/// 0..1 from a seed and a salt.
fn jitter(seed: u64, salt: &str) -> f32 {
    let mut h = seed ^ 0x9E37_79B9_7F4A_7C15;
    for b in salt.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01B3);
    }
    h ^= h >> 29;
    (h % 10_000) as f32 / 10_000.0
}

/// A spring released at rest toward 1: 0 at t <= 0, overshooting and
/// settling on 1. `zeta` < 1 is under-damped.
fn spring(t: f32, omega: f32, zeta: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    let wd = omega * (1.0 - zeta * zeta).sqrt();
    1.0 - (-zeta * omega * t).exp() * ((wd * t).cos() + zeta * omega / wd * (wd * t).sin())
}

/// The root's progress: a falling pole's angle (accelerating, reaching 1 at
/// impact), then one small bounce back up, damped out.
pub fn topple(t: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    if t < IMPACT_SECS {
        let u = t / IMPACT_SECS;
        // A pole tipping from near upright gains angle ~ u^2..u^3.
        return u * u * (0.4 + 0.6 * u);
    }
    let tau = t - IMPACT_SECS;
    1.0 - 0.07 * (-7.0 * tau).exp() * (13.0 * tau).sin().abs()
}

/// How deep in the skeleton each bone hangs (the root bone 0), by name.
fn depths(puppet: &Puppet) -> BTreeMap<String, usize> {
    let mut depth_of_segment: Vec<usize> = Vec::with_capacity(puppet.segments.len());
    let mut out = BTreeMap::new();
    for s in &puppet.segments {
        let d = match s.parent {
            None => 0,
            Some(p) => depth_of_segment[p] + usize::from(s.chain_index == 0),
        };
        depth_of_segment.push(d);
        out.entry(s.bone.clone()).or_insert(d);
    }
    out
}

impl Fall {
    /// From `from` (the pose it was in) to `to` (its dead pose), for this
    /// puppet's skeleton; `seed` varies the timing per creature.
    ///
    /// Legs give way first -- from standing, that is the buckle. But a figure
    /// already low (kneeling, sitting, lying on its side) is held down by its
    /// folded limbs, and straightening them first stood it back up before it
    /// fell. A bone whose move to its dead angle, alone, would lift the
    /// figure is HELD: it gives way only as fast as the body goes down, so
    /// the fall starts from where the creature was.
    pub fn new(puppet: &Puppet, from: &Pose, to: &Pose, seed: u64) -> Fall {
        let height = |p: &Pose| puppet.pose(p).bounds()[1][1];
        let h0 = height(from);
        let rest = puppet.pose(&Pose::default()).bounds();
        let extent = [0, 1, 2].map(|k| rest[1][k] - rest[0][k]);
        let long = if extent[1] >= extent[0].max(extent[2]) {
            [0.0, 1.0, 0.0]
        } else if extent[2] >= extent[0] {
            [0.0, 0.0, 1.0]
        } else {
            [1.0, 0.0, 0.0]
        };
        let lift = LIFTS * puppet.height.max(0.1);
        let mut held = std::collections::BTreeSet::new();
        // Only an upright body (kneeling, sitting) is held up by folded
        // limbs; one already lying has nothing to stand up on, and holding
        // its limbs only kept a prone body's spread arms up as it rolled.
        let q = from.root;
        let upright = 2.0 * q[0].abs().clamp(0.0, 1.0).acos() <= 45f32.to_radians();
        for key in to.bones.keys().chain(from.bones.keys()).filter(|_| upright) {
            if held.contains(key) {
                continue;
            }
            let mut moved = from.clone();
            moved.bones.insert(key.clone(), to.bone(key));
            if height(&moved) > h0 + lift {
                held.insert(key.clone());
            }
        }
        let depth = depths(puppet);
        let max_depth = depth.values().copied().max().unwrap_or(1).max(1) as f32;
        let mut springs = BTreeMap::new();
        for bone in depth.keys() {
            for key in [bone.clone(), stem(bone).to_owned()] {
                if springs.contains_key(&key) {
                    continue;
                }
                let d = depth.get(&key).or_else(|| depth.get(&format!("{key}_l"))).copied().unwrap_or(0) as f32 / max_depth;
                let legs = key.contains("leg") || key.contains("foot") || key.contains("paw");
                let j = jitter(seed, &key);
                // Legs give way first; the further a bone hangs from the
                // root, the later it is released and the looser it swings.
                let delay = if legs { 0.02 * j } else { 0.06 + 0.22 * d + 0.06 * j };
                let omega = 11.0 - 5.0 * d + 2.0 * j;
                let kick = if legs { 0.1 } else { 0.18 + 0.3 * d };
                springs.insert(key, (delay, omega, kick));
            }
        }
        let lying = !upright && h0 < puppet.height.max(0.1) * LYING_SHARE;
        Fall { from: from.clone(), to: to.clone(), springs, held, lying, long }
    }

    /// Done falling: from here on the dead pose itself is drawn.
    pub fn done(t: f32) -> bool {
        t >= FALL_SECS
    }

    /// The figure `t` seconds after death.
    pub fn at(&self, t: f32) -> Pose {
        if Fall::done(t) {
            return self.to.clone();
        }
        let root_p = topple(t);
        let mut keys: Vec<String> = Vec::new();
        for k in self.from.bones.keys().chain(self.to.bones.keys()) {
            // A pair named by one pose may be split by the other: move the
            // sides, so neither entry shadows the other (as `Pose::blend`).
            let sided = k.ends_with("_l") || k.ends_with("_r");
            let split = |p: &Pose| p.bones.contains_key(&format!("{k}_l")) || p.bones.contains_key(&format!("{k}_r"));
            if !sided && (split(&self.from) || split(&self.to)) {
                keys.extend([format!("{k}_l"), format!("{k}_r")]);
            } else {
                keys.push(k.clone());
            }
        }
        let mut bones = BTreeMap::new();
        for k in keys {
            if bones.contains_key(&k) {
                continue;
            }
            let (delay, omega, kick) = self.springs.get(&k).or_else(|| self.springs.get(stem(&k))).copied().unwrap_or((0.1, 8.0, 0.2));
            let mut p = spring(t - delay, omega, 0.32);
            // The impact whips everything once more.
            let tau = t - IMPACT_SECS;
            if tau > 0.0 && !self.lying {
                p += kick * (-6.0 * tau).exp() * (15.0 * tau).sin();
            }
            if self.lying {
                p = p.clamp(0.0, 1.0);
            }
            // Nothing hangs back much once the body is down.
            let p = p.max(root_p * 0.6);
            // A held bone waits until the body is half way down, then gives
            // way with it.
            let p = if self.held.contains(&k) || self.held.contains(stem(&k)) { p.min(((root_p - 0.5) * 2.0).clamp(0.0, 1.0)) } else { p };
            let (a, b) = (self.from.bone(&k), self.to.bone(&k));
            bones.insert(k, a.lerp(&b, p));
        }
        Pose { root: turn_low(self.from.root, self.to.root, self.long, root_p), bones }
    }
}

/// Under this share of its standing height, a body not upright is lying.
const LYING_SHARE: f32 = 0.45;

/// The shortest rotation taking unit `from` onto unit `to`.
fn swing(from: V3, to: V3) -> M3 {
    let axis = cross(from, to);
    let (s, c) = (len(axis), dot(from, to));
    if s > 1e-6 {
        axis_angle(scale(axis, 1.0 / s), s.atan2(c).to_degrees())
    } else if c > 0.0 {
        IDENTITY
    } else {
        let side = if from[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
        axis_angle(norm_or(cross(from, side), [0.0, 0.0, 1.0]), 180.0)
    }
}

fn transpose(m: &M3) -> M3 {
    [0, 1, 2].map(|i| [0, 1, 2].map(|j| m[j][i]))
}

/// Radians, wrapped into (-pi, pi].
fn wrap(a: f32) -> f32 {
    let t = std::f32::consts::TAU;
    let a = a.rem_euclid(t);
    if a > std::f32::consts::PI {
        a - t
    } else {
        a
    }
}

/// The root `p` of the way from `a` to `b`, turning so the body's long axis
/// (`long`, rest space) never climbs. Slerping the two straight across
/// swung a body lying face down through standing to lie face up -- the
/// dead standing up to fall. Instead the long axis's elevation moves
/// steadily from one end's to the other's while its heading swings round
/// the vertical, and the roll about it is eased separately. From standing
/// (the axis straight up) this is the same topple as before.
fn turn_low(a: Quat, b: Quat, long: V3, p: f32) -> Quat {
    let (ma, mb) = (quat_to_m3(a), quat_to_m3(b));
    let (la, lb) = (mul_mv(&ma, long), mul_mv(&mb, long));
    let elevation = |l: V3| l[1].clamp(-1.0, 1.0).asin();
    let heading = |l: V3| (l[0].hypot(l[2]) > 1e-3).then(|| l[0].atan2(l[2]));
    let (ha, hb) = match (heading(la), heading(lb)) {
        (Some(x), Some(y)) => (x, y),
        (Some(x), None) => (x, x),
        (None, Some(y)) => (y, y),
        (None, None) => (0.0, 0.0),
    };
    let (ea, eb) = (elevation(la), elevation(lb));
    let (e, h) = (ea + (eb - ea) * p, ha + wrap(hb - ha) * p);
    let l = [e.cos() * h.sin(), e.sin(), e.cos() * h.cos()];
    // What is left of each end once its long axis is swung home: a roll
    // about the long axis.
    let side = norm_or(cross(long, if long[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] }), [0.0, 0.0, 1.0]);
    let roll = |m: &M3, l: V3| {
        let twist = mul_mm(&transpose(&swing(long, l)), m);
        let v = mul_mv(&twist, side);
        dot(long, cross(side, v)).atan2(dot(side, v))
    };
    let (ra, rb) = (roll(&ma, la), roll(&mb, lb));
    let r = ra + wrap(rb - ra) * p;
    quat_from_m3(&mul_mm(&swing(long, l), &axis_angle(long, r.to_degrees())))
}

fn stem(key: &str) -> &str {
    key.strip_suffix("_l").or_else(|| key.strip_suffix("_r")).unwrap_or(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PoseSet;

    fn humanoid() -> Puppet {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/rigs/humanoid.puppet.toml")).unwrap();
        Puppet::from_baked_toml(&text).unwrap()
    }

    #[test]
    fn a_fall_starts_where_it_was_and_ends_dead() {
        let puppet = humanoid();
        let set = PoseSet::shipped("biped").unwrap();
        let (stand, dead) = (set.get("stand").unwrap(), set.get("dead").unwrap());
        let fall = Fall::new(&puppet, stand, dead, 7);
        let h0 = puppet.pose(&fall.at(0.0)).bounds()[1][1];
        assert!((h0 - puppet.pose(stand).bounds()[1][1]).abs() < 1e-3, "starts standing");
        assert_eq!(fall.at(FALL_SECS), *dead);
        // It goes down: height falls through the fall, and by impact it is
        // most of the way.
        let h = |t: f32| puppet.pose(&fall.at(t)).bounds()[1][1];
        let lying = puppet.pose(dead).bounds()[1][1];
        assert!(h(0.3) < h0 && h(IMPACT_SECS) < h0 * 0.7, "{} {} {}", h0, h(0.3), h(IMPACT_SECS));
        assert!((h(FALL_SECS - 0.01) - lying).abs() < 0.25 * h0, "settled: {} vs {lying}", h(FALL_SECS - 0.01));
        // The limbs overshoot their dead angle and swing back: some frame
        // after impact differs from the dead pose, the last one barely.
        assert!(fall.at(0.8) != *dead);
        // Never through the floor.
        for k in 0..40 {
            let b = puppet.pose(&fall.at(k as f32 * FALL_SECS / 40.0)).bounds();
            assert!(b[0][1].abs() < 1e-3, "grounded at every frame");
        }
    }

    /// From a pose already low (kneeling, sitting, lying) the fall goes on
    /// down from there: straightening the folded legs first, and turning
    /// straight across from lying face down to face up, stood the body back
    /// up before it fell. Into the dead pose `dead_hash` picks, it never
    /// rises past where it starts or ends by more than a tenth of its
    /// height.
    #[test]
    fn a_fall_from_a_low_pose_never_rises() {
        let puppet = humanoid();
        let set = PoseSet::shipped("biped").unwrap();
        let h = |p: &Pose| puppet.pose(p).bounds()[1][1];
        for from in set.names() {
            if from.starts_with("dead") {
                continue;
            }
            for seed in 0..4u64 {
                let start = set.get(from).unwrap();
                let dead = set.dead_name(set.dead_hash(start, seed)).unwrap();
                let fall = Fall::new(&puppet, start, set.get(dead).unwrap(), seed);
                let (h0, h1) = (h(start), h(set.get(dead).unwrap()));
                let highest = (1..60).map(|k| h(&fall.at(k as f32 * FALL_SECS / 60.0))).fold(0.0f32, f32::max);
                assert!(highest < h0.max(h1) + 0.1 * puppet.height, "{from} -> {dead}: from {h0:.2} to {h1:.2} ft, up to {highest:.2} on the way");
            }
        }
        // Lying on its back, it stays on its back.
        assert_eq!(set.dead_name(set.dead_hash(set.get("supine").unwrap(), 0)), Some("dead_2"));
        assert_eq!(set.dead_name(set.dead_hash(set.get("fallen_side").unwrap(), 1)), Some("dead"));
        // Standing, the caller's variety picks.
        let stand = set.get("stand").unwrap();
        assert_ne!(set.dead_name(set.dead_hash(stand, 0)), set.dead_name(set.dead_hash(stand, 1)));
    }

    /// The low turn starts at one root and ends at the other, exactly.
    #[test]
    fn the_low_turn_meets_both_ends() {
        let set = PoseSet::shipped("biped").unwrap();
        let roots: Vec<Quat> = ["stand", "prone", "supine", "fallen_side", "dead", "dead_2"].iter().map(|n| set.get(n).unwrap().root).collect();
        let near = |a: Quat, b: Quat| {
            let (ma, mb) = (quat_to_m3(a), quat_to_m3(b));
            (0..3).all(|i| (0..3).all(|j| (ma[i][j] - mb[i][j]).abs() < 1e-3))
        };
        for (i, a) in roots.iter().enumerate() {
            for (j, b) in roots.iter().enumerate() {
                for long in [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] {
                    assert!(near(turn_low(*a, *b, long, 0.0), *a), "{i}->{j} {long:?}: starts off");
                    assert!(near(turn_low(*a, *b, long, 1.0), *b), "{i}->{j} {long:?}: ends off");
                }
            }
        }
    }

    #[test]
    fn the_topple_accelerates_then_bounces_once() {
        assert_eq!(topple(0.0), 0.0);
        let early = topple(IMPACT_SECS * 0.25);
        let late = topple(IMPACT_SECS * 0.75) - topple(IMPACT_SECS * 0.5);
        assert!(early < late, "slow then fast");
        assert!((topple(IMPACT_SECS) - 1.0).abs() < 1e-5);
        let bounce = (0..50).map(|k| topple(IMPACT_SECS + k as f32 * 0.01)).fold(1.0f32, f32::min);
        assert!(bounce < 0.99 && bounce > 0.9, "{bounce}");
        assert!((topple(FALL_SECS) - 1.0).abs() < 0.01);
        // A spring overshoots.
        assert!((0..100).map(|k| spring(k as f32 * 0.02, 8.0, 0.32)).any(|p| p > 1.05));
    }
}
