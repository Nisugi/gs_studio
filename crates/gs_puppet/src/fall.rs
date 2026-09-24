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
//! * Every bone follows on its own SPRING (under-damped), released a moment
//!   after the bone it hangs from -- the legs buckle first, the trunk goes,
//!   and the arms, head and tail trail it, swing past where they come to
//!   rest and settle back. The impact kicks them once more.
//!
//! Grounding still runs per frame (`Puppet::pose` drops the lowest surface
//! onto y = 0), so the figure never sinks through the floor. The motion is
//! a pure function of time and a seed: the same death plays the same way,
//! and any frame can be drawn without the ones before it.

use std::collections::BTreeMap;

use crate::math::{slerp, Quat};
use crate::pose::{BonePose, Pose};
use crate::puppet::Puppet;

/// When the body hits the ground.
pub const IMPACT_SECS: f32 = 0.55;
/// When the fall has settled into the dead pose.
pub const FALL_SECS: f32 = 1.7;

/// One creature's fall.
#[derive(Debug, Clone)]
pub struct Fall {
    from: Pose,
    to: Pose,
    /// Pose key -> (release delay s, spring angular frequency rad/s,
    /// how hard the impact kicks it).
    springs: BTreeMap<String, (f32, f32, f32)>,
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
    pub fn new(puppet: &Puppet, from: &Pose, to: &Pose, seed: u64) -> Fall {
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
        Fall { from: from.clone(), to: to.clone(), springs }
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
            if tau > 0.0 {
                p += kick * (-6.0 * tau).exp() * (15.0 * tau).sin();
            }
            // Nothing hangs back much once the body is down.
            let p = p.max(root_p * 0.6);
            let (a, b) = (self.from.bone(&k), self.to.bone(&k));
            bones.insert(k, BonePose { rot: slerp(a.rot, b.rot, p), bend: lerp_bend(&a.bend, &b.bend, p) });
        }
        Pose { root: slerp(self.from.root, self.to.root, root_p) as Quat, bones }
    }
}

fn stem(key: &str) -> &str {
    key.strip_suffix("_l").or_else(|| key.strip_suffix("_r")).unwrap_or(key)
}

fn lerp_bend(a: &[f32], b: &[f32], t: f32) -> Vec<f32> {
    let n = a.len().max(b.len());
    let at = |v: &[f32], i: usize| v.get(i).copied().unwrap_or(0.0);
    (0..n).map(|i| at(a, i) + (at(b, i) - at(a, i)) * t).collect()
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
