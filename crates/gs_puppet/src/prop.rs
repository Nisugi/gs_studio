//! Held props: a weapon or a shield in a puppet's hand.
//!
//! What can be held, and how each thing is shaped, is data:
//! `data/props/weapons.toml` (see its header). A SHAPE is a few parts laid
//! along the prop's axis -- a pommel, a grip, a guard, a blade -- each a
//! run of elliptical stations like a body segment's, in units of the
//! weapon's length; a WEAPON (one per Lich weapon base: broadsword,
//! morning star, ...) names its shape, its length in feet and every name
//! the game calls it by.
//!
//! [`Puppet::holding`] builds the prop onto a puppet as extra segments hung
//! off the hand ([`Kind::Prop`]), after the body. Nothing downstream needs
//! to know: posing carries them with the hand (their bone is in no pose),
//! the skin draws them depth-tested with the body in their own colours, a
//! fall swings them with the arm. They are never textured, never baked and
//! never lift the figure off the ground.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::math::{add, cross, dot, norm_or, scale, sub, V3};
use crate::puppet::{Kind, Puppet, Segment, Station};

/// The props file this crate ships, embedded.
pub const SHIPPED_PROPS: &str = include_str!("../../../data/props/weapons.toml");

/// Props are sized for a creature this tall; a bigger one holds a bigger one.
pub const PROP_SCALE_HEIGHT_FT: f32 = 6.0;

/// How a prop sits in a hanging hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Grip {
    /// Ahead of the creature, a little down: swords, axes, maces.
    Forward,
    /// Upright through the fist: staves, polearms, bows.
    Up,
    /// Hanging from the fist: whips, flails.
    Down,
    /// A disc on the forearm, facing out.
    Shield,
}

/// Which hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hand {
    Right,
    Left,
}

impl Hand {
    fn suffix(self) -> &'static str {
        match self {
            Hand::Right => "_r",
            Hand::Left => "_l",
        }
    }

    fn word(self) -> &'static str {
        match self {
            Hand::Right => "right",
            Hand::Left => "left",
        }
    }
}

/// One part of a shape, in units of the weapon's length.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub at: [f32; 2],
    pub w: [f32; 2],
    pub d: [f32; 2],
    pub off: [f32; 2],
    pub round: bool,
    pub color: [u8; 3],
}

/// A weapon's shape, ready to build: its grip, its length in feet (for a
/// [`PROP_SCALE_HEIGHT_FT`] creature) and its parts.
#[derive(Debug, Clone, PartialEq)]
pub struct Prop {
    pub grip: Grip,
    pub length: f32,
    pub parts: Vec<Part>,
}

/// One weapon base, as `[weapon.<key>]` gives it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Weapon {
    #[serde(skip)]
    pub key: String,
    pub base: String,
    pub category: String,
    pub shape: String,
    pub length: f32,
    #[serde(default = "one")]
    pub hands: u8,
    #[serde(default)]
    pub names: Vec<String>,
}

fn one() -> u8 {
    1
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum Span {
    One(f32),
    Two([f32; 2]),
}

impl Span {
    fn pair(&self) -> [f32; 2] {
        match self {
            Span::One(v) => [*v, *v],
            Span::Two(p) => *p,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PartDef {
    at: [f32; 2],
    w: Span,
    #[serde(default)]
    d: Option<Span>,
    #[serde(default)]
    off: Option<Span>,
    #[serde(default)]
    round: bool,
    material: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ShapeDef {
    grip: Grip,
    parts: Vec<PartDef>,
}

#[derive(Debug, Deserialize)]
struct PropFile {
    #[serde(default)]
    material: BTreeMap<String, [u8; 3]>,
    #[serde(default)]
    shape: BTreeMap<String, ShapeDef>,
    #[serde(default)]
    weapon: BTreeMap<String, Weapon>,
}

/// Every shape and weapon of a props file, checked.
#[derive(Debug, Clone)]
pub struct PropSet {
    shapes: BTreeMap<String, (Grip, Vec<Part>)>,
    pub weapons: BTreeMap<String, Weapon>,
    /// Lower-case name -> weapon key, every base and alias.
    names: BTreeMap<String, String>,
}

impl PropSet {
    pub fn from_toml(text: &str) -> Result<PropSet, String> {
        let file: PropFile = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut shapes = BTreeMap::new();
        for (name, def) in file.shape {
            let mut parts = Vec::new();
            for (k, p) in def.parts.iter().enumerate() {
                let where_ = format!("shape '{name}', part {}", k + 1);
                let color = *file.material.get(&p.material).ok_or(format!("{where_}: no material '{}'", p.material))?;
                if !(p.at[0] < p.at[1]) {
                    return Err(format!("{where_}: at = {:?} is not from < to", p.at));
                }
                let w = p.w.pair();
                let d = p.d.as_ref().map_or(w, Span::pair);
                let off = p.off.as_ref().map_or([0.0; 2], Span::pair);
                if [w, d].iter().flatten().any(|v| !(v.is_finite() && *v >= 0.0)) {
                    return Err(format!("{where_}: a width is not a size"));
                }
                parts.push(Part { at: p.at, w, d, off, round: p.round, color });
            }
            if parts.is_empty() {
                return Err(format!("shape '{name}' has no parts"));
            }
            shapes.insert(name, (def.grip, parts));
        }
        let mut weapons = BTreeMap::new();
        let mut names = BTreeMap::new();
        for (key, mut w) in file.weapon {
            if !shapes.contains_key(&w.shape) {
                return Err(format!("weapon '{key}': no shape '{}'", w.shape));
            }
            if !(w.length.is_finite() && w.length > 0.0) {
                return Err(format!("weapon '{key}': length {} is not a length", w.length));
            }
            w.key = key.clone();
            for n in w.names.iter().chain([&w.base, &key]) {
                names.entry(n.to_lowercase()).or_insert_with(|| key.clone());
            }
            weapons.insert(key, w);
        }
        Ok(PropSet { shapes, weapons, names })
    }

    /// The props file this crate ships.
    pub fn shipped() -> &'static PropSet {
        static SET: OnceLock<PropSet> = OnceLock::new();
        SET.get_or_init(|| PropSet::from_toml(SHIPPED_PROPS).expect("the shipped props file parses"))
    }

    /// A weapon by its key, base or any of its names, any case.
    pub fn weapon(&self, name: &str) -> Option<&Weapon> {
        let key = self.names.get(&name.trim().to_lowercase())?;
        self.weapons.get(key)
    }

    /// The weapon a piece of text names, if any: the longest weapon name
    /// that appears in it as whole words ("a rusty carp's tongue").
    pub fn find_in(&self, text: &str) -> Option<&Weapon> {
        let text = text.to_lowercase();
        let bounded = |at: usize, len: usize| {
            let before = text[..at].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
            let after = text[at + len..].chars().next().is_none_or(|c| !c.is_alphanumeric());
            before && after
        };
        let mut best: Option<(usize, &str)> = None;
        for (name, key) in &self.names {
            let found = text.match_indices(name.as_str()).any(|(at, _)| bounded(at, name.len()));
            if found && best.is_none_or(|(n, _)| name.len() > n) {
                best = Some((name.len(), key.as_str()));
            }
        }
        best.and_then(|(_, k)| self.weapons.get(k))
    }

    /// A weapon's shape, ready for [`Puppet::holding`].
    pub fn prop(&self, weapon: &Weapon) -> Option<Prop> {
        let (grip, parts) = self.shapes.get(&weapon.shape)?;
        Some(Prop { grip: *grip, length: weapon.length, parts: parts.clone() })
    }

    /// Every shape name.
    pub fn shape_names(&self) -> impl Iterator<Item = &str> {
        self.shapes.keys().map(String::as_str)
    }
}

/// Stations along one part: straight parts need few, a ball more.
const STRAIGHT_STATIONS: usize = 4;
const ROUND_STATIONS: usize = 9;

impl Puppet {
    /// How much bigger than a [`PROP_SCALE_HEIGHT_FT`] creature's this
    /// puppet's props are.
    pub fn prop_scale(&self) -> f32 {
        (self.height / PROP_SCALE_HEIGHT_FT).clamp(0.3, 5.0)
    }

    /// This puppet with `held` built onto its hands (see the module docs),
    /// sized by [`Puppet::prop_scale`]. A hand the puppet does not have
    /// holds nothing; a shield goes on the forearm whichever hand it is.
    pub fn holding(&self, held: &[(Hand, &Prop)]) -> Puppet {
        let mut out = self.clone();
        for (hand, prop) in held {
            let Some(grip) = grip_frame(self, *hand, prop.grip) else {
                continue;
            };
            let group = out.groups.len();
            out.groups.push(format!("held_{}", hand.word()));
            let length = prop.length * self.prop_scale();
            for (k, part) in prop.parts.iter().enumerate() {
                let station = |t: f32, bulge: f32| {
                    let lerp = |p: [f32; 2]| p[0] + (p[1] - p[0]) * t;
                    let along = part.at[0] + (part.at[1] - part.at[0]) * t;
                    let c = add(add(grip.at, scale(grip.axis, along * length)), scale(grip.across, lerp(part.off) * length));
                    Station { c, w: (lerp(part.w) * length * bulge).max(0.004), d: (lerp(part.d) * length * bulge).max(0.004) }
                };
                let stations: Vec<Station> = if part.round {
                    (0..ROUND_STATIONS)
                        .map(|i| {
                            let t = i as f32 / (ROUND_STATIONS - 1) as f32;
                            station(t, (1.0 - (2.0 * t - 1.0).powi(2)).max(0.0).sqrt())
                        })
                        .collect()
                } else {
                    (0..STRAIGHT_STATIONS).map(|i| station(i as f32 / (STRAIGHT_STATIONS - 1) as f32, 1.0)).collect()
                };
                out.segments.push(Segment {
                    name: format!("held_{}.{k}", hand.word()),
                    bone: format!("held_{}", hand.word()),
                    part: None,
                    parent: Some(grip.segment),
                    group,
                    kind: Kind::Prop { color: part.color },
                    chain_index: 0,
                    head: stations[0].c,
                    tail: stations[stations.len() - 1].c,
                    axis_w: grip.across,
                    axis_d: grip.flat,
                    hinge: grip.flat,
                    stations,
                    bare: 0,
                    hidden: false,
                });
            }
        }
        out
    }
}

/// Where and which way a prop sits: all rest-space.
struct GripFrame {
    /// The segment it hangs from.
    segment: usize,
    at: V3,
    /// Along the prop, grip -> tip.
    axis: V3,
    /// Across its flat (`off` and `w`), and through it (`d`).
    across: V3,
    flat: V3,
}

/// The hand's last segment (or, with no hand, the arm's) and the prop's
/// frame in it.
fn grip_frame(puppet: &Puppet, hand: Hand, grip: Grip) -> Option<GripFrame> {
    let side = hand.suffix();
    let last = |want: &dyn Fn(&Segment) -> bool| {
        puppet
            .segments
            .iter()
            .enumerate()
            .filter(|(_, s)| s.kind.is_body() && s.bone.ends_with(side) && want(s))
            .max_by_key(|(_, s)| s.chain_index)
            .map(|(i, _)| i)
    };
    let arm_part = match hand {
        Hand::Right => "rightArm",
        Hand::Left => "leftArm",
    };
    let (segment, in_hand) = match last(&|s| s.kind == Kind::Hand || s.part.as_deref().is_some_and(|p| p.ends_with("Hand"))) {
        Some(i) => (i, true),
        None => (last(&|s| s.part.as_deref() == Some(arm_part))?, false),
    };
    let s = &puppet.segments[segment];
    let down = norm_or(sub(s.tail, s.head), [0.0, -1.0, 0.0]);
    let forward = norm_or(sub([0.0, 0.0, 1.0], scale(down, down[2])), [0.0, 0.0, 1.0]);
    let at = if in_hand { add(s.head, scale(sub(s.tail, s.head), 0.55)) } else { s.tail };
    let out_x = if hand == Hand::Left { 1.0 } else { -1.0 };
    let axis = match grip {
        Grip::Forward => norm_or(add(scale(forward, 0.86), scale(down, 0.5)), forward),
        Grip::Up => norm_or(add(scale(down, -1.0), scale(forward, 0.12)), [0.0, 1.0, 0.0]),
        Grip::Down => norm_or(add(down, scale(forward, 0.3)), down),
        Grip::Shield => [out_x, 0.0, 0.0],
    };
    // The flat's edge side: down for a blade held forward, forward for one
    // held upright, up the arm for a shield.
    let edge = match grip {
        Grip::Forward | Grip::Down => down,
        Grip::Up => forward,
        Grip::Shield => scale(down, -1.0),
    };
    let across = norm_or(sub(edge, scale(axis, dot(edge, axis))), [0.0, 1.0, 0.0]);
    let flat = norm_or(cross(axis, across), [1.0, 0.0, 0.0]);
    // A shield sits on the forearm, not in the fist.
    let at = if grip == Grip::Shield { add(s.head, scale(sub(s.head, s.tail), 0.6)) } else { at };
    Some(GripFrame { segment, at, axis, across, flat })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Pose;

    fn humanoid() -> Puppet {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/rigs/humanoid.puppet.toml")).unwrap();
        Puppet::from_baked_toml(&text).unwrap()
    }

    #[test]
    fn the_shipped_file_parses_and_names_find_their_weapon() {
        let set = PropSet::shipped();
        assert!(set.weapons.len() > 50, "{}", set.weapons.len());
        assert_eq!(set.weapon("Carp's Tongue").unwrap().base, "broadsword");
        assert_eq!(set.weapon("morning star").unwrap().shape, "morning_star");
        assert!(set.weapon("closed fist").is_none(), "a fist is the hand itself");
        assert_eq!(set.find_in("a rusty spiked mace").unwrap().base, "morning star", "the longer name wins over 'mace'");
        assert_eq!(set.find_in("an orcish short sword").unwrap().base, "short sword");
        assert!(set.find_in("a scimitarish thing").is_none(), "whole words only");
        for w in set.weapons.values() {
            assert!(set.prop(w).is_some(), "{} has a shape", w.key);
        }
    }

    #[test]
    fn a_bad_file_says_what_is_wrong() {
        let bad = "[material]\nsteel = [1, 2, 3]\n[shape.s]\ngrip = \"forward\"\nparts = [{ at = [0.0, 1.0], w = 0.1, material = \"gold\" }]\n";
        assert!(PropSet::from_toml(bad).unwrap_err().contains("no material 'gold'"));
        let bad = "[weapon.x]\nbase = \"x\"\ncategory = \"edged\"\nshape = \"nope\"\nlength = 1.0\n";
        assert!(PropSet::from_toml(bad).unwrap_err().contains("no shape 'nope'"));
    }

    #[test]
    fn a_held_sword_follows_the_hand_and_never_lifts_the_body() {
        let puppet = humanoid();
        let set = PropSet::shipped();
        let sword = set.prop(set.weapon("broadsword").unwrap()).unwrap();
        let armed = puppet.holding(&[(Hand::Right, &sword)]);
        let body = puppet.segments.len();
        assert_eq!(armed.segments.len(), body + sword.parts.len());
        assert!(armed.segments[body..].iter().all(|s| s.kind.is_prop() && s.parent.is_some()));
        // Held in the right hand (negative x), pointing ahead (+z).
        let tip = armed.segments.last().unwrap().tail;
        let hand = &armed.segments[armed.segments[body].parent.unwrap()];
        assert!(hand.bone.ends_with("_r") && tip[0] < 0.0 && tip[2] > hand.tail[2] + 1.5, "tip {tip:?}");

        // It swings with the arm, and the ground stays where the feet are.
        let mut pose = Pose::default();
        let arm_up = crate::BonePose::euler([-150.0, 0.0, 0.0], vec![]);
        pose.bones.insert("arm_r".into(), arm_up);
        let (plain, held) = (puppet.pose(&pose), armed.pose(&pose));
        assert_eq!(plain.lift, held.lift, "a prop never lifts the figure");
        let raised = held.point(armed.segments.len() - 1, tip);
        assert!(raised[1] > puppet.height, "the sword is raised with the arm: {raised:?}");

        // And it is not baked.
        assert_eq!(armed.to_baked_toml().unwrap(), puppet.to_baked_toml().unwrap());
    }

    #[test]
    fn a_held_sword_is_drawn_in_its_own_colour() {
        let puppet = humanoid();
        let set = PropSet::shipped();
        let sword = set.prop(set.weapon("broadsword").unwrap()).unwrap();
        let armed = puppet.holding(&[(Hand::Right, &sword)]);
        let cam = crate::Camera::framing(puppet.height * 1.3, [300.0, 300.0]);
        let look = crate::Look { yaw: 90.0, ..crate::Look::default() };
        let skin = armed.pose(&Pose::default()).skin(&|p| cam.project(p), &look);
        let plain = puppet.pose(&Pose::default()).skin(&|p| cam.project(p), &look);
        let steel = |s: &crate::Skin| {
            s.rgba.chunks_exact(4).filter(|p| p[3] > 200 && p[2] > p[0] + 6 && p[2] > 120).count()
        };
        assert!(steel(&skin) > 40 && steel(&plain) == 0, "steel pixels: {} vs {}", steel(&skin), steel(&plain));
    }
}
