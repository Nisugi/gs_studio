//! A puppet as LAYERS of Gaussians anchored to its skin.
//!
//! The skin's surface (every station ring, sampled round as `splat::cloud`
//! samples it) is laid out once per puppet as a grid of [`Anchor`]s: a
//! surface address (segment, station, ring angle) and what does not change
//! with the pose (where it is on the surface layout, which way it faces at
//! rest, how far up the figure). Posed, each anchor is a column through the
//! body, and what stands in it depends on the figure's state:
//!
//! - the SKIN dot on the surface, wearing the painted sheet and the baked
//!   marks (scars, bruises, statuses) as the smooth skin does;
//! - where a wound has opened it, the surface SUNK into the body. A wound
//!   is a few placed [`Gash`]es per part (two scratches at rank 1, gashes to
//!   the flesh at rank 2, wider ones to the bone at rank 3), each a smooth
//!   pit: nothing at its rim, deepest along its middle. A dot there sits at
//!   the pit's depth and is what the body is made of that deep -- skin,
//!   FAT, FLESH, BONE -- so the layers show as bands down its walls; where
//!   the wall is steep more dots are stacked up it, every dot faces the way
//!   the pit's surface does (so a light catches one wall and not the
//!   other) and is darker the deeper it is. The skin round it is bloodied,
//!   bruised and a little raised. What made the wound ([`Cause`], per
//!   part) decides its shape: a slash gashes, a puncture bores a hole, a
//!   crush dents and bruises without opening, fire chars a crater whose
//!   cracks glow, cold leaves a crust of frost, lightning a branching burn.
//!   A severed limb's stump ends in a cross-section, flesh round bone;
//! - a wound ARRIVES ([`Fresh`]: a part just hit, and how long ago): it
//!   opens over a third of a second in a flash, a burn cools from white
//!   hot to embers, frost grows, and LOOSE dots leave the body from it --
//!   blood, embers, chips of ice, sparks -- thrown out, falling, lying
//!   where they land until the hit is no longer fresh ([`FRESH_SECS`]);
//! - a COAT dot standing off the surface where a [`Coating`] has reached:
//!   it climbs the figure from the ground and thickens behind its front, so
//!   the outline grows.
//!
//! The skin's dots need not be made here: a FITTED cloud (a real splat of
//! the creature, `splat::load_ply`) can be worn in their place
//! ([`LayerCloud::fit`], [`LayerLook::fitted`]). Each of its Gaussians is
//! given the surface address it lies nearest, and so its part, its facing
//! and its place in a wound -- it is tinted, sunk, cut away, coated and
//! thrown off exactly as a made dot is, in its own colour and shape.
//!
//! Rings turn with `Posed::ring_xf` as the skin's do (through
//! `draw::surfaces`), so joints bend alike. The dots are [`Gaussian`]s, drawn
//! by `splat::render` or a GPU host through `splat::prepare`; each keeps the
//! anchor it stands on ([`Dot::anchor`]), which is what lets a later step
//! have it let go and find its way back.

use crate::draw::{Look, Surface, BODY_DEAD};
use crate::math::{add, cross, dot, len, lerp as lerp3, norm_or, scale, sub, V3};
use crate::pose::Posed;
use crate::model::Puppet;
use crate::skin::{smoothstep, Effects};
use crate::splat::{BoundCloud, Gaussian, SplatParams};
use crate::uv::{Texture, UvLayout};
use crate::wounds;

/// A coating's full thickness by default, feet.
pub const COAT_FT: f32 = 0.08;
/// How far a web's strand stands off the skin, and half its width, feet.
const WEB_LIFT_FT: f32 = 0.006;
const WEB_HALF_FT: f32 = 0.0035;
/// How wet ice shows a lamp (it shines as wet does: the v2 pass's glint).
const ICE_SHINE: f32 = 0.6;
/// A poisoned figure's sweat, as wet.
const POISON_SWEAT: f32 = 0.35;
/// Blood a dead figure bled a while since, dark and drying.
const OLD_BLOOD: [f32; 3] = [58.0, 10.0, 12.0];
/// A dead figure's skin, a waxy grey-blue.
const DEAD_SKIN: [f32; 3] = [128.0, 142.0, 150.0];
/// An invisible figure's skin: how much of it shows only edge-on
/// (`Gaussian::rim`), and the pale it is turned toward.
const INVISIBLE_RIM: f32 = 0.99;
const GHOST_RGB: [f32; 3] = [214.0, 232.0, 250.0];

/// How deep the skin goes, and the fat under it, feet: below is flesh.
const SKIN_FT: f32 = 0.006;
const FAT_FT: f32 = 0.016;
/// The girth (mean radius, feet) a cause's wound sizes are for; a thinner
/// part's are smaller in proportion, within these.
const GASH_GIRTH_FT: f32 = 0.35;
const GASH_SCALE: (f32, f32) = (0.4, 1.2);
/// A gash is no deeper than this share of the part's radius.
const GASH_MAX_OF_RADIUS: f32 = 0.45;
/// How far the skin round an open gash stands up, feet.
const LIP_FT: f32 = 0.006;
/// A part with fewer anchors than this is too small to gash.
const GASH_MIN_ANCHORS: usize = 150;
/// The bone's share of a stump's cross-section, by radius.
const STUMP_BONE: f32 = 0.4;
/// Near a mark (within this, in its `across`) the surface is drawn finer:
/// each anchor as `DETAIL` x `DETAIL` smaller dots, so a wound's edge is
/// not a whole dot wide.
const DETAIL_REACH: f32 = 1.6;
const DETAIL: usize = 2;
/// How thick a cold wound's frost stands, feet.
const FROST_FT: f32 = 0.03;

/// How long a hit shows as fresh, seconds: its loose dots lie that long.
pub const FRESH_SECS: f32 = 4.0;
/// A wound opens over this long, its flash dies away in this, a burn cools
/// in this (each to 1/e), seconds; loose dots fall at this, ft/s2.
const OPEN_SECS: f32 = 0.35;
const FLASH_SECS: f32 = 0.18;
const COOL_SECS: f32 = 1.6;
const FALL_FT_S2: f32 = 24.0;

/// What made a wound: it decides the wound's shape and what it throws off.
/// The game's damage types (its critical tables'), by their names there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Cause {
    #[default]
    Slash,
    Puncture,
    Crush,
    Fire,
    Cold,
    Lightning,
    Impact,
    /// Unarmed: a jab, a punch, a kick, a hold (the game's `ucs_*` tables;
    /// `grapple` is a hold too).
    Jab,
    Punch,
    Kick,
    Grapple,
    Unbalance,
    Acid,
    Steam,
    Plasma,
    Disintegrate,
    Disruption,
    NonCorporeal,
    Vacuum,
}

/// A burn's colours: the skin scorched round it, the crust over it (and how
/// much of it covers), its embers' colours cold to hot, how much they glow
/// (0: none -- a scald, acid) and how much of that glow stays once cool.
struct Burn {
    scorch: [f32; 3],
    crust: [f32; 3],
    cover: f32,
    ember: ([f32; 3], [f32; 3]),
    embers: f32,
    keep: f32,
}

/// What a hit throws off a mark: how many dots, how fast (ft/s), how big
/// (feet), their colour and opacity, whether they glow; `rises`: they drift
/// up and fade over that many seconds instead of falling; `scatter`: they
/// start all over the mark (in its radii) instead of at its middle.
struct Spray {
    count: i32,
    speed: f32,
    size: f32,
    rgb: [f32; 3],
    soft: f32,
    glows: bool,
    rises: Option<f32>,
    scatter: f32,
}

impl Cause {
    pub const ALL: [Cause; 19] = [
        Cause::Slash,
        Cause::Puncture,
        Cause::Crush,
        Cause::Fire,
        Cause::Cold,
        Cause::Lightning,
        Cause::Impact,
        Cause::Jab,
        Cause::Punch,
        Cause::Kick,
        Cause::Grapple,
        Cause::Unbalance,
        Cause::Acid,
        Cause::Steam,
        Cause::Plasma,
        Cause::Disintegrate,
        Cause::Disruption,
        Cause::NonCorporeal,
        Cause::Vacuum,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Cause::Slash => "slash",
            Cause::Puncture => "puncture",
            Cause::Crush => "crush",
            Cause::Fire => "fire",
            Cause::Cold => "cold",
            Cause::Lightning => "lightning",
            Cause::Impact => "impact",
            Cause::Jab => "jab",
            Cause::Punch => "punch",
            Cause::Kick => "kick",
            Cause::Grapple => "grapple",
            Cause::Unbalance => "unbalance",
            Cause::Acid => "acid",
            Cause::Steam => "steam",
            Cause::Plasma => "plasma",
            Cause::Disintegrate => "disintegrate",
            Cause::Disruption => "disruption",
            Cause::NonCorporeal => "non_corporeal",
            Cause::Vacuum => "vacuum",
        }
    }

    /// By the game's name for it: `ucs_kick` is a kick, `non-corporeal` and
    /// `noncorporeal` are `non_corporeal`.
    pub fn from_name(name: &str) -> Option<Cause> {
        let name = name.trim().to_ascii_lowercase().replace(['-', ' '], "_");
        let name = name.strip_prefix("ucs_").unwrap_or(&name);
        let name = if name == "noncorporeal" { "non_corporeal" } else { name };
        Cause::ALL.into_iter().find(|c| c.name() == name)
    }

    /// The kind of mark it makes: the blunt ones bruise as a crush does,
    /// acid, steam and plasma burn as fire does, disruption branches as
    /// lightning does; the rest are their own.
    fn base(self) -> Cause {
        match self {
            Cause::Impact | Cause::Jab | Cause::Punch | Cause::Kick | Cause::Grapple | Cause::Unbalance => Cause::Crush,
            Cause::Acid | Cause::Steam | Cause::Plasma => Cause::Fire,
            Cause::Disruption => Cause::Lightning,
            other => other,
        }
    }

    /// A wound of `rank` on a part of trunk girth: how many marks, half a
    /// mark's length, half its width and its depth, feet.
    fn shape(self, rank: u8) -> (usize, f32, f32, f32) {
        let r = rank.clamp(1, 3) as usize - 1;
        match self {
            // rank 1 only scratches
            Cause::Slash => [(2, 0.14, 0.012, 0.0), (2, 0.22, 0.035, 0.035), (3, 0.3, 0.06, 0.075)][r],
            Cause::Puncture => [(2, 0.018, 0.018, 0.012), (2, 0.035, 0.035, 0.05), (3, 0.05, 0.05, 0.09)][r],
            // dents, not holes: a crush, a wider blow, a fist's knuckles, a
            // boot, the fingers of a hold, a fall
            Cause::Crush => [(1, 0.16, 0.13, 0.0), (1, 0.24, 0.2, 0.012), (2, 0.3, 0.24, 0.025)][r],
            Cause::Impact => [(1, 0.18, 0.15, 0.0), (1, 0.26, 0.22, 0.014), (2, 0.32, 0.26, 0.03)][r],
            Cause::Jab => [(1, 0.05, 0.045, 0.0), (2, 0.06, 0.055, 0.006), (3, 0.07, 0.06, 0.012)][r],
            Cause::Punch => [(1, 0.08, 0.07, 0.0), (2, 0.1, 0.09, 0.008), (2, 0.13, 0.11, 0.018)][r],
            Cause::Kick => [(1, 0.13, 0.1, 0.0), (1, 0.18, 0.14, 0.012), (2, 0.22, 0.17, 0.028)][r],
            Cause::Grapple => [(2, 0.12, 0.022, 0.0), (3, 0.15, 0.026, 0.004), (4, 0.18, 0.03, 0.008)][r],
            Cause::Unbalance => [(1, 0.2, 0.16, 0.0), (1, 0.26, 0.2, 0.0), (2, 0.3, 0.22, 0.008)][r],
            Cause::Fire => [(1, 0.1, 0.085, 0.0), (1, 0.17, 0.14, 0.012), (2, 0.23, 0.19, 0.025)][r],
            // eaten in; scalded wide and shallow; burnt deep
            Cause::Acid => [(2, 0.06, 0.05, 0.004), (2, 0.11, 0.09, 0.02), (3, 0.15, 0.12, 0.04)][r],
            Cause::Steam => [(1, 0.14, 0.12, 0.0), (1, 0.22, 0.18, 0.0), (2, 0.28, 0.22, 0.006)][r],
            Cause::Plasma => [(1, 0.08, 0.07, 0.006), (1, 0.13, 0.11, 0.03), (2, 0.18, 0.15, 0.055)][r],
            Cause::Cold => [(1, 0.1, 0.09, 0.0), (1, 0.18, 0.15, 0.0), (2, 0.24, 0.2, 0.0)][r],
            Cause::Lightning => [(1, 0.25, 0.012, 0.0), (1, 0.4, 0.016, 0.004), (2, 0.5, 0.02, 0.006)][r],
            Cause::Disruption => [(1, 0.22, 0.014, 0.0), (2, 0.34, 0.018, 0.0), (3, 0.42, 0.022, 0.0)][r],
            Cause::Disintegrate => [(1, 0.09, 0.08, 0.006), (1, 0.16, 0.13, 0.02), (2, 0.22, 0.18, 0.04)][r],
            Cause::NonCorporeal => [(1, 0.14, 0.12, 0.0), (1, 0.22, 0.18, 0.0), (2, 0.28, 0.22, 0.0)][r],
            // its "depth" is how far it bulges OUT
            Cause::Vacuum => [(1, 0.1, 0.09, 0.006), (1, 0.16, 0.13, 0.012), (2, 0.2, 0.17, 0.02)][r],
        }
    }

    /// It opens the skin (the others mark, dent or coat it).
    fn opens(self) -> bool {
        matches!(self, Cause::Slash | Cause::Puncture)
    }

    /// How ragged a round mark's rim is.
    fn ragged(self) -> f32 {
        match self {
            Cause::Puncture | Cause::Jab | Cause::Punch => 0.15,
            Cause::Fire | Cause::Acid | Cause::Plasma | Cause::Cold | Cause::Disintegrate => 0.5,
            _ => 0.3,
        }
    }

    /// The flash of the hit: its colour, and how far it reaches (`across`).
    fn flash(self) -> ([f32; 3], f32) {
        match self {
            Cause::Slash => ([255.0, 236.0, 224.0], 3.0),
            Cause::Puncture => ([255.0, 236.0, 224.0], 2.4),
            Cause::Fire => ([255.0, 214.0, 130.0], 1.8),
            Cause::Plasma => ([236.0, 214.0, 255.0], 2.2),
            Cause::Steam => ([255.0, 244.0, 240.0], 1.3),
            Cause::Acid => ([226.0, 240.0, 150.0], 1.4),
            Cause::Cold => ([226.0, 242.0, 255.0], 1.6),
            Cause::Lightning => ([214.0, 232.0, 255.0], 6.0),
            Cause::Disruption => ([190.0, 150.0, 255.0], 5.0),
            Cause::Disintegrate => ([230.0, 220.0, 255.0], 1.6),
            Cause::NonCorporeal => ([220.0, 232.0, 250.0], 1.5),
            _ => ([255.0, 240.0, 230.0], 1.4),
        }
    }

    fn burn(self) -> Burn {
        let fire = Burn { scorch: [168.0, 58.0, 44.0], crust: [30.0, 24.0, 22.0], cover: 0.95, ember: ([255.0, 92.0, 16.0], [255.0, 228.0, 140.0]), embers: 1.0, keep: 0.3 };
        match self {
            Cause::Plasma => Burn { scorch: [190.0, 80.0, 160.0], crust: [24.0, 16.0, 30.0], ember: ([170.0, 90.0, 255.0], [250.0, 236.0, 255.0]), keep: 0.5, ..fire },
            // a scald: blistered, not charred
            Cause::Steam => Burn { scorch: [206.0, 70.0, 62.0], crust: [228.0, 150.0, 140.0], cover: 0.55, embers: 0.0, ..fire },
            // eaten raw, its edge yellow-green
            Cause::Acid => Burn { scorch: [156.0, 164.0, 58.0], crust: [112.0, 38.0, 34.0], cover: 0.9, embers: 0.0, ..fire },
            _ => fire,
        }
    }

    fn spray(self) -> Spray {
        let blood = Spray { count: 14, speed: 5.0, size: 0.009, rgb: [150.0, 12.0, 14.0], soft: 1.0, glows: false, rises: None, scatter: 0.0 };
        match self {
            Cause::Slash => blood,
            Cause::Puncture => Spray { count: 10, speed: 6.5, ..blood },
            Cause::Vacuum => Spray { count: 20, speed: 7.0, ..blood },
            Cause::Crush | Cause::Impact => Spray { count: 5, speed: 3.0, ..blood },
            Cause::Kick => Spray { count: 4, speed: 3.0, ..blood },
            Cause::Punch => Spray { count: 3, speed: 3.0, ..blood },
            Cause::Jab => Spray { count: 2, speed: 3.0, ..blood },
            Cause::Grapple | Cause::Unbalance => Spray { count: 0, ..blood },
            Cause::Fire => Spray { count: 16, speed: 5.5, size: 0.008, rgb: [255.0, 150.0, 40.0], glows: true, ..blood },
            Cause::Plasma => Spray { count: 18, speed: 8.0, size: 0.007, rgb: [200.0, 150.0, 255.0], glows: true, ..blood },
            Cause::Acid => Spray { count: 12, speed: 4.0, rgb: [150.0, 180.0, 40.0], ..blood },
            Cause::Cold => Spray { count: 12, speed: 4.5, rgb: [214.0, 236.0, 255.0], ..blood },
            Cause::Lightning => Spray { count: 14, speed: 10.0, size: 0.006, rgb: [200.0, 222.0, 255.0], glows: true, ..blood },
            // puffs, motes: drifting up and fading
            Cause::Steam => Spray { count: 14, speed: 1.6, size: 0.03, rgb: [236.0, 238.0, 240.0], soft: 0.3, rises: Some(1.2), scatter: 0.7, ..blood },
            Cause::Disruption => Spray { count: 8, speed: 1.0, size: 0.008, rgb: [60.0, 30.0, 90.0], rises: Some(1.5), scatter: 0.6, ..blood },
            Cause::NonCorporeal => Spray { count: 10, speed: 0.8, size: 0.012, rgb: [214.0, 226.0, 240.0], soft: 0.5, glows: true, rises: Some(2.0), scatter: 0.8, ..blood },
            // the surface itself, coming away as dust
            Cause::Disintegrate => Spray { count: 60, speed: 1.2, size: 0.009, rgb: [96.0, 92.0, 100.0], rises: Some(1.8), scatter: 1.0, ..blood },
        }
    }
}

/// A part just hit, and how long ago (seconds): its wounds arrive.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fresh<'a> {
    pub part: &'a str,
    pub age: f32,
}

/// A cut throat's slash, by its seed (`Effects::cutthroat`), and how far
/// under the eyes a throat is cut, feet (the half-elves' faces, measured).
const THROAT_SEED: u32 = 0x7407_7407;
const THROAT_UNDER_EYES_FT: f32 = 0.48;

/// One injury placed on a segment's surface (the surface layout, feet): a
/// mark about `c` along `dir`, shaped by what made it.
#[derive(Debug, Clone, Copy)]
struct Gash {
    kind: Cause,
    c: [f32; 2],
    dir: [f32; 2],
    half_len: f32,
    half_w: f32,
    depth: f32,
    /// Deep enough to show bone at its bottom.
    bone: bool,
    phase: f32,
    seed: u32,
    /// The surface address it is centred on (station, ring angle).
    from: (f32, f32),
}

impl Gash {
    /// How far across the mark `at` is: 0 its middle, 1 its rim, more
    /// outside it.
    fn across(&self, at: [f32; 2]) -> f32 {
        let (px, py) = (at[0] - self.c[0], at[1] - self.c[1]);
        let along = px * self.dir[0] + py * self.dir[1];
        let side = -px * self.dir[1] + py * self.dir[0];
        match self.kind.base() {
            // a tapering, slightly wavering slit
            Cause::Slash => {
                let waver = 0.3 * self.half_w * (along / self.half_len * 4.0 + self.phase).sin();
                let across = (side - waver).abs();
                let past = along.abs() - self.half_len;
                if past >= 0.0 {
                    return 1.0 + across.hypot(past) / self.half_w;
                }
                let taper = (1.0 - (along / self.half_len).powi(2)).max(0.0).sqrt();
                (across / (self.half_w * taper).max(1e-4)).min(1.0 + across / self.half_w)
            }
            // a wandering line with branches off it, thinning to its ends
            Cause::Lightning => {
                let hl = self.half_len;
                let bend = |a: f32| 0.1 * hl * (a / hl * 5.0 + self.phase).sin();
                let mut d = (side - bend(along.clamp(-hl, hl))).hypot((along.abs() - hl).max(0.0));
                for b in 0..5 {
                    let h = |k: u32| wounds::hash(self.seed as i32 + b, k);
                    let t = (h(1) * 1.6 - 0.8) * hl;
                    let from = [t, bend(t)];
                    let lean = (0.5 + 0.7 * h(2)) * if h(3) > 0.5 { 1.0 } else { -1.0 };
                    let l = hl * (0.25 + 0.3 * h(4));
                    let v = [l * lean.cos() * if h(5) > 0.5 { 1.0 } else { -1.0 }, l * lean.sin()];
                    let u = (((along - from[0]) * v[0] + (side - from[1]) * v[1]) / (l * l).max(1e-9)).clamp(0.0, 1.0);
                    // a branch is thinner than the trunk
                    d = d.min((along - from[0] - v[0] * u).hypot(side - from[1] - v[1] * u) * 1.4);
                }
                d / (self.half_w * (1.0 - 0.6 * (along.abs() / hl).min(1.0)))
            }
            // a round mark with a ragged rim
            _ => {
                let a = side.atan2(along);
                let rim = 1.0 + self.kind.ragged() * (wounds::noise([a.cos() * 1.7 + self.phase, a.sin() * 1.7], self.seed) - 0.5);
                (along / self.half_len).hypot(side / self.half_w) / rim
            }
        }
    }

    /// How deep it is at `across`: a slash's pit has steep walls and a
    /// rounded floor, a puncture's is a shaft, a dent is smooth.
    fn depth_at(&self, across: f32) -> f32 {
        if across >= 1.0 {
            return 0.0;
        }
        let inside = 1.0 - across * across;
        self.depth
            * match self.kind.base() {
                Cause::Slash => inside.powf(0.6),
                Cause::Puncture => (1.0 - across.powi(4)).sqrt(),
                Cause::Crush => inside * inside,
                _ => inside,
            }
    }
}

/// A wound of `rank` by `kind`: its marks on segment `seg`, each centred on
/// one of `places` (where it may go), sized for a part `radius` thick, and
/// `open` of the way open (1: arrived). `crit` is the game's critical rank
/// (1..=9, 0 unknown): three to a wound rank, each a little worse. The
/// first mark is the worst of them; the rest are smaller, shallower and at
/// their own slants. `behind`: surface addresses the first marks come out
/// opposite (a wound that went through the body). Fixed by the segment, so
/// a wound does not move from frame to frame.
#[allow(clippy::too_many_arguments)]
fn gashes(seg: u32, rank: u8, kind: Cause, salt: u32, places: &[&Anchor], radius: f32, open: f32, crit: u8, behind: &[(f32, f32)]) -> Vec<Gash> {
    if rank == 0 || places.is_empty() {
        return Vec::new();
    }
    let (count, half_len, half_w, depth) = kind.shape(rank);
    let within = if crit == 0 { 0.5 } else { ((crit.clamp(1, 9) - 1) % 3) as f32 / 2.0 };
    let k = (radius / GASH_GIRTH_FT).clamp(GASH_SCALE.0, GASH_SCALE.1) * (0.8 + 0.4 * within);
    let depth = (depth * (0.85 + 0.3 * within)).min(radius * GASH_MAX_OF_RADIUS) * open;
    // a slit widens as it opens; a round mark spreads
    let (grow_len, grow_w) = match kind.base() {
        Cause::Slash | Cause::Lightning => (1.0, 0.2 + 0.8 * open),
        _ => (0.5 + 0.5 * open, 0.5 + 0.5 * open),
    };
    // Marks keep apart: two side by side leave a strip of whole skin
    // between them that reads as skin in the middle of one wound. One that
    // finds no room (in a few tries) is left out.
    let mut taken: Vec<([f32; 2], f32)> = Vec::new();
    (0..count)
        .filter_map(|j| {
            let id = (seg.wrapping_mul(31).wrapping_add(j as u32)) as i32;
            let lead = j == 0;
            let size = if lead { 1.05 + 0.25 * wounds::hash(id, salt ^ 0x53) } else { 0.5 + 0.45 * wounds::hash(id, salt ^ 0x53) };
            let reach = half_len.max(half_w) * k * size;
            let roomy = |p: &Anchor| p.feet.is_some_and(|f| taken.iter().all(|(c, r)| (f[0] - c[0]).hypot(f[1] - c[1]) > (r + reach) * 1.15));
            let place = match behind.get(j) {
                // opposite the mark on the other side: the same way along,
                // the ring angle mirrored front to back
                Some(&(station, theta)) => *places.iter().min_by(|a, b| {
                    let off = |p: &Anchor| {
                        let turn = ((p.theta + theta + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI).abs();
                        (p.station - station).abs() * crate::model::STATION_STEP_FT + turn * p.radius
                    };
                    off(a).total_cmp(&off(b))
                })?,
                None => (0..12u32).map(|t| places[((wounds::hash(id, salt ^ t.wrapping_mul(0x77)) * places.len() as f32) as usize).min(places.len() - 1)]).find(|p| roomy(p))?,
            };
            taken.push((place.feet?, reach));
            // any slant across the part
            let lean = (wounds::hash(id, salt ^ 0x51) * 2.0 - 1.0) * 1.35;
            Some(Gash {
                kind,
                c: place.feet?,
                dir: [lean.cos(), lean.sin()],
                half_len: half_len * k * size * grow_len,
                half_w: half_w * k * size.min(1.1) * grow_w,
                depth: depth * if lead { 1.0 } else { 0.7 },
                bone: rank >= 3 && lead,
                phase: wounds::hash(id, salt ^ 0x54) * std::f32::consts::TAU,
                seed: (id as u32) ^ salt.wrapping_mul(0x9E37),
                from: (place.station, place.theta),
            })
        })
        .collect()
}

/// What a hit throws off the body from mark `g` (at `at`, facing `nrm`),
/// as particles ([`Particle::at`] says where each is at an age): blood,
/// embers, chips of ice, sparks -- flying, then lying where they landed
/// (the ground is y = 0) until the hit is stale; or steam, dust, motes,
/// drifting up and fading.
fn loose(g: &Gash, at: V3, nrm: V3, alpha: f32) -> Vec<Particle> {
    let spray = g.kind.spray();
    // a scratch sheds little
    let count = if g.depth <= 0.0 && g.kind.opens() { spray.count / 4 } else { spray.count };
    // across the mark, for what starts all over it
    let side = norm_or(cross(nrm, [0.0, 1.0, 0.0]), [1.0, 0.0, 0.0]);
    let up = cross(side, nrm);
    let glow = match g.kind {
        Cause::Lightning => ParticleGlow::Spark,
        _ if spray.glows => ParticleGlow::Ember,
        _ => ParticleGlow::None,
    };
    (0..count)
        .map(|j| {
            let h = |k: u32| wounds::hash((g.seed as i32) ^ (j * 7919), k);
            let spread = [h(1) * 2.0 - 1.0, h(2) * 2.0 - 1.0, h(3) * 2.0 - 1.0];
            let r = spray.size * (0.6 + 0.8 * h(5));
            let from = add(at, add(scale(side, (h(6) * 2.0 - 1.0) * g.half_len * spray.scatter), scale(up, (h(7) * 2.0 - 1.0) * g.half_w * spray.scatter)));
            let alpha = alpha * spray.soft;
            if let Some(life) = spray.rises {
                // each leaves in its own moment, drifts out and up, and is gone
                let v = scale(norm_or(add(add(scale(nrm, 0.6), scale(spread, 0.4)), [0.0, 1.0, 0.0]), [0.0, 1.0, 0.0]), spray.speed * (0.5 + 0.5 * h(4)));
                return Particle { from, v, r, rises: Some(life), delay: h(8) * life * 0.5, land: 0.0, ground: 0.0, glow, rgb: spray.rgb, alpha, loops: false };
            }
            let dir = norm_or(add(add(nrm, scale(spread, 0.8)), [0.0, 0.35, 0.0]), nrm);
            let v = scale(dir, spray.speed * (0.3 + 0.7 * h(4)));
            let ground = r * 0.5;
            let fall = (from[1] - ground).max(0.0);
            let land = (v[1] + (v[1] * v[1] + 2.0 * FALL_FT_S2 * fall).sqrt()) / FALL_FT_S2;
            Particle { from, v, r, rises: None, delay: 0.0, land, ground, glow, rgb: spray.rgb, alpha, loops: false }
        })
        .collect()
}

/// How a thrown dot gives off light.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParticleGlow {
    None,
    /// An ember: glowing as it leaves, cooling as it flies.
    Ember,
    /// A spark: bright, gone in a moment, never lands.
    Spark,
    /// A wisp of shadow (hidden, `shade`): dark, see-through (a film, never
    /// the surface), unlit, swaying wide as it rises and spreads. (Steam
    /// off a frenzied figure is one too, pale: `heat`.)
    Shadow,
    /// A mote circling where it leaves from, aglow and twinkling, on a loop
    /// of its own (stunned stars, `daze`; calm motes, `calm`; the light
    /// round a vulnerable figure's feet, `exposed`): `v` is [the circle's
    /// radius, how high it bobs, 0], and its loop is one turn round.
    Orbit,
}

/// A dot a hit throws off ([`Particle::at`]): where it leaves from and how
/// fast, how big it is, and how it moves -- thrown, falling and lying where
/// it lands (`rises` None), or drifting up and fading (steam, dust, motes).
/// The same motion the GPU gives it (`cloud_gpu.wgsl`, `thrown`), so a
/// hit's arrival is built once and moved there frame by frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Particle {
    pub from: V3,
    pub v: V3,
    pub r: f32,
    /// Drifting up, gone after this many seconds of its own (it leaves
    /// `delay` seconds after the hit); None: thrown.
    pub rises: Option<f32>,
    pub delay: f32,
    /// When a thrown one lands (seconds after the hit), and the height of
    /// the ground it lies on.
    pub land: f32,
    pub ground: f32,
    pub glow: ParticleGlow,
    /// Its colour (0..255) and opacity before it fades.
    pub rgb: [f32; 3],
    pub alpha: f32,
    /// Rising on a loop of its own for as long as it is built, not once
    /// after a hit (an invisible figure's glints, `ghost`): `delay` is its
    /// place in the loop, the age it is drawn at a running clock, and `v`
    /// how fast it rises in the world, whatever way the part it leaves is
    /// turned (only its height is used).
    pub loops: bool,
}

impl Particle {
    /// It `age` seconds after the hit (one that [`Particle::loops`]: `age`
    /// on a running clock); None when it is not there (not yet left,
    /// faded, or a spark past its moment).
    pub fn at(&self, age: f32) -> Option<Gaussian> {
        let glows = self.glow != ParticleGlow::None;
        let r = self.r;
        if let (Some(life), true) = (self.rises, self.loops) {
            // a glint: in and out on its loop, twinkling, swaying as it
            // rises; a wisp of shadow: no twinkle, spreading, swaying wider
            let shadow = self.glow == ParticleGlow::Shadow;
            let life = life.max(0.1);
            let t = (age + self.delay).rem_euclid(life);
            // circling: once round a loop, bobbing twice, always there
            if self.glow == ParticleGlow::Orbit {
                let a = std::f32::consts::TAU * t / life;
                let twinkle = 0.7 + 0.3 * (age * 7.0 + self.delay * 37.0).sin();
                let c = add(self.from, [self.v[0] * a.cos(), self.v[1] * (2.0 * a).sin(), self.v[0] * a.sin()]);
                return Some(Gaussian { c, axes: [[r, 0.0, 0.0], [0.0, r, 0.0], [0.0, 0.0, r]], n: [0.0; 3], rgb: self.rgb, alpha: self.alpha * twinkle, glow: 1.0, wet: 0.0, film: false, rim: 0.0 });
            }
            let fade = smoothstep(0.0, 0.15 * life, t) * (1.0 - smoothstep(0.45 * life, life, t));
            let twinkle = if shadow { 1.0 } else { 0.55 + 0.45 * (age * 9.0 + self.delay * 37.0).sin() };
            if fade * twinkle <= 0.02 {
                return None;
            }
            let r = r * (1.0 + if shadow { 1.2 } else { 0.6 } * t / life);
            let sway = if shadow { 0.07 } else { 0.02 } * (t * 2.7 + self.delay * 11.0).sin();
            let c = add(self.from, [sway, self.v[1] * t, 0.6 * sway]);
            return Some(Gaussian { c, axes: [[r, 0.0, 0.0], [0.0, r, 0.0], [0.0, 0.0, r]], n: [0.0; 3], rgb: self.rgb, alpha: self.alpha * fade * twinkle, glow: if shadow { 0.0 } else { fade }, wet: 0.0, film: shadow, rim: 0.0 });
        }
        if let Some(life) = self.rises {
            let t = age - self.delay;
            let fade = smoothstep(0.0, 0.1, t) * (1.0 - smoothstep(0.3 * life, life, t));
            if fade <= 0.02 {
                return None;
            }
            let grown = r * (1.0 + t / life);
            let axes = [[grown, 0.0, 0.0], [0.0, grown, 0.0], [0.0, 0.0, grown]];
            return Some(Gaussian { c: add(self.from, scale(self.v, t)), axes, n: [0.0; 3], rgb: self.rgb, alpha: self.alpha * fade, glow: if glows { 0.5 * fade } else { 0.0 }, wet: 0.0, film: false, rim: 0.0 });
        }
        let stale = 1.0 - smoothstep(FRESH_SECS - 1.2, FRESH_SECS, age);
        // an ember cools as it flies, a spark is gone in a moment
        let (glow, fade) = match self.glow {
            ParticleGlow::Spark => (1.0, 1.0 - smoothstep(0.1, 0.45, age)),
            ParticleGlow::Ember => ((-age).exp(), stale),
            ParticleGlow::None | ParticleGlow::Shadow | ParticleGlow::Orbit => (0.0, stale),
        };
        if fade <= 0.02 {
            return None;
        }
        let rgb = if glows { wounds::mix([70.0, 34.0, 24.0], self.rgb, glow) } else { self.rgb };
        let (from, v) = (self.from, self.v);
        if age < self.land {
            let c = [from[0] + v[0] * age, from[1] + v[1] * age - 0.5 * FALL_FT_S2 * age * age, from[2] + v[2] * age];
            let vel = [v[0], v[1] - FALL_FT_S2 * age, v[2]];
            // drawn out along its flight
            let along = norm_or(vel, [0.0, -1.0, 0.0]);
            let across = norm_or(cross(along, [0.0, 1.0, 0.0]), [1.0, 0.0, 0.0]);
            let axes = [scale(along, r + len(vel) * 0.002), scale(across, r), scale(cross(along, across), r)];
            Some(Gaussian { c, axes, n: [0.0; 3], rgb, alpha: self.alpha * fade, glow, wet: 0.0, film: false, rim: 0.0 })
        } else if self.glow != ParticleGlow::Spark {
            let c = [from[0] + v[0] * self.land, self.ground, from[2] + v[2] * self.land];
            let axes = [[r * 1.7, 0.0, 0.0], [0.0, 0.0, r * 1.7], [0.0, r * 0.25, 0.0]];
            Some(Gaussian { c, axes, n: [0.0, 1.0, 0.0], rgb, alpha: self.alpha * fade, glow: glow * 0.8, wet: 0.0, film: false, rim: 0.0 })
        } else {
            None
        }
    }
}

/// A hit's arrival as built for a GPU to play ([`LayerCloud::arrival`]):
/// the hit part at each of [`ARRIVAL_AGES`] (no thrown dots), and what it
/// throws off, as particles.
pub struct Arrival {
    pub frames: Vec<Vec<Dot>>,
    pub particles: Vec<Particle>,
}

/// The ages (seconds after a hit) a hit part's arrival is built at, for a
/// GPU to blend between: close together while the wound opens and flashes
/// (the first [`OPEN_SECS`]), wider as a burn cools, the last when it is
/// no longer fresh.
pub const ARRIVAL_AGES: [f32; 12] = [0.0, 0.04, 0.09, 0.15, 0.22, 0.3, 0.4, 0.6, 0.9, 1.4, 2.2, FRESH_SECS];

/// What [`LayerCloud::dots_of`] does with what a hit throws off.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Thrown {
    /// Dots, where they are at the hit's age.
    Dots,
    /// Nothing.
    Skip,
    /// Particles, to be moved by whoever draws them.
    Particles,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Layer {
    Bone,
    Flesh,
    Fat,
    Skin,
    Coat,
    /// A web over the body (webbed): strands along the skin, and threads
    /// bridging limb to body. Films (`Gaussian::film`).
    Web,
    /// What another status builds on the body (`status_layers`): vines and
    /// roots, ropes of light, running blood, boils, blisters, a veil of
    /// shadow, a possessed figure's wisps.
    Status,
    /// What a status builds round the whole figure, across its parts:
    /// webbed's webs and bound's coils of rope round the held figure
    /// (`wraps`), frozen's icicles, frost needles and the ice at its feet
    /// (`frost`). Built once with the body, not with a hit's frames.
    Wrap,
    /// Blood a bleeding figure's open wound pours out (`blood`): its pool
    /// in the wound, its runs down the figure, the soak round it, its
    /// drips and the pool on the ground. `from`: the segment of the wound
    /// it comes from (it lies on whatever part it ran onto), which a
    /// host rebuilding that segment as a hit arrives builds it with.
    Blood { from: u32 },
    /// Thrown off the body by a hit: no longer on an anchor.
    Loose,
}

/// What a coating is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Material {
    Ice,
    Stone,
}

impl Material {
    pub const ALL: [Material; 2] = [Material::Ice, Material::Stone];

    pub fn name(self) -> &'static str {
        match self {
            Material::Ice => "ice",
            Material::Stone => "stone",
        }
    }

    pub fn from_name(name: &str) -> Option<Material> {
        Material::ALL.into_iter().find(|m| m.name().eq_ignore_ascii_case(name.trim()))
    }

    /// Its colour at surface point `at` (feet) and its opacity.
    fn look(self, at: [f32; 2], seg: u32) -> ([f32; 3], f32) {
        match self {
            Material::Ice => {
                let glint = wounds::noise([at[0] / 0.05, at[1] / 0.05], seg ^ 0x1CE) > 0.86;
                let c = wounds::mix([132.0, 182.0, 236.0], [176.0, 214.0, 248.0], wounds::fbm([at[0] / 0.2, at[1] / 0.2], seg ^ 0x1CE5));
                (if glint { [244.0, 250.0, 255.0] } else { c }, 0.3)
            }
            Material::Stone => {
                let grain = wounds::fbm([at[0] / 0.09, at[1] / 0.09], seg ^ 0x570E);
                (wounds::mix([92.0, 90.0, 86.0], [150.0, 146.0, 138.0], grain), 0.95)
            }
        }
    }
}

/// A coating over the body: `progress` 0 (none) to 1 (head to foot),
/// `thickness_ft` thick once grown.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coating {
    pub material: Material,
    pub progress: f32,
    pub thickness_ft: f32,
    /// It came AFTER the wounds: it lies over them, whole (frozen once
    /// already hurt). Else the wounds came after it and cut through it.
    pub over_wounds: bool,
}

impl Coating {
    pub fn new(material: Material, progress: f32) -> Coating {
        Coating { material, progress, thickness_ft: COAT_FT, over_wounds: false }
    }

    /// The same coating, laid over the wounds (or cut by them).
    pub fn over_wounds(self, over: bool) -> Coating {
        Coating { over_wounds: over, ..self }
    }

    /// `ice:0.6`, `stone` (all of it), `ice:0.5:0.12` (that thick, feet);
    /// a last `:over` lays it over the wounds.
    pub fn parse(text: &str) -> Option<Coating> {
        let (text, over_wounds) = match text.trim().strip_suffix(":over") {
            Some(rest) => (rest, true),
            None => (text, false),
        };
        let mut it = text.split(':');
        let material = Material::from_name(it.next()?)?;
        let progress = it.next().map_or(Some(1.0), |v| v.trim().parse().ok())?;
        let thickness_ft = it.next().map_or(Some(COAT_FT), |v| v.trim().parse().ok())?;
        Some(Coating { material, progress, thickness_ft, over_wounds })
    }

    /// How far off the surface the coating stands at an anchor (0: not
    /// there yet). Its front climbs from the ground to past the crown with a
    /// ragged edge, and it thickens behind the front.
    fn height(&self, a: &Anchor) -> f32 {
        let at = a.at();
        let seg = a.seg as u32;
        let ragged = wounds::fbm([at[0] / 0.35, at[1] / 0.35], seg.wrapping_mul(131) ^ 0xC0A7) - 0.5;
        let front = self.progress.clamp(0.0, 1.0) * 1.2 - 0.1 + ragged * 0.12;
        let behind = front - a.up;
        if behind <= 0.0 {
            return 0.0;
        }
        let grown = smoothstep(0.0, 0.3, behind);
        let lumps = wounds::fbm([at[0] / 0.12, at[1] / 0.12], seg ^ 0x7A1C);
        self.thickness_ft.max(0.0) * (0.15 + 0.85 * grown) * (0.5 + 0.5 * lumps)
    }
}

/// One place on the skin, in the puppet's rest space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anchor {
    pub seg: usize,
    /// As `UvLayout::locate` takes them.
    pub station: f32,
    pub theta: f32,
    /// On the 1 px-per-foot surface layout (feet), where it has a place.
    pub feet: Option<[f32; 2]>,
    /// Facing at rest: -1 straight back .. 1 straight forward.
    pub front: f32,
    /// How far up the standing figure, 0 the ground .. 1 its top.
    pub up: f32,
    /// The body's radius here, feet.
    pub radius: f32,
    /// The step to the next anchor round the ring and along the bone, feet.
    step: [f32; 2],
    /// How far the ring angle turns per foot round the ring here (an
    /// ellipse's angle does not run evenly round it).
    turn: f32,
    /// How many stations a foot along the bone is here (0 on a last ring).
    rate: f32,
    /// The other body segment it lies inside at rest (the body's segments
    /// overlap: a chest's rings run on inside the belly's). What stands ON
    /// the skin is left off it while it is still inside that hull as posed
    /// -- so a coating is one layer thick, and comes back where a bend
    /// brings the skin out.
    inside: Option<u32>,
}

impl Anchor {
    /// One of the `n` x `n` smaller anchors this one splits into (`part`
    /// counts them), each with its own place on `surface`.
    fn split(&self, part: usize, n: usize, surface: &UvLayout) -> Anchor {
        let off = |k: usize| (k as f32 + 0.5) / n as f32 - 0.5;
        let theta = self.theta + off(part % n) * self.step[0] * self.turn;
        let station = (self.station + off(part / n) * self.step[1] * self.rate).max(0.0);
        Anchor { station, theta, feet: surface.locate(self.seg, station, theta).or(self.feet), step: [self.step[0] / n as f32, self.step[1] / n as f32], ..*self }
    }

    /// A surface point for the procedural looks: the layout's, else its own
    /// address.
    fn at(&self) -> [f32; 2] {
        self.feet.unwrap_or([self.theta * self.radius, self.station * crate::model::STATION_STEP_FT])
    }
}

/// A column of a pit or dent, in the world: where its surface was, which
/// way that faced, how deep it goes and how wide the column is. Another
/// segment's dots inside it are in the wound's way (`LayerCloud::dots`).
#[derive(Debug, Clone)]
struct Hollow {
    seg: usize,
    p: V3,
    n: V3,
    deep: f32,
    r: f32,
}

/// The pits and dents a cloud's wounds made, by segment
/// ([`LayerCloud::dots_and_hollows`]): what clears another segment's dots
/// out of them. A host that builds a hit part alone hands the rest of the
/// body's to it ([`LayerCloud::arrival`]), so it is cleared as a whole
/// build would clear it.
#[derive(Debug, Clone, Default)]
pub struct Hollows(Vec<Hollow>);

/// How far proud of a hollow's surface another segment's dot may sit and
/// still be cleared away with what is in the hollow, feet.
const HOLLOW_OVER_FT: f32 = 0.03;

/// A point is this far inside a hull (1: on its surface) before it counts
/// as buried in it: what is near the surface stays, so two hulls crossing
/// do not both lose their dots along the crossing.
const BURIED: f32 = 0.85;

/// Ice is two films deep: under its face (`rgb`, `alpha`), at `c`, a
/// deeper, bluer one, so the body shows through ice with depth to it.
fn ice_under(material: Material, c: V3, axes: [V3; 3], n: V3, rgb: [f32; 3], alpha: f32) -> Option<Gaussian> {
    (material == Material::Ice).then(|| Gaussian { c, axes, n, rgb: wounds::mix(rgb, [92.0, 146.0, 214.0], 0.55), alpha: alpha * 0.4, glow: 0.0, wet: 0.0, film: true, rim: 0.0 })
}

/// Whether `p` (of anchor `a`) lies inside the part `a` may be buried in.
fn inside_other(surfs: &[Surface], a: &Anchor, p: V3) -> bool {
    a.inside.and_then(|j| surfs.iter().find(|s| s.index == j as usize)).is_some_and(|o| {
        let n = o.rings.len();
        let axis = norm_or(sub(o.rings[n - 1].c, o.rings[0].c), [0.0, 1.0, 0.0]);
        in_hull(o.rings.iter().map(|r| (r.c, r.a, r.b)), axis, crate::model::STATION_STEP_FT * 0.6, p)
    })
}

/// A ring's reach at angle (sin, cos) round it.
fn radial_of(r: &crate::draw::SurfRing, sin: f32, cos: f32) -> V3 {
    add(scale(r.a, cos), scale(r.b, sin))
}

/// Whether `p` is inside the hull of these rings (centre and semi-axes),
/// which run along `axis`, `slab` apart.
fn in_hull(rings: impl Iterator<Item = (V3, V3, V3)>, axis: V3, slab: f32, p: V3) -> bool {
    for (c, a, b) in rings {
        let e = sub(p, c);
        if dot(e, axis).abs() > slab {
            continue;
        }
        let (u, v) = (dot(e, a) / dot(a, a).max(1e-6), dot(e, b) / dot(b, b).max(1e-6));
        if u * u + v * v < BURIED * BURIED {
            return true;
        }
    }
    false
}

/// One Gaussian of the layered cloud: its layer and the anchor it stands
/// on (None: a stump's cross-section, which has none).
#[derive(Debug, Clone)]
pub struct Dot {
    pub layer: Layer,
    pub anchor: Option<u32>,
    /// The fitted cloud's Gaussian it wears (its index there: a
    /// [`FittedSkin`] is bound from that cloud in order), when it is one.
    /// A host that bound that cloud to a mesh binds this dot as it did.
    pub source: Option<u32>,
    pub g: Gaussian,
}

/// How the figure looks: its wounds and scars (`look`; the yaw is ignored
/// -- the renderer turns the cloud), the statuses on its skin, its painted
/// sheet and its coating; what made each part's wound (`causes`: a slash
/// where it does not say) and the part just hit (`fresh`).
#[derive(Clone, Copy, Default)]
pub struct LayerLook<'a> {
    pub look: Look<'a>,
    pub effects: Effects,
    pub texture: Option<&'a Texture>,
    pub coat: Option<Coating>,
    pub causes: &'a [(&'a str, Cause)],
    /// The game's critical rank of each part's wound (1..=9; a middling
    /// one where it does not say): how bad it is within its wound rank.
    pub crits: &'a [(&'a str, u8)],
    pub fresh: Option<Fresh<'a>>,
    /// A fitted cloud worn as the skin ([`LayerCloud::fit`], of the same
    /// cloud and puppet): its Gaussians in place of the made skin dots.
    pub fitted: Option<&'a FittedSkin>,
    /// The pose a status holds the figure in (webbed, bound: arms pinned,
    /// legs together), where it is wrapped whole (`wraps`); None: each part
    /// is wrapped on its own.
    pub held: Option<&'a Posed<'a>>,
    /// The pose the figure is SHOWN in, when the cloud is built in another
    /// (a host that poses the cloud by its parts builds it as drawn): which
    /// way is down for what runs on it (`blood`). None: as built (or
    /// held).
    pub stands: Option<&'a Posed<'a>>,
}

/// A fitted cloud bound to a puppet's anchors: each Gaussian moves with
/// the body (`splat::BoundCloud`) and has a surface address of its own.
#[derive(Debug, Clone)]
pub struct FittedSkin {
    bound: BoundCloud,
    /// Per Gaussian: its own address, as an anchor.
    own: Vec<Anchor>,
    /// Per anchor of the cloud it was fitted to: the Gaussians nearest it.
    by_anchor: Vec<Vec<u32>>,
    /// Per anchor: how far off the puppet's surface the cloud's is there
    /// (feet, outward), and the cloud's colour there -- its own Gaussians',
    /// else its segment's.
    offset: Vec<f32>,
    tone: Vec<[f32; 3]>,
}

/// How far over a fitted cloud's surface a wound's marks are laid, feet.
const OVERLAY_FT: f32 = 0.004;
/// Round an open wound in a fitted cloud, this far out from its rim (feet),
/// the skin is made dots in the cloud's colour there: the cloud's own
/// Gaussians that near are taken out if they are big enough to reach the
/// wound (a Gaussian centred beside it would lie across it).
const PATCH_FT: f32 = 0.08;

impl FittedSkin {
    pub fn len(&self) -> usize {
        self.own.len()
    }

    pub fn is_empty(&self) -> bool {
        self.own.is_empty()
    }
}

/// A puppet's anchors, built once; [`LayerCloud::dots`] poses them.
#[derive(Debug, Clone)]
pub struct LayerCloud {
    anchors: Vec<Anchor>,
    /// Per segment, its anchors' range.
    spans: Vec<(usize, usize)>,
    /// The 1 px-per-foot surface layout the marks are baked on.
    surface: UvLayout,
    params: SplatParams,
    /// Each anchor's place at rest.
    rest: Vec<V3>,
}

impl LayerCloud {
    /// Bind a fitted cloud (in the puppet's rest space, standing its
    /// height: `splat::fit_to_height`) to these anchors: each Gaussian to
    /// the anchor it lies nearest, with its own address worked out from
    /// where it is beside that anchor.
    pub fn fit(&self, puppet: &Puppet, cloud: Vec<Gaussian>) -> FittedSkin {
        self.fit_bound(puppet, BoundCloud::bind(puppet, cloud))
    }

    /// [`LayerCloud::fit`] of a cloud already bound to the puppet's
    /// segments (a modelled body's, which moves as its mesh does:
    /// `mesh::SplatBody::bound_cloud`).
    pub fn fit_bound(&self, puppet: &Puppet, bound: BoundCloud) -> FittedSkin {
        let cloud = &bound.rest;
        const CELL: f32 = 0.08;
        let cell = |p: V3| [(p[0] / CELL).floor() as i32, (p[1] / CELL).floor() as i32, (p[2] / CELL).floor() as i32];
        let mut grid: std::collections::HashMap<[i32; 3], Vec<u32>> = std::collections::HashMap::new();
        for (k, p) in self.rest.iter().enumerate() {
            grid.entry(cell(*p)).or_default().push(k as u32);
        }
        let nearest = |q: V3| -> Option<u32> {
            let c = cell(q);
            // wider and wider, until what was found is nearer than the
            // cells not yet looked in
            for reach in 1..=8i32 {
                let mut best: Option<(f32, u32)> = None;
                for dx in -reach..=reach {
                    for dy in -reach..=reach {
                        for dz in -reach..=reach {
                            for &k in grid.get(&[c[0] + dx, c[1] + dy, c[2] + dz]).into_iter().flatten() {
                                let d = len(sub(self.rest[k as usize], q));
                                if best.is_none_or(|(b, _)| d < b) {
                                    best = Some((d, k));
                                }
                            }
                        }
                    }
                }
                if let Some((d, k)) = best.filter(|(d, _)| *d <= reach as f32 * CELL) {
                    let _ = d;
                    return Some(k);
                }
            }
            (0..self.rest.len()).min_by(|a, b| len(sub(self.rest[*a], q)).total_cmp(&len(sub(self.rest[*b], q)))).map(|k| k as u32)
        };
        let mut own = Vec::with_capacity(cloud.len());
        let mut by_anchor = vec![Vec::new(); self.anchors.len()];
        // per anchor: its Gaussians' offsets and colours, summed
        let mut sums = vec![(0.0f32, [0.0f32; 3], 0u32); self.anchors.len()];
        for (f, g) in cloud.iter().enumerate() {
            let Some(k) = nearest(g.c) else {
                own.push(Anchor { seg: 0, station: 0.0, theta: 0.0, feet: None, front: 0.0, up: 0.0, radius: 0.0, step: [0.0; 2], turn: 0.0, rate: 0.0, inside: None });
                continue;
            };
            let a = self.anchors[k as usize];
            let s = &puppet.segments[a.seg];
            // round the ring and along the bone from the anchor
            let n = s.stations.len().max(1);
            let k0 = (a.station.floor() as usize).min(n - 1);
            let k1 = (k0 + 1).min(n - 1);
            let t = a.station - k0 as f32;
            let (hw, hd) = (s.stations[k0].w + (s.stations[k1].w - s.stations[k0].w) * t, s.stations[k0].d + (s.stations[k1].d - s.stations[k0].d) * t);
            let (sin, cos) = a.theta.sin_cos();
            let tangent = norm_or(add(scale(s.axis_w, -hw * sin), scale(s.axis_d, hd * cos)), [1.0, 0.0, 0.0]);
            let along = norm_or(sub(s.tail, s.head), [0.0, 1.0, 0.0]);
            let e = sub(g.c, self.rest[k as usize]);
            let theta = a.theta + dot(e, tangent) * a.turn;
            let station = (a.station + dot(e, along) * a.rate).max(0.0);
            own.push(Anchor { station, theta, feet: self.surface.locate(a.seg, station, theta).or(a.feet), ..a });
            by_anchor[k as usize].push(f as u32);
            let outward = norm_or(add(scale(s.axis_w, hw * cos), scale(s.axis_d, hd * sin)), [0.0, 0.0, 1.0]);
            let sum = &mut sums[k as usize];
            *sum = (sum.0 + dot(e, outward), add(sum.1, g.rgb), sum.2 + 1);
        }
        // an anchor with none of its own takes its segment's
        let mut of_seg = vec![(0.0f32, [0.0f32; 3], 0u32); puppet.segments.len()];
        for (a, sum) in self.anchors.iter().zip(&sums) {
            let seg = &mut of_seg[a.seg];
            *seg = (seg.0 + sum.0, add(seg.1, sum.1), seg.2 + sum.2);
        }
        let mean = |sum: &(f32, [f32; 3], u32)| (sum.2 > 0).then(|| (sum.0 / sum.2 as f32, scale(sum.1, 1.0 / sum.2 as f32)));
        let (offset, tone) = self.anchors.iter().zip(&sums).map(|(a, sum)| mean(sum).or_else(|| mean(&of_seg[a.seg])).unwrap_or((0.0, [128.0; 3]))).unzip();
        FittedSkin { bound, own, by_anchor, offset, tone }
    }

    pub fn build(puppet: &Puppet, params: &SplatParams) -> LayerCloud {
        let spacing = params.spacing.max(0.005);
        let surface = UvLayout::new(puppet, 1.0);
        let mut anchors = Vec::new();
        // each anchor's place at rest, for finding the buried ones
        let mut rest: Vec<V3> = Vec::new();
        let mut spans = Vec::with_capacity(puppet.segments.len());
        for (i, s) in puppet.segments.iter().enumerate() {
            let start = anchors.len();
            // as `splat::cloud`: garments and props too, not what they hide
            for k in 0..if s.hidden { 0 } else { s.stations.len() } {
                let st = &s.stations[k];
                let next = s.stations.get(k + 1);
                let gap = next.map_or(0.0, |nx| len(sub(nx.c, st.c)));
                let subs = if next.is_some() { ((gap / spacing).round() as usize).max(1) } else { 1 };
                for sub_k in 0..subs {
                    let t = sub_k as f32 / subs as f32;
                    let (c, hw, hd) = match next {
                        Some(nx) if t > 0.0 => (lerp3(st.c, nx.c, t), st.w + (nx.w - st.w) * t, st.d + (nx.d - st.d) * t),
                        _ => (st.c, st.w, st.d),
                    };
                    // Ramanujan's ellipse perimeter.
                    let circ = std::f32::consts::PI * (3.0 * (hw + hd) - ((3.0 * hw + hd) * (hw + 3.0 * hd)).sqrt());
                    let m = ((circ / spacing).ceil() as usize).clamp(6, 400);
                    let along = if gap > 1e-4 { gap / subs as f32 } else { spacing };
                    for j in 0..m {
                        let theta = j as f32 / m as f32 * std::f32::consts::TAU;
                        let (sin, cos) = theta.sin_cos();
                        let radial = add(scale(s.axis_w, cos * hw), scale(s.axis_d, sin * hd));
                        let radius = len(radial);
                        anchors.push(Anchor {
                            seg: i,
                            station: k as f32 + t,
                            theta,
                            feet: surface.locate(i, k as f32 + t, theta),
                            front: if radius > 1e-5 { radial[2] / radius } else { 0.0 },
                            up: ((c[1] + radial[1]) / puppet.height.max(1e-3)).clamp(0.0, 1.0),
                            radius,
                            step: [circ / m as f32, along],
                            turn: 1.0 / (hw * sin).hypot(hd * cos).max(0.01),
                            rate: if gap > 1e-4 { 1.0 / gap } else { 0.0 },
                            inside: None,
                        });
                        rest.push(add(c, radial));
                    }
                }
            }
            spans.push((start, anchors.len()));
        }
        let slab = crate::model::STATION_STEP_FT * 0.6;
        for (a, p) in anchors.iter_mut().zip(&rest) {
            a.inside = puppet.segments.iter().enumerate().position(|(j, o)| {
                j != a.seg && !o.hidden && o.kind.is_body() && {
                    let axis = norm_or(sub(o.tail, o.head), [0.0, 1.0, 0.0]);
                    in_hull(o.stations.iter().map(|st| (st.c, scale(o.axis_w, st.w), scale(o.axis_d, st.d))), axis, slab, *p)
                }
            })
            .map(|j| j as u32);
        }
        LayerCloud { anchors, spans, surface, params: *params, rest }
    }

    pub fn anchors(&self) -> &[Anchor] {
        &self.anchors
    }

    pub fn len(&self) -> usize {
        self.anchors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.anchors.is_empty()
    }

    /// The cloud in `posed`'s pose (the puppet this was built for), as
    /// Gaussians for `splat::render` / `splat::prepare`.
    pub fn posed(&self, posed: &Posed<'_>, look: &LayerLook<'_>) -> Vec<Gaussian> {
        self.dots(posed, look).into_iter().map(|d| d.g).collect()
    }

    /// [`LayerCloud::posed`], each Gaussian with its layer and anchor.
    pub fn dots(&self, posed: &Posed<'_>, look: &LayerLook<'_>) -> Vec<Dot> {
        self.dots_of(posed, look, None, Thrown::Dots, &[]).0
    }

    /// [`LayerCloud::dots`], and the pits and dents its wounds made.
    pub fn dots_and_hollows(&self, posed: &Posed<'_>, look: &LayerLook<'_>) -> (Vec<Dot>, Hollows) {
        let (dots, _, hollows) = self.dots_of(posed, look, None, Thrown::Dots, &[]);
        (dots, Hollows(hollows))
    }

    /// [`LayerCloud::dots`] of the segments `only` marks (by segment index)
    /// alone: what a host rebuilds while a hit arrives, keeping the rest
    /// ([`LayerCloud::fresh_segments`]). No web threads (they span
    /// segments: the rest has them), and only these segments' wounds clear
    /// what lies in them.
    pub fn dots_in(&self, posed: &Posed<'_>, look: &LayerLook<'_>, only: &[bool]) -> Vec<Dot> {
        self.dots_of(posed, look, Some(only), Thrown::Dots, &[]).0
    }

    /// The hit part's arrival (`look.fresh` names the part; its age is
    /// not used), built for a GPU to play: the segments `only` marks at
    /// each of [`ARRIVAL_AGES`], with nothing thrown off, and what the hit
    /// throws off as [`Particle`]s (from the wound fully open). `around`:
    /// the rest of the body's pits ([`LayerCloud::dots_and_hollows`] of
    /// the whole), which clear the hit part's dots out of another part's
    /// open wound as a whole build would. Built once a hit, the frames side
    /// by side on every core.
    pub fn arrival(&self, posed: &Posed<'_>, look: &LayerLook<'_>, only: &[bool], around: &Hollows) -> Arrival {
        let Some(fresh) = look.fresh else { return Arrival { frames: Vec::new(), particles: Vec::new() } };
        let at = |age: f32| LayerLook { fresh: Some(Fresh { age, ..fresh }), ..*look };
        let frames = std::thread::scope(|scope| {
            let running: Vec<_> = ARRIVAL_AGES.iter().map(|age| { let look = at(*age); scope.spawn(move || self.dots_of(posed, &look, Some(only), Thrown::Skip, &around.0).0) }).collect();
            running.into_iter().map(|r| r.join().expect("an arrival frame")).collect()
        });
        let particles = self.dots_of(posed, &at(OPEN_SECS * 2.0), Some(only), Thrown::Particles, &[]).1;
        Arrival { frames, particles }
    }

    /// What drifts off the figure as `look` shows it, each on a loop of its
    /// own ([`Particle::loops`]), for a host with a clock to play: an
    /// invisible figure's glints (`ghost`), a hidden one's wisps of shadow
    /// (`shade`), a possessed one's smoke and sparks (`possess`), a blinded
    /// one's mist curling off its eyes (`blind`). Placed in `posed`'s pose,
    /// as [`LayerCloud::dots`].
    pub fn motes(&self, posed: &Posed<'_>, look: &LayerLook<'_>) -> Vec<Particle> {
        let fx = look.effects;
        if !fx.moving() {
            return Vec::new();
        }
        let puppet = posed.puppet;
        let shown = look.stands.or(look.held).unwrap_or(posed);
        let skin = self.skin_points(shown, &look.look, look.fitted);
        let rough: Vec<bool> = puppet.segments.iter().map(|s| s.part.as_deref().is_some_and(|q| q.ends_with("Hand"))).collect();
        let parent: Vec<Option<usize>> = puppet.segments.iter().map(|s| s.parent).collect();
        let trunk: Vec<bool> = puppet.segments.iter().map(|s| s.kind == crate::Kind::Trunk).collect();
        let parts = crate::skin_field::Parts { rough: &rough, parent: &parent, trunk: &trunk };
        let seed = |salt: u32| puppet.family.bytes().fold(salt, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
        let mut out = Vec::new();
        if look.effects.invisible {
            out.extend(crate::ghost::glints(&skin, &posed.xf, &shown.xf, &parts, seed(0x6057)));
        }
        if look.effects.hidden {
            out.extend(crate::shade::wisps(&skin, &posed.xf, &shown.xf, &parts, seed(0x5A0E)));
        }
        if look.effects.possessed {
            out.extend(crate::possess::wisps(&skin, &posed.xf, &shown.xf, &parts, seed(0x9055)));
        }
        if look.effects.blinded {
            out.extend(crate::blind::wisps(&skin, &posed.xf, &shown.xf, posed, &parts, seed(0xB11D)));
        }
        if fx.stunned || fx.dazed {
            out.extend(crate::daze::stars(&skin, &posed.xf, &shown.xf, &parts, fx.stunned, seed(0x57A2)));
        }
        if fx.frenzied || fx.overexerted {
            out.extend(crate::heat::steam(&skin, &posed.xf, &shown.xf, &parts, fx.frenzied, seed(0x57EA)));
        }
        if fx.calmed {
            out.extend(crate::calm::motes(&skin, &posed.xf, &shown.xf, &parts, seed(0xCA1A)));
        }
        if fx.vulnerable {
            out.extend(crate::exposed::motes(&skin, &posed.xf, &shown.xf, &parts, seed(0xE705)));
        }
        if fx.silenced {
            out.extend(crate::hush::glints(&skin, &posed.xf, &shown.xf, posed, seed(0x4054)));
        }
        if fx.immobile {
            out.extend(crate::fetter::glints(&skin, &posed.xf, &shown.xf, &parts, seed(0xFE77)));
        }
        out
    }

    /// The skin of the figure posed as `posed`, a point an anchor (on a
    /// fitted cloud's surface where `fitted` is worn): what a wrap is wound
    /// round (`wraps`).
    fn skin_points(&self, posed: &Posed<'_>, look: &Look<'_>, fitted: Option<&FittedSkin>) -> Vec<crate::wraps::SkinPoint> {
        let unturned = Look { yaw: 0.0, ..*look };
        let surfs = crate::draw::surfaces(posed, &unturned);
        let fitted = fitted.filter(|f| f.offset.len() == self.anchors.len());
        let mut out = Vec::new();
        for surf in &surfs {
            let i = surf.index;
            let Some(&(from, to)) = self.spans.get(i) else { continue };
            let s = &posed.puppet.segments[i];
            if matches!(s.kind, crate::Kind::Prop { .. }) {
                continue;
            }
            let kept = surf.rings.len().min(s.stations.len());
            if kept == 0 {
                continue;
            }
            for (ai, a) in self.anchors[from..to].iter().enumerate() {
                if a.station > (kept - 1) as f32 {
                    continue;
                }
                let k = (a.station.floor() as usize).min(kept - 1);
                let k1 = (k + 1).min(kept - 1);
                let t = a.station - k as f32;
                let (r0, r1) = (&surf.rings[k], &surf.rings[k1]);
                let (c, ra, rb) = (lerp3(r0.c, r1.c, t), lerp3(r0.a, r1.a, t), lerp3(r0.b, r1.b, t));
                let (sin, cos) = a.theta.sin_cos();
                let radial = add(scale(ra, cos), scale(rb, sin));
                let proud = fitted.map_or(0.0, |f| f.offset[from + ai]);
                let p = add(add(c, radial), scale(norm_or(radial, [0.0, 0.0, 1.0]), proud));
                // facing out of the ring's ellipse, tilted as the part narrows along it
                let ellipse = add(scale(ra, cos / dot(ra, ra).max(1e-9)), scale(rb, sin / dot(rb, rb).max(1e-9)));
                let ellipse = norm_or(ellipse, norm_or(radial, [0.0, 0.0, 1.0]));
                let along = if k1 > k { sub(r1.c, r0.c) } else if k > 0 { sub(r0.c, surf.rings[k - 1].c) } else { [0.0; 3] };
                let (k0, k2) = if k1 > k { (k, k1) } else { (k.saturating_sub(1), k) };
                let narrows = len(radial_of(&surf.rings[k0], sin, cos)) - len(radial_of(&surf.rings[k2], sin, cos));
                let n = norm_or(add(ellipse, scale(norm_or(along, [0.0; 3]), narrows / len(along).max(1e-4))), ellipse);
                let hidden = inside_other(&surfs, a, p);
                out.push(crate::wraps::SkinPoint { p, n, anchor: (from + ai) as u32, seg: i, hidden });
            }
        }
        out
    }

    /// The segments `look.fresh` touches (by segment index): the hit part's,
    /// and on a back hit each segment its back wound is split across. Only
    /// these change while a hit arrives.
    pub fn fresh_segments(&self, posed: &Posed<'_>, look: &LayerLook<'_>) -> Vec<bool> {
        let puppet = posed.puppet;
        let mut out = vec![false; puppet.segments.len()];
        let Some(fresh) = look.fresh else { return out };
        let unturned = Look { yaw: 0.0, ..look.look };
        let surfs: Vec<Surface> = crate::draw::surfaces(posed, &unturned);
        let marks = crate::skin::segment_marks(puppet, &surfs, &unturned, look.effects);
        for (i, s) in puppet.segments.iter().enumerate() {
            let split = marks.get(i).is_some_and(|m| m.split);
            out[i] = s.part.as_deref() == Some(fresh.part) || (split && fresh.part == "back");
        }
        out
    }

    /// The dots of the segments `only` marks (all: None), what is thrown
    /// off as `thrown` says, and the pits and dents those segments' wounds
    /// made. `around`: other segments' pits as well (those of a segment
    /// `only` does not mark), clearing these segments' dots out of them.
    fn dots_of(&self, posed: &Posed<'_>, look: &LayerLook<'_>, only: Option<&[bool]>, thrown: Thrown, around: &[Hollow]) -> (Vec<Dot>, Vec<Particle>, Vec<Hollow>) {
        let puppet = posed.puppet;
        let spacing = self.params.spacing.max(0.005);
        let overlap = self.params.overlap.clamp(0.2, 2.0);
        let alpha = self.params.alpha.clamp(0.05, 1.0);
        let min_half = spacing * 0.5;
        let unturned = Look { yaw: 0.0, ..look.look };
        let surfs: Vec<Surface> = crate::draw::surfaces(posed, &unturned);
        let (_, stump) = crate::draw::severed(posed, &unturned);
        // vines, ropes, blood, poison's veins and disease's boils are built
        // on the body (`status_layers`, `blood`, `afflict`), not painted:
        // poison and disease only tint it (below)
        let painted_fx = crate::skin::Effects { rooted: false, bound: false, bleeding: false, poisoned: false, diseased: false, hidden: false, possessed: false, ..look.effects };
        let marks = crate::skin::segment_marks(puppet, &surfs, &unturned, painted_fx);
        // the sheet paints everything but the wounds themselves, which are
        // gashes here (a stump keeps its paint)
        let painted: Vec<wounds::Marks> = marks.iter().enumerate().map(|(i, m)| if stump[i] { *m } else { wounds::Marks { rank: 0, back: 0, ..*m } }).collect();
        let sheet = painted.iter().any(|m| m.any()).then(|| crate::skin::marks_sheet(puppet, &self.surface, &surfs, &painted, min_half));
        let body = (if unturned.dead { BODY_DEAD } else { unturned.body }).map(|c| c as f32);
        let thin = spacing * 0.15;
        // the coating in force: the host's, else a frozen figure's -- ice
        // head to foot, over its wounds (it froze once hurt)
        let coat = look.coat.or_else(|| look.effects.frozen.then(|| Coating::new(Material::Ice, 1.0).over_wounds(true)));
        let coat = coat.as_ref();
        let surf_of = |seg: usize| surfs.iter().find(|s| s.index == seg);
        // a fitted cloud worn as the skin, moved into this pose
        let fitted = look.fitted.filter(|f| f.by_anchor.len() == self.anchors.len()).map(|f| (f, f.bound.posed(posed)));
        let fitted = fitted.as_ref();
        // dead, wearing a fitted cloud: how much of it is skin, anchor by
        // anchor (`dead_skin`), so the pallor goes on its skin and its
        // clothes keep their colours
        // (frenzied or overexerted, the same: its flush goes on its skin)
        let flushed = look.effects.frenzied || look.effects.overexerted;
        let dead_skin = fitted.filter(|_| unturned.dead || flushed).and_then(|(skin, _)| dead_skin(puppet, &self.anchors, &skin.tone));
        let per_surf = |surf: &Surface, out: &mut Vec<Dot>, hollows: &mut Vec<Hollow>, particles: &mut Vec<Particle>, wells: &mut Vec<crate::blood::Well>| {
            let i = surf.index;
            let Some(&(from, to)) = self.spans.get(i) else { return };
            let s = &puppet.segments[i];
            let m = &marks[i];
            // a stump keeps its first rings; `surfaces` adds four for its end
            let kept = if stump[i] { surf.rings.len().saturating_sub(4) } else { surf.rings.len() }.min(s.stations.len());
            if kept == 0 {
                return;
            }
            let own = match s.kind {
                crate::Kind::Prop { color } => Some(color.map(|c| c as f32)),
                _ => None,
            };
            let base = surf.base.map(|c| c as f32);
            // what made its wound, and whether it was just hit
            let cause_of = |part: &str| look.causes.iter().find(|(p, _)| *p == part).map_or(Cause::default(), |(_, c)| *c);
            let crit_of = |part: &str| look.crits.iter().find(|(p, _)| *p == part).map_or(0, |(_, c)| *c);
            let part = s.part.as_deref();
            let age = look.fresh.filter(|f| part == Some(f.part) || (m.split && f.part == "back")).map(|f| f.age.max(0.0));
            let open = age.map_or(1.0, |t| smoothstep(0.0, OPEN_SECS, t));
            let flash = age.map_or(0.0, |t| (-t / FLASH_SECS).exp());
            let hot = age.map_or(0.0, |t| (-t / COOL_SECS).exp());
            // its wounds: a few marks, placed away from where it meets
            // another part, a split trunk's own on its front and the
            // back's on its rear
            let slice = &self.anchors[from..to];
            let (mut gashed, mut front_gashes): (Vec<Gash>, usize) = if s.kind.is_body() && !stump[i] && slice.len() >= GASH_MIN_ANCHORS && (m.rank > 0 || m.back > 0) {
                let radius = slice.iter().map(|a| a.radius).sum::<f32>() / slice.len() as f32;
                let tint_at = |a: &Anchor| {
                    let k = (a.station.floor() as usize).min(kept - 1);
                    let (t0, t1) = (surf.rings[k].tint, surf.rings[(k + 1).min(kept - 1)].tint);
                    t0 + (t1 - t0) * (a.station - k as f32)
                };
                let places = |want: &dyn Fn(&Anchor) -> bool| -> Vec<&Anchor> {
                    // along the middle of its length: not at a joint, a crotch, a collar
                    let (lo, hi) = (0.18 * (kept - 1) as f32, 0.82 * (kept - 1) as f32);
                    slice.iter().filter(|a| a.feet.is_some() && a.station >= lo && a.station <= hi && tint_at(a) > 0.75 && want(a)).collect()
                };
                let mut own = places(&|a| a.front > if m.split { 0.35 } else { -0.2 });
                if own.is_empty() {
                    own = places(&|_| true);
                }
                let kind = cause_of(part.unwrap_or(""));
                let mut list = gashes(i as u32, m.rank, kind, 0xA, &own, radius, open, crit_of(part.unwrap_or("")), &[]);
                let front = list.len();
                if m.split {
                    // the worst of a wound goes through: the back's marks,
                    // made by the same thing, come out behind the front's
                    let through: Vec<(f32, f32)> = if m.rank >= 3 && cause_of("back") == kind { list.iter().map(|g| g.from).collect() } else { Vec::new() };
                    list.extend(gashes(i as u32, m.back, cause_of("back"), 0xBAC, &places(&|a| a.front < -0.35), radius, open, crit_of("back"), &through));
                }
                (list, front)
            } else {
                (Vec::new(), 0)
            };
            // a cut throat: a deep slash straight across the front of the
            // neck, among its front wounds, bleeding whatever else does
            if look.effects.cutthroat && part == Some("neck") && s.kind.is_body() && !stump[i] {
                // on the throat that shows: not inside the head or the chest
                // at rest (a neck's top rings run up under the jaw), facing
                // ahead, midway along what shows
                // -- and, where it has eyes, as far under them as a throat is
                // (a modelled body's head is not the rig's: its neck shows
                // lower than the puppet's rings say)
                // (with a fitted cloud worn, a neck ring the chest hides at rest
                // stands on the cloud's own throat: `skin_points` judges what
                // shows, there)
                let mut seen: Vec<&Anchor> = slice.iter().filter(|a| a.feet.is_some() && (a.inside.is_none() || fitted.is_some()) && a.front > 0.7).collect();
                seen.sort_by(|a, b| a.station.total_cmp(&b.station));
                let eyes = &puppet.eyes;
                let throat_up = (!eyes.is_empty()).then(|| (eyes.iter().map(|e| e.pos[1]).sum::<f32>() / eyes.len() as f32 - THROAT_UNDER_EYES_FT) / puppet.height.max(0.1));
                let front = match throat_up {
                    Some(t) => seen.iter().copied().min_by(|a, b| ((a.up - t).abs() - 0.03 * a.front).total_cmp(&((b.up - t).abs() - 0.03 * b.front))),
                    None => {
                        let mid = seen.get(seen.len() / 2).map(|a| a.station);
                        mid.and_then(|m| seen.iter().copied().filter(|a| (a.station - m).abs() < 0.75).max_by(|a, b| a.front.total_cmp(&b.front)))
                    }
                };
                if let Some((a, c)) = front.and_then(|a| Some((a, a.feet?))) {
                    let throat = Gash {
                        kind: Cause::Slash,
                        c,
                        // round the ring: across the throat
                        dir: [1.0, 0.0],
                        half_len: (a.radius * 1.25).min(0.24),
                        half_w: 0.035 * (0.2 + 0.8 * open),
                        depth: 0.075f32.min(a.radius * GASH_MAX_OF_RADIUS) * open,
                        bone: false,
                        phase: 0.6,
                        seed: THROAT_SEED,
                        from: (a.station, a.theta),
                    };
                    gashed.insert(front_gashes, throat);
                    front_gashes += 1;
                }
            }
            // bleeding: which of its wounds are open, and the skin in and
            // round each (`blood`)
            // (and where poison got in: `afflict`)
            // (dead, its open wounds have bled out under it: `blood`, darkened)
            let bleeds: Vec<bool> = gashed.iter().map(|g| (look.effects.bleeding || look.effects.poisoned || unturned.dead || g.seed == THROAT_SEED) && g.kind.opens() && g.depth > 0.0).collect();
            let mut lips: Vec<Vec<crate::blood::Lip>> = vec![Vec::new(); gashed.len()];
            // the deepest pit at a surface point, and the nearest gash's
            // `across` there
            let pit = |at: [f32; 2]| {
                gashed.iter().fold((0.0f32, f32::MAX, None), |(d, u, g), x| {
                    let across = x.across(at);
                    let deep = x.depth_at(across);
                    if deep > d || (d <= 0.0 && across < u) {
                        (deep.max(d), u.min(across), Some(x))
                    } else {
                        (d, u.min(across), g)
                    }
                })
            };
            // where each mark's middle is in the world, for what it throws off
            let mut middle: Vec<(f32, V3, V3)> = vec![(f32::MAX, [0.0; 3], [0.0; 3]); gashed.len()];
            let bone_dir = norm_or(sub(surf.rings[kept - 1].c, surf.rings[0].c), norm_or(cross(surf.rings[0].a, surf.rings[0].b), [0.0, 1.0, 0.0]));
            // where an anchor is on the skin as posed: its point, which way it
            // faces, round the ring and along the bone (as the dots below)
            let place = |a: &Anchor| {
                let st = a.station.min((kept - 1) as f32);
                let k = (st.floor() as usize).min(kept - 1);
                let k1 = (k + 1).min(kept - 1);
                let t = st - k as f32;
                let (r0, r1) = (&surf.rings[k], &surf.rings[k1]);
                let (c, ra, rb) = (lerp3(r0.c, r1.c, t), lerp3(r0.a, r1.a, t), lerp3(r0.b, r1.b, t));
                let (sin, cos) = a.theta.sin_cos();
                let radial = add(scale(ra, cos), scale(rb, sin));
                let tangent = norm_or(sub(scale(rb, cos), scale(ra, sin)), [1.0, 0.0, 0.0]);
                let step = if k1 > k {
                    norm_or(sub(r1.c, r0.c), bone_dir)
                } else if k > 0 {
                    norm_or(sub(r0.c, surf.rings[k - 1].c), bone_dir)
                } else {
                    bone_dir
                };
                let nrm = norm_or(cross(tangent, step), norm_or(radial, [0.0, 0.0, 1.0]));
                let nrm = if dot(nrm, radial) < 0.0 { scale(nrm, -1.0) } else { nrm };
                (add(c, radial), nrm, tangent, step)
            };
            // how far an anchor's skin is pushed in or out by a wound there
            // (None: a wound has opened or burnt it away)
            let skin_shift = |a: &Anchor| -> Option<f32> {
                let Some(feet) = a.feet.filter(|_| !gashed.is_empty()) else { return Some(0.0) };
                let (deep, across, gash) = pit(feet);
                match gash {
                    Some(g) if g.kind.opens() && deep > 0.0 => None,
                    Some(g) if g.kind.base() == Cause::Fire && across < 1.15 => None,
                    Some(g) if g.kind == Cause::Vacuum => Some(deep),
                    Some(g) if !g.kind.opens() => Some(-deep),
                    _ => Some(0.0),
                }
            };
            let inside_other = |a: &Anchor, p: V3| inside_other(&surfs, a, p);
            for (ai, whole) in self.anchors[from..to].iter().enumerate() {
                if whole.station > (kept - 1) as f32 {
                    continue;
                }
                if let (Some(feet), true) = (whole.feet, bleeds.contains(&true)) {
                    let (p, nrm, _, _) = place(whole);
                    let p = add(p, scale(nrm, fitted.map_or(0.0, |(skin, _)| skin.offset[from + ai])));
                    for (k, g) in gashed.iter().enumerate().filter(|(k, _)| bleeds[*k]) {
                        let across = g.across(feet);
                        if across < 1.3 && !inside_other(whole, p) {
                            lips[k].push(crate::blood::Lip { p, n: nrm, across, anchor: (from + ai) as u32 });
                        }
                    }
                }
                // over a fitted cloud, what lies over the skin is laid once an
                // anchor, on the cloud's surface: the cloud's Gaussians are as
                // many and as big as its fit made them, too uneven to carry it
                let webbed = look.effects.webbed && look.held.is_none();
                let coated = fitted.is_some() && coat.is_some();
                if webbed || coated {
                    if let Some(shift) = skin_shift(whole) {
                        let (p, nrm, tangent, step) = place(whole);
                        let p = add(p, scale(nrm, fitted.map_or(0.0, |(skin, _)| skin.offset[from + ai]) + shift));
                        if !inside_other(whole, p) {
                            let anchor = Some((from + ai) as u32);
                            let at = whole.at();
                            if let (Some(coat), true) = (coat, coated) {
                                let h = coat.height(whole);
                                if h > 0.0 {
                                    let (rgb, alpha) = coat.material.look(at, i as u32);
                                    let axes = [scale(tangent, whole.step[0] * overlap), scale(step, whole.step[1] * overlap), scale(nrm, thin)];
                                    out.push(Dot { layer: Layer::Coat, anchor, source: None, g: Gaussian { c: add(p, scale(nrm, h)), axes, n: nrm, rgb, alpha, glow: 0.0, wet: 0.0, film: coat.material == Material::Ice, rim: 0.0 } });
                                    if let Some(g) = ice_under(coat.material, add(p, scale(nrm, h * 0.45)), axes, nrm, rgb, alpha) {
                                        out.push(Dot { layer: Layer::Coat, anchor, source: None, g });
                                    }
                                }
                            }
                            if webbed {
                                // each strand near: a thin film along it, laid on its middle
                                let reach = whole.step[0].max(whole.step[1]) * 0.55;
                                for (d, [dx, dy]) in crate::skin::web_strands(at) {
                                    if d.abs() > reach {
                                        continue;
                                    }
                                    let across = norm_or(add(scale(tangent, dx), scale(step, dy)), tangent);
                                    let along = norm_or(cross(nrm, across), step);
                                    let c = add(sub(p, scale(across, d)), scale(nrm, WEB_LIFT_FT));
                                    let axes = [scale(along, reach * 1.6), scale(across, WEB_HALF_FT), scale(nrm, WEB_HALF_FT * 0.5)];
                                    out.push(Dot { layer: Layer::Web, anchor, source: None, g: Gaussian { c, axes, n: nrm, rgb: crate::skin::WEB_RGB, alpha: 0.85, glow: 0.0, wet: 0.0, film: true, rim: 0.0 } });
                                }
                            }
                        }
                    }
                }
                // a fitted cloud's Gaussians here, each at its own address;
                // else the made dot, finer near a wound
                let worn = fitted.map(|(skin, _)| skin.by_anchor[from + ai].as_slice());
                let fine = !gashed.is_empty() && whole.feet.is_some_and(|f| pit(f).1 < DETAIL_REACH);
                let grain = if fine { spacing / DETAIL as f32 } else { spacing };
                // Over a fitted cloud the made dots are drawn only near a
                // wound, and are the wound itself: its pit in place of the
                // cloud's Gaussians there (big and long as those are, sunk
                // and tinted one by one they smeared it), its blood and
                // bruising laid over the ones round it.
                let n_worn = worn.map_or(0, |list| list.len());
                let n_made = match (fine, worn.is_some()) {
                    (true, _) => DETAIL * DETAIL,
                    (false, true) => 0,
                    (false, false) => 1,
                };
                for piece in 0..n_worn + n_made {
                let dressed = worn.zip(fitted).filter(|_| piece < n_worn).map(|(list, (skin, moved))| (skin.own[list[piece] as usize], &moved[list[piece] as usize]));
                let source = worn.filter(|_| piece < n_worn).map(|list| list[piece]);
                let overlay = fitted.filter(|_| piece >= n_worn).map(|(skin, _)| (skin.offset[from + ai], skin.tone[from + ai]));
                let mut a = match dressed {
                    Some((own, _)) => own,
                    None if fine => whole.split(piece - n_worn, DETAIL, &self.surface),
                    None => *whole,
                };
                a.station = a.station.min((kept - 1) as f32);
                let a = &a;
                let k = (a.station.floor() as usize).min(kept - 1);
                let k1 = (k + 1).min(kept - 1);
                let t = a.station - k as f32;
                let (r0, r1) = (&surf.rings[k], &surf.rings[k1]);
                let (c, ra, rb) = (lerp3(r0.c, r1.c, t), lerp3(r0.a, r1.a, t), lerp3(r0.b, r1.b, t));
                let tint = r0.tint + (r1.tint - r0.tint) * t;
                let (sin, cos) = a.theta.sin_cos();
                let radial = add(scale(ra, cos), scale(rb, sin));
                let p = add(c, radial);
                let tangent = sub(scale(rb, cos), scale(ra, sin));
                // along the step to the next ring, as `splat::cloud`
                let step = if k1 > k {
                    norm_or(sub(r1.c, r0.c), bone_dir)
                } else if k > 0 {
                    norm_or(sub(r0.c, surf.rings[k - 1].c), bone_dir)
                } else {
                    bone_dir
                };
                let nrm = norm_or(cross(tangent, step), norm_or(radial, [0.0, 0.0, 1.0]));
                let nrm = if dot(nrm, radial) < 0.0 { scale(nrm, -1.0) } else { nrm };
                let axes = [scale(norm_or(tangent, [1.0, 0.0, 0.0]), a.step[0] * overlap), scale(step, a.step[1] * overlap), scale(nrm, thin)];
                let anchor = Some((from + ai) as u32);
                let at = a.at();
                // where the cloud has it, in its own shape, colour and light
                let dressed = dressed.map(|(_, g)| g);
                let (p, axes) = dressed.map_or((p, axes), |g| (g.c, g.axes));
                // (a mark laid over the cloud: on the cloud's surface, over its colour there)
                let p = overlay.map_or(p, |(off, _)| add(p, scale(nrm, off)));
                let mut rgb = dressed
                    .map(|g| g.rgb)
                    .or(overlay.map(|(_, tone)| tone))
                    .or(own)
                    .or_else(|| look.texture.and_then(|tx| tx.layout.locate(i, a.station, a.theta).and_then(|uv| tx.sample(uv))))
                    .unwrap_or_else(|| if stump[i] { wounds::mix(body, base, tint) } else { body });
                // dead: a grey-blue pallor over its skin (all of it, where
                // there is no telling skin from clothes), its clothes dulled,
                // and the marks over that
                if unturned.dead && !stump[i] {
                    let skin = dead_skin.as_ref().map_or(1.0, |w| w[from + ai]);
                    rgb = wounds::mix(rgb, DEAD_SKIN, 0.72 * skin);
                    let grey = (rgb[0] + rgb[1] + rgb[2]) / 3.0;
                    rgb = wounds::mix(rgb, [grey; 3], 0.3 * (1.0 - skin)).map(|c| c * (1.0 - 0.08 * (1.0 - skin)));
                }
                // hidden: sunk in shadow, most at its feet (`shade`), under
                // the marks -- its wounds stay as red as ever against the dark
                if look.effects.hidden && !stump[i] {
                    rgb = crate::shade::shaded(rgb, a.up);
                }
                // possessed: gone cold toward a dead violet (`possess`)
                if look.effects.possessed && !stump[i] {
                    rgb = crate::possess::tinted(rgb);
                }
                // frenzied, overexerted: flushed (`heat`), its skin only
                // where its skin can be told from its clothes
                if flushed && !stump[i] {
                    let skin = dead_skin.as_ref().map_or(1.0, |w| w[from + ai]);
                    rgb = crate::heat::flushed(rgb, look.effects.frenzied, skin);
                }
                // poisoned, a sickly cast; diseased, a pallor
                if look.effects.poisoned {
                    rgb = wounds::mix(rgb, wounds::POISON_TINT_RGB, 0.14);
                }
                if look.effects.diseased {
                    rgb = wounds::mix(rgb, wounds::DISEASE_PALE_RGB, 0.25);
                }
                // the marks sheet: its colour, and how much it glows (nerve
                // marks) and how wet it is (fresh blood) there
                let (mut marks_glow, mut wet) = (0.0f32, 0.0f32);
                if let (Some(sheet), Some(feet), true) = (&sheet, a.feet, painted[i].any()) {
                    (rgb, marks_glow, wet) = sheet.apply(rgb, feet);
                }
                // iced over: the surface under the ice shines as wet does
                // poisoned: a sweat's sheen; overexerted, more of one
                if look.effects.poisoned {
                    wet = wet.max(POISON_SWEAT);
                }
                if look.effects.overexerted && !stump[i] {
                    wet = wet.max(crate::heat::SWEAT * dead_skin.as_ref().map_or(1.0, |w| w[from + ai]));
                }
                if coat.is_some_and(|c| c.material == Material::Ice && c.height(a) > 0.0) {
                    wet = wet.max(ICE_SHINE);
                }
                let plain = rgb;
                let (deep, across, gash) = match a.feet {
                    Some(feet) if !gashed.is_empty() => pit(feet),
                    _ => (0.0, f32::MAX, None),
                };
                if let Some(feet) = a.feet {
                    for (near, g) in middle.iter_mut().zip(&gashed) {
                        let u = g.across(feet);
                        if u < near.0 {
                            *near = (u, p, nrm);
                        }
                    }
                }
                if deep > 0.0 && gash.is_some_and(|g| g.kind != Cause::Vacuum) {
                    hollows.push(Hollow { seg: i, p, n: nrm, deep, r: a.step[0].max(a.step[1]) });
                }
                let seg = i as u32;
                let mut glow = marks_glow;
                let mut skin_alpha = dressed.map_or(alpha, |g| g.alpha);
                // a crust standing on it (frost): its height, colour, opacity
                let mut crust = None;
                if let Some(g) = gash {
                    let mottle = wounds::fbm([at[0] / 0.09, at[1] / 0.09], seg ^ 0xB2);
                    match g.kind.base() {
                        // bruised round it, bloodied at its rim; a scratch
                        // (no depth) is a dark line of clotted blood
                        Cause::Slash | Cause::Puncture => {
                            let bruise = wounds::mix(wounds::OLD_BRUISE, wounds::BRUISE, if g.depth > 0.0 { 0.8 } else { 0.4 });
                            rgb = wounds::mix(rgb, bruise, (1.0 - smoothstep(1.3, 4.0, across)) * (0.3 + 0.5 * mottle));
                            rgb = wounds::mix(rgb, wounds::BLOOD, (1.0 - smoothstep(1.0, 1.7, across)) * 0.85);
                            if g.depth <= 0.0 {
                                rgb = wounds::mix(rgb, wounds::CLOT, 1.0 - smoothstep(0.6, 1.0, across));
                            }
                        }
                        // a deep, mottled bruise, darkest in the middle
                        Cause::Crush => {
                            let bruise = wounds::mix(wounds::OLD_BRUISE, wounds::BRUISE, mottle);
                            rgb = wounds::mix(rgb, bruise, (1.0 - smoothstep(0.6, 1.7, across)) * (0.6 + 0.35 * mottle));
                            rgb = wounds::mix(rgb, wounds::CLOT, (1.0 - smoothstep(0.0, 0.8, across)) * 0.3);
                        }
                        // scorched round it, a crust over it (char; a scald's
                        // blisters; flesh acid has eaten raw); a fire's and a
                        // plasma's cracks glow -- white hot when just hit,
                        // embers after
                        Cause::Fire => {
                            let burn = g.kind.burn();
                            rgb = wounds::mix(rgb, burn.scorch, (1.0 - smoothstep(1.0, 2.0, across)) * 0.75);
                            let crust = if g.kind == Cause::Acid { wounds::mix(burn.crust, [150.0, 150.0, 40.0], smoothstep(0.5, 0.7, mottle)) } else { burn.crust };
                            rgb = wounds::mix(rgb, crust, (1.0 - smoothstep(0.7, 1.1, across)) * burn.cover);
                            if burn.embers > 0.0 {
                                let heat = (1.0 - across).clamp(0.0, 1.0);
                                let cracks = smoothstep(0.52, 0.64, wounds::fbm([at[0] / 0.035, at[1] / 0.035], seg ^ 0xF12E));
                                let ember = (cracks * (heat * 1.6).min(1.0) * (burn.keep + (1.0 - burn.keep) * hot) + heat * hot * 0.6).min(1.0) * burn.embers;
                                rgb = wounds::mix(rgb, wounds::mix(burn.ember.0, burn.ember.1, heat * hot), ember);
                                // and its light on the skin round it: bright
                                // when just hit, a faint warmth after
                                let spill = (1.0 - across / 3.0).clamp(0.0, 1.0).powi(2) * (0.12 + 0.88 * hot);
                                let warm = wounds::mix(burn.ember.0, burn.ember.1, 0.3);
                                rgb = [0, 1, 2].map(|c| (rgb[c] + warm[c] * spill * 0.6).min(255.0));
                                glow = ember.max(spill * 0.5);
                            }
                        }
                        // pale and blue round it, a crust of frost on it
                        Cause::Cold => {
                            rgb = wounds::mix(rgb, [150.0, 182.0, 222.0], (1.0 - smoothstep(0.7, 2.0, across)) * 0.75);
                            if across < 1.0 {
                                let grown = age.map_or(1.0, |t| smoothstep(0.0, 0.8, t));
                                let h = FROST_FT * (1.0 - across * across) * (0.4 + 0.6 * wounds::fbm([at[0] / 0.05, at[1] / 0.05], seg ^ 0xF057)) * grown;
                                if h > 0.001 {
                                    crust = Some((h, wounds::mix(Material::Ice.look(at, seg).0, [240.0, 248.0, 255.0], 0.4), 0.85));
                                }
                            }
                        }
                        // a dark burnt line, the skin red along it, tingling
                        // with light while it is fresh; disruption's is a
                        // dark vein under the skin
                        Cause::Lightning => {
                            let (halo, line, lit) = if g.kind == Cause::Disruption {
                                ([126.0, 96.0, 156.0], [44.0, 20.0, 62.0], [170.0, 120.0, 255.0])
                            } else {
                                ([196.0, 84.0, 96.0], [70.0, 18.0, 30.0], [190.0, 215.0, 255.0])
                            };
                            rgb = wounds::mix(rgb, halo, (1.0 - smoothstep(1.0, 3.5, across)) * 0.65);
                            rgb = wounds::mix(rgb, line, 1.0 - smoothstep(0.4, 1.0, across));
                            glow = (1.0 - smoothstep(0.3, 1.0, across)) * hot * 0.8;
                            rgb = wounds::mix(rgb, lit, glow);
                        }
                        // gone to ash, its edge frayed and, while fresh,
                        // alight; the skin round it greyed
                        Cause::Disintegrate => {
                            let fray = wounds::noise([at[0] / 0.02, at[1] / 0.02], seg ^ 0xD15);
                            let gone = 1.0 - smoothstep(0.75, 1.5, across + (fray - 0.5) * 0.7);
                            rgb = wounds::mix(rgb, [150.0, 146.0, 156.0], (1.0 - smoothstep(1.2, 2.2, across)) * 0.35);
                            rgb = wounds::mix(rgb, wounds::mix([86.0, 82.0, 90.0], [38.0, 36.0, 44.0], wounds::fbm([at[0] / 0.04, at[1] / 0.04], seg ^ 0xA5)), gone);
                            glow = (1.0 - ((across - 1.0) / 0.35).abs()).clamp(0.0, 1.0) * hot * 0.7;
                            rgb = wounds::mix(rgb, [226.0, 214.0, 255.0], glow);
                        }
                        // not all there: pale, thin, faintly lit
                        Cause::NonCorporeal => {
                            let fade = 1.0 - smoothstep(0.4, 1.5, across);
                            rgb = wounds::mix(rgb, [206.0, 220.0, 236.0], fade * 0.8);
                            skin_alpha *= 1.0 - 0.6 * fade;
                            glow = fade * (0.12 + 0.5 * hot);
                        }
                        // swollen out, bruised dark, burst in blood
                        Cause::Vacuum => {
                            let bruise = wounds::mix(wounds::OLD_BRUISE, wounds::BRUISE, mottle);
                            rgb = wounds::mix(rgb, bruise, (1.0 - smoothstep(0.7, 1.8, across)) * 0.8);
                            let burst = smoothstep(0.56, 0.62, wounds::fbm([at[0] / 0.04, at[1] / 0.04], seg ^ 0x7AC)) * (1.0 - smoothstep(0.6, 1.1, across));
                            rgb = wounds::mix(rgb, wounds::BLOOD, burst * 0.9);
                        }
                        // (`base` names no other)
                        _ => {}
                    }
                    // the hit's flash
                    let (flash_rgb, reach) = g.kind.flash();
                    let lit = flash * (1.0 - across / reach);
                    if lit > 0.02 {
                        rgb = wounds::mix(rgb, flash_rgb, lit);
                        glow = glow.max(lit);
                    }
                }
                // how far outside an open wound's rim this is, feet (None: by no open wound)
                let outside = gash.filter(|g| g.kind.opens() && g.depth > 0.0).map(|g| ((across - 1.0) * g.half_w).max(0.0));
                // the cloud's Gaussians in an open wound give way to its pit,
                // and so do those beside it that would reach across it
                if dressed.is_some() && outside.is_some_and(|d| deep > 0.0 || d < axes.iter().map(|a| len(*a)).fold(0.0, f32::max).min(PATCH_FT)) {
                    continue;
                }
                match gash.filter(|g| g.kind.opens() && deep > 0.0) {
                    None => {
                        // whole skin: standing up a little round an open
                        // gash, pressed in by a dent or a burn
                        let lift = match gash {
                            Some(g) if g.kind.opens() => {
                                if g.depth > 0.0 && across < 2.0 {
                                    LIP_FT * (1.0 - smoothstep(1.0, 2.0, across))
                                } else {
                                    0.0
                                }
                            }
                            // a vacuum's swelling stands out; the rest are pressed in
                            Some(g) if g.kind == Cause::Vacuum => deep,
                            _ => -deep,
                        };
                        match (dressed.is_some(), overlay.is_some()) {
                            // a fitted Gaussian keeps its own colour and
                            // light (it is not shaded); the marks go over it
                            (true, _) => out.push(Dot { layer: Layer::Skin, anchor, source, g: Gaussian { c: add(p, scale(nrm, lift)), axes, n: [0.0; 3], rgb: plain, alpha: skin_alpha, glow: marks_glow, wet, film: false, rim: 0.0 } }),
                            // those marks: as strong as they change the
                            // colour under them, or glow
                            (false, true) => {
                                let change = (0..3).map(|c| (rgb[c] - plain[c]).abs()).fold(0.0, f32::max) / 90.0;
                                // (whole skin, where the cloud's own was taken out)
                                let strength = if outside.is_some_and(|d| d < PATCH_FT) { 1.0 } else { change.max(glow).min(1.0) };
                                if strength > 0.06 {
                                    out.push(Dot { layer: Layer::Skin, anchor, source: None, g: Gaussian { c: add(p, scale(nrm, lift + OVERLAY_FT)), axes, n: nrm, rgb, alpha: alpha * strength, glow, wet, film: false, rim: 0.0 } });
                                }
                            }
                            (false, false) => out.push(Dot { layer: Layer::Skin, anchor, source: None, g: Gaussian { c: add(p, scale(nrm, lift)), axes, n: nrm, rgb, alpha: skin_alpha, glow, wet, film: false, rim: 0.0 } }),
                        }
                        if let Some((h, rgb, alpha)) = crust {
                            out.push(Dot { layer: Layer::Coat, anchor, source: None, g: Gaussian { c: add(p, scale(nrm, h)), axes, n: nrm, rgb, alpha, glow, wet: 0.0, film: false, rim: 0.0 } });
                        }
                    }
                    Some(g) => {
                        // in the pit: its slope round the ring and along the
                        // bone, for which way its surface faces here
                        let slope = |to: Option<[f32; 2]>, run: f32| to.map_or(0.0, |f| ((pit(f).0 - deep) / run.max(1e-4)).clamp(-6.0, 6.0));
                        let gx = slope(self.surface.locate(i, a.station, a.theta + a.step[0] * a.turn), a.step[0]);
                        let gy = if a.rate > 0.0 { slope(self.surface.locate(i, a.station + a.step[1] * a.rate, a.theta), a.step[1]) } else { 0.0 };
                        let facing = norm_or(add(nrm, add(scale(norm_or(tangent, [1.0, 0.0, 0.0]), gx), scale(step, gy))), nrm);
                        // what the body is made of that deep, darker the deeper
                        let stuff = |d: f32| {
                            let (layer, c) = if d < SKIN_FT {
                                (Layer::Skin, wounds::mix(rgb, wounds::CLOT, 0.7 * (1.0 - glow)))
                            // an open wound is RED inside: the fat under
                            // the skin and the bone at the bottom are seen
                            // through blood, the bone only in its very middle
                            } else if d < FAT_FT {
                                (Layer::Fat, wounds::mix(wounds::fat(at, seg), wounds::BLOOD, 0.65))
                            } else if g.bone && d > g.depth * 0.93 {
                                (Layer::Bone, wounds::mix(wounds::bone(at, seg), wounds::BLOOD, 0.88))
                            } else {
                                (Layer::Flesh, wounds::mix(wounds::flesh(at, seg), [150.0, 14.0, 16.0], 0.45))
                            };
                            (layer, c.map(|v| v * (1.0 - 0.45 * (d / 0.08).min(1.0))))
                        };
                        // the floor, and up a steep wall as many more as close it
                        let rise = gx.hypot(gy) * a.step[0].max(a.step[1]);
                        let more = ((rise / (grain * 0.6)).ceil() as usize).min(12);
                        for k in 0..=more {
                            let d = (deep - rise * k as f32 / (more + 1) as f32).max(0.0);
                            let (layer, rgb) = stuff(d);
                            out.push(Dot { layer, anchor, source: None, g: Gaussian { c: sub(p, scale(nrm, d)), axes, n: facing, rgb, alpha, glow, wet: wet.max(0.5), film: false, rim: 0.0 } });
                        }
                    }
                }
                // What is on the skin goes with the skin: pressed in with a
                // dent, raised on a swelling, cut through over an open wound
                // and gone from a burn. And it is one layer: none on skin
                // that is inside another part of the body as posed.
                // That is a wound made AFTER it; one made before it lies
                // under it, the coating whole across an open gash.
                let over = coat.is_some_and(|c| c.over_wounds);
                let under = match gash {
                    Some(g) if over && g.kind.opens() => Some(0.0),
                    Some(g) if g.kind.opens() && deep > 0.0 => None,
                    Some(g) if !over && g.kind.base() == Cause::Fire && across < 1.15 => None,
                    Some(g) if g.kind == Cause::Vacuum => Some(deep),
                    Some(g) if !g.kind.opens() => Some(-deep),
                    _ => Some(0.0),
                };
                let buried = || {
                    a.inside.and_then(|j| surf_of(j as usize)).is_some_and(|o| {
                        let n = o.rings.len();
                        let axis = norm_or(sub(o.rings[n - 1].c, o.rings[0].c), [0.0, 1.0, 0.0]);
                        in_hull(o.rings.iter().map(|r| (r.c, r.a, r.b)), axis, crate::model::STATION_STEP_FT * 0.6, p)
                    })
                };
                // (over a fitted cloud the coat is laid an anchor at a time, above)
                if let (Some(coat), Some(shift), None, None) = (coat, under, overlay, fitted) {
                    let h = coat.height(a);
                    if h > 0.0 && !buried() {
                        let (rgb, alpha) = coat.material.look(at, i as u32);
                        out.push(Dot { layer: Layer::Coat, anchor, source: None, g: Gaussian { c: add(p, scale(nrm, h + shift)), axes, n: nrm, rgb, alpha, glow: 0.0, wet: 0.0, film: coat.material == Material::Ice, rim: 0.0 } });
                        if let Some(g) = ice_under(coat.material, add(p, scale(nrm, h * 0.45 + shift)), axes, nrm, rgb, alpha) {
                            out.push(Dot { layer: Layer::Coat, anchor, source: None, g });
                        }
                    }
                }
                }
            }
            // what the hit threw off, from each mark's middle
            if let Some(age) = age.filter(|_| thrown != Thrown::Skip) {
                for (g, (near, at, nrm)) in gashed.iter().zip(&middle) {
                    if *near < 2.0 {
                        for p in loose(g, *at, *nrm, alpha) {
                            if thrown == Thrown::Particles {
                                particles.push(p);
                            } else if let Some(g) = p.at(age) {
                                out.push(Dot { layer: Layer::Loose, anchor: None, source: None, g });
                            }
                        }
                    }
                }
            }
            if stump[i] {
                // the cut end: flesh round bone -- or closed skin, healed
                let end = &surf.rings[kept - 1];
                let out_dir = if kept > 1 { norm_or(sub(end.c, surf.rings[kept - 2].c), bone_dir) } else { bone_dir };
                let radius = (len(end.a) + len(end.b)) * 0.5;
                let rounds = ((radius / spacing).ceil() as usize).max(1);
                let raw = surf.rank >= 3;
                for ir in 0..rounds {
                    let f = (ir as f32 + 0.5) / rounds as f32;
                    let circ = std::f32::consts::TAU * radius * f;
                    let n = ((circ / spacing).ceil() as usize).max(1);
                    for j in 0..n {
                        let theta = j as f32 / n as f32 * std::f32::consts::TAU;
                        let (sin, cos) = theta.sin_cos();
                        let radial = add(scale(end.a, cos * f), scale(end.b, sin * f));
                        let tangent = norm_or(sub(scale(end.b, cos), scale(end.a, sin)), [1.0, 0.0, 0.0]);
                        let at = [radial[0] + radial[2], radial[1] + i as f32];
                        let (layer, rgb) = if !raw {
                            (Layer::Skin, wounds::mix(body, base, end.tint))
                        } else if f < STUMP_BONE {
                            (Layer::Bone, wounds::bone(at, i as u32))
                        } else {
                            (Layer::Flesh, wounds::flesh(at, i as u32))
                        };
                        let axes = [scale(tangent, circ / n as f32 * overlap), scale(norm_or(radial, tangent), radius / rounds as f32 * overlap), scale(out_dir, thin)];
                        out.push(Dot { layer, anchor: None, source: None, g: Gaussian { c: add(end.c, radial), axes, n: out_dir, rgb, alpha, glow: 0.0, wet: 0.0, film: false, rim: 0.0 } });
                    }
                }
            }
            for (k, lips) in lips.into_iter().enumerate() {
                if bleeds[k] && middle[k].0 < 2.0 && lips.iter().any(|l| l.across < 1.0) {
                    // (a cut throat bleeds as a deep wound, its soak not so
                    // wide as to bury the cut)
                    let rank = if gashed[k].seed == THROAT_SEED { 2 } else if k < front_gashes { m.rank } else { m.back }.max(1);
                    wells.push(crate::blood::Well { seg: i, rank, age, seed: gashed[k].seed ^ (i as u32).wrapping_mul(0x2545_F491), lips });
                }
            }
            // what its statuses build on it (`status_layers`), on its anchors
            let fx = if look.held.is_some() { crate::skin::Effects { bound: false, ..look.effects } } else { look.effects };
            if own.is_none() && (fx.rooted || fx.bound) {
                let mut by_ring: Vec<Vec<(f32, u32)>> = vec![Vec::new(); kept];
                let mut proud = vec![(0.0f32, 0u32); kept];
                for (ai, a) in self.anchors[from..to].iter().enumerate() {
                    let k = (a.station.round().max(0.0) as usize).min(kept - 1);
                    by_ring[k].push((a.theta, (from + ai) as u32));
                    if let Some((skin, _)) = fitted {
                        proud[k].0 += skin.offset[from + ai];
                        proud[k].1 += 1;
                    }
                }
                let proud: Vec<f32> = proud.iter().map(|(sum, n)| if *n > 0 { sum / *n as f32 } else { 0.0 }).collect();
                let seg = crate::status_layers::Seg { part: part.unwrap_or(""), rings: &surf.rings[..kept], proud: &proud, by_ring: &by_ring, bone: bone_dir, seed: (i as u32).wrapping_mul(0x9E37_79B9) ^ 0x5747, height: puppet.height };
                crate::status_layers::build(&seg, &fx, out);
            }
        };
        // a segment at a time on every core, kept in the segments' order
        let chosen: Vec<&Surface> = surfs.iter().filter(|s| only.is_none_or(|o| o.get(s.index).copied().unwrap_or(false))).collect();
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(16);
        let share = chosen.len().div_ceil(threads).max(1);
        let per_surf = &per_surf;
        type Made = (Vec<Dot>, Vec<Hollow>, Vec<Particle>, Vec<crate::blood::Well>);
        let parts: Vec<Made> = std::thread::scope(|scope| {
            let running: Vec<_> = chosen
                .chunks(share)
                .map(|chunk| {
                    scope.spawn(move || {
                        let (mut out, mut hollows, mut particles, mut wells) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
                        for surf in chunk {
                            per_surf(surf, &mut out, &mut hollows, &mut particles, &mut wells);
                        }
                        (out, hollows, particles, wells)
                    })
                })
                .collect();
            running.into_iter().map(|r| r.join().expect("a segment's dots")).collect()
        });
        let (mut dots, mut hollows, mut particles, mut wells) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for (d, h, p, w) in parts {
            dots.extend(d);
            hollows.extend(h);
            particles.extend(p);
            wells.extend(w);
        }
        // The body's segments overlap (a chest's rings run on inside the
        // belly's, a collarbone lies in the chest), and a wound sunk into
        // one would show the skin of another lying just under it: what
        // another segment has inside a hollow is cleared out of it.
        // invisible: the skin itself pale, seen only edge-on -- a glassy rim
        // round a nearly empty middle (`Gaussian::rim`), glinting
        // (`LayerCloud::motes`); what is in its wounds and built on it
        // still shows, hanging in the air
        if look.effects.invisible {
            for d in dots.iter_mut().filter(|d| d.layer == Layer::Skin) {
                d.g.rim = INVISIBLE_RIM;
                d.g.rgb = wounds::mix(d.g.rgb, GHOST_RGB, 0.55);
                d.g.glow = d.g.glow.max(0.3);
            }
        }
        // hidden (its skin sunk in shadow above, `shade`): shadow clinging
        // round its outline, a haze off its skin, on the part it rides --
        // and none over an open wound, whose flesh it would darken and leave
        // only its wet shine
        if look.effects.hidden {
            const CLEAR_FT: f32 = 0.15;
            let near_wound = |c: V3| hollows.iter().any(|h| len(sub(h.p, c)) < h.r + CLEAR_FT);
            let mut clinging = Vec::new();
            for (k, d) in dots.iter_mut().enumerate().filter(|(_, d)| d.layer == Layer::Skin) {
                let up = d.anchor.and_then(|a| self.anchors.get(a as usize)).map_or(0.5, |a| a.up);
                d.g.wet *= 0.3;
                if let Some(g) = crate::shade::aura(d.g.c, norm_or(d.g.n, [0.0; 3]), up, (k as u32).wrapping_mul(0x9E37_79B9)).filter(|_| !near_wound(d.g.c)) {
                    clinging.push(Dot { layer: Layer::Wrap, anchor: d.anchor, source: None, g });
                }
            }
            dots.extend(clinging);
        }
        let own = hollows.len();
        hollows.extend(around.iter().filter(|h| only.is_none_or(|o| !o.get(h.seg).copied().unwrap_or(false))).cloned());
        if !hollows.is_empty() {
            const CELL: f32 = 0.05;
            let cell = |p: V3| [(p[0] / CELL).floor() as i32, (p[1] / CELL).floor() as i32, (p[2] / CELL).floor() as i32];
            let mut grid: std::collections::HashMap<[i32; 3], Vec<usize>> = std::collections::HashMap::new();
            for (k, h) in hollows.iter().enumerate() {
                // filed under every cell its column passes through
                let steps = ((h.deep + HOLLOW_OVER_FT) / CELL).ceil() as i32 + 1;
                for j in 0..=steps {
                    let at = cell(sub(h.p, scale(h.n, -HOLLOW_OVER_FT + (h.deep + HOLLOW_OVER_FT) * j as f32 / steps as f32)));
                    let list = grid.entry(at).or_default();
                    if list.last() != Some(&k) {
                        list.push(k);
                    }
                }
            }
            dots.retain(|d| {
                let Some(seg) = d.anchor.map(|a| self.anchors[a as usize].seg) else { return true };
                let c = cell(d.g.c);
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        for dz in -1..=1 {
                            let Some(list) = grid.get(&[c[0] + dx, c[1] + dy, c[2] + dz]) else { continue };
                            for h in list.iter().map(|k| &hollows[*k]).filter(|h| h.seg != seg) {
                                let v = sub(h.p, d.g.c);
                                let under = dot(v, h.n);
                                if under > -HOLLOW_OVER_FT && under < h.deep + 0.01 && len(sub(v, scale(h.n, under))) < h.r {
                                    return false;
                                }
                            }
                        }
                    }
                }
                true
            });
        }
        if look.effects.webbed && only.is_none() && look.held.is_none() {
            bind_threads(&mut dots, &self.anchors, puppet);
        }
        // webbed, bound: the held figure wrapped whole (`wraps`), once a build
        if let (Some(held), None) = (look.held, only) {
            let wrap = crate::wraps::Wrap { web: look.effects.webbed, rope: look.effects.bound };
            if wrap.web || wrap.rope {
                let skin = self.skin_points(held, &look.look, look.fitted);
                let seed = puppet.family.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
                dots.extend(crate::wraps::build(&skin, &wrap, &held.xf, &posed.xf, puppet.height, seed));
            }
        }
        // what is built by the way the figure is shown (`skin_field`):
        // bleeding, what its open wounds pour out, down it (`blood`); frozen,
        // icicles off it and ice at its feet, once a build (`frost`)
        let frosted = look.effects.frozen && only.is_none();
        let afflicted = (look.effects.poisoned || look.effects.diseased || look.effects.possessed) && only.is_none();
        // (a cut throat's well is its own: the others only when bleeding)
        let bleeding = (look.effects.bleeding || unturned.dead || look.effects.cutthroat) && !wells.is_empty();
        let shaded = look.effects.hidden && only.is_none();
        let blinded = look.effects.blinded && only.is_none();
        let sealed = look.effects.silenced && only.is_none();
        let fettered = look.effects.immobile && only.is_none();
        let exposed = look.effects.vulnerable && only.is_none();
        let raging = look.effects.frenzied && only.is_none();
        if bleeding || frosted || afflicted || shaded || blinded || sealed || fettered || exposed || raging {
            let shown = look.stands.or(look.held).unwrap_or(posed);
            let skin = self.skin_points(shown, &look.look, look.fitted);
            let rough: Vec<bool> = puppet.segments.iter().map(|s| s.part.as_deref().is_some_and(|q| q.ends_with("Hand"))).collect();
            let parent: Vec<Option<usize>> = puppet.segments.iter().map(|s| s.parent).collect();
            let trunk: Vec<bool> = puppet.segments.iter().map(|s| s.kind == crate::Kind::Trunk).collect();
            let parts = crate::skin_field::Parts { rough: &rough, parent: &parent, trunk: &trunk };
            if bleeding {
                let from = dots.len();
                dots.extend(crate::blood::build(&wells, &skin, &posed.xf, &shown.xf, &parts, spacing));
                // dead: blood that ran out of it a while since, as it lies --
                // darkened, tacky, not wet
                if unturned.dead {
                    for d in &mut dots[from..] {
                        d.g.rgb = wounds::mix(d.g.rgb, OLD_BLOOD, 0.5);
                        d.g.wet *= 0.3;
                    }
                }
            }
            if afflicted {
                // where poison got in: each open wound's middle, as shown
                let entries: Vec<(V3, V3, f32)> = wells
                    .iter()
                    .filter_map(|w| {
                        let inner: Vec<&crate::blood::Lip> = w.lips.iter().filter(|l| l.across < 1.0).collect();
                        let (from, into) = (posed.xf.get(w.seg)?, shown.xf.get(w.seg)?);
                        let c = scale(inner.iter().fold([0.0; 3], |c, l| add(c, l.p)), 1.0 / inner.len().max(1) as f32);
                        let n = norm_or(inner.iter().fold([0.0; 3], |c, l| add(c, l.n)), [0.0, 0.0, 1.0]);
                        Some((crate::skin_field::carry(from, into, c), crate::skin_field::turn(from, into, n), inner.len() as f32))
                    })
                    .collect();
                let what = crate::afflict::Affliction { poisoned: look.effects.poisoned, diseased: look.effects.diseased, possessed: look.effects.possessed, entries: &entries };
                let seed = puppet.family.bytes().fold(0xD15Eu32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
                let part_of = |seg: usize| puppet.segments.get(seg).and_then(|s| s.part.as_deref());
                dots.extend(crate::afflict::build(&what, &skin, &posed.xf, &shown.xf, &parts, &part_of, seed));
            }
            if frosted {
                let seed = puppet.family.bytes().fold(0x1CEu32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
                dots.extend(crate::frost::build(&skin, &posed.xf, &shown.xf, &parts, COAT_FT * 0.75, seed));
            }
            if shaded {
                let seed = puppet.family.bytes().fold(0x5AD0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
                dots.extend(crate::shade::pool(&skin, &posed.xf, &shown.xf, &parts, seed));
            }
            // possessed: its eyes burning (its veins are `afflict`'s)
            if look.effects.possessed && only.is_none() {
                dots.extend(crate::possess::eyes(posed));
            }
            // blinded: a band of dark mist over its eyes
            if look.effects.blinded && only.is_none() {
                let seed = puppet.family.bytes().fold(0xB11Du32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
                dots.extend(crate::blind::band(&skin, &posed.xf, &shown.xf, posed, seed));
            }
            // frenzied: its eyes aglow red (`heat`)
            if raging {
                dots.extend(crate::heat::eyes(posed));
            }
            // silenced: a seal of light across its mouth (`hush`)
            if sealed {
                let seed = puppet.family.bytes().fold(0x4054u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
                dots.extend(crate::hush::seal(&skin, &posed.xf, &shown.xf, posed, seed));
            }
            // immobile: bands of light round its ankles (`fetter`)
            if fettered {
                let seed = puppet.family.bytes().fold(0xFE77u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
                dots.extend(crate::fetter::bands(&skin, &posed.xf, &shown.xf, &parts, seed));
            }
            // vulnerable: a ring of red light on the ground (`exposed`)
            if exposed {
                let seed = puppet.family.bytes().fold(0xE705u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
                dots.extend(crate::exposed::ring(&skin, &posed.xf, &shown.xf, &parts, seed));
            }
        }
        hollows.truncate(own);
        (dots, particles, hollows)
    }
}

/// A fitted cloud's skin colour: the middle of its colours on the face and
/// the hands (`tone`, per anchor), of those that look like skin at all --
/// warm, not grey, not dark. None where too few do (a masked face, gloves).
fn skin_tone(puppet: &Puppet, anchors: &[Anchor], tone: &[[f32; 3]]) -> Option<[f32; 3]> {
    let seen: Vec<[f32; 3]> = anchors
        .iter()
        .zip(tone)
        .filter(|(a, _)| {
            let part = puppet.segments.get(a.seg).and_then(|s| s.part.as_deref()).unwrap_or("");
            (part == "head" && a.front > 0.55) || part.ends_with("Hand")
        })
        .map(|(_, t)| *t)
        .filter(|t| t[0] > t[1] && t[1] > t[2] && t[0] - t[2] > 15.0 && t[0] > 70.0)
        .collect();
    if seen.len() < 20 {
        return None;
    }
    let middle = |k: usize| {
        let mut v: Vec<f32> = seen.iter().map(|t| t[k]).collect();
        v.sort_by(f32::total_cmp);
        v[v.len() / 2]
    };
    Some([middle(0), middle(1), middle(2)])
}

/// How much of a fitted cloud is skin, per anchor, from its colours there
/// (`tone`), by how much they look like the face's and hands' skin
/// ([`skin_tone`]): on the head, neck and hands loosely in hue (lips,
/// a flushed cheek) but not much darker (hair); elsewhere closely, and
/// only on a part that is mostly that -- a bare arm, not a shirt the same
/// tan in patches. None where there is no telling (no skin tone found).
fn dead_skin(puppet: &Puppet, anchors: &[Anchor], tone: &[[f32; 3]]) -> Option<Vec<f32>> {
    let skin = skin_tone(puppet, anchors, tone)?;
    let part = |a: &Anchor| puppet.segments.get(a.seg).and_then(|s| s.part.as_deref()).unwrap_or("");
    let bare = |p: &str| matches!(p, "head" | "neck") || p.ends_with("Hand");
    let mut out: Vec<f32> = anchors
        .iter()
        .zip(tone)
        .map(|(a, t)| if bare(part(a)) { skin_like(*t, skin, 0.05, 0.1, 0.1, 0.25) } else { skin_like(*t, skin, 0.015, 0.035, 0.35, 0.8) })
        .collect();
    // a covered part keeps its colours whole
    let mut share = vec![(0.0f32, 0u32); puppet.segments.len()];
    for (a, w) in anchors.iter().zip(&out) {
        if let Some(s) = share.get_mut(a.seg) {
            *s = (s.0 + w, s.1 + 1);
        }
    }
    for (a, w) in anchors.iter().zip(out.iter_mut()) {
        let (sum, n) = share.get(a.seg).copied().unwrap_or((0.0, 1));
        if !bare(part(a)) && sum < 0.5 * n.max(1) as f32 {
            *w = 0.0;
        }
    }
    Some(out)
}

/// How much `rgb` looks like a skin of colour `tone`: 1 within `chroma.0`
/// of it in hue and saturation (rgb over its sum) and `light.0` in
/// lightness (log of the ratio: it is shaded), 0 past `.1` of either.
fn skin_like(rgb: [f32; 3], tone: [f32; 3], chroma_from: f32, chroma_to: f32, light_from: f32, light_to: f32) -> f32 {
    let (a, b) = (rgb.iter().sum::<f32>().max(1.0), tone.iter().sum::<f32>().max(1.0));
    let chroma = (0..3).map(|k| (rgb[k] / a - tone[k] / b).powi(2)).sum::<f32>().sqrt();
    let light = (a / b).ln().abs();
    (1.0 - smoothstep(chroma_from, chroma_to, chroma)) * (1.0 - smoothstep(light_from, light_to, light))
}

/// A webbed figure's threads (as `skin::bind_threads` draws them on the
/// smooth skin): a few short lines of film from a limb's skin to the
/// nearest skin of the body -- the trunk for an arm, the other leg for a
/// leg -- across the gap between them. Placed by a hash of the form, so the
/// web does not crawl from frame to frame.
fn bind_threads(dots: &mut Vec<Dot>, anchors: &[Anchor], puppet: &Puppet) {
    const MIN_FT: f32 = 0.05;
    const MAX_FT: f32 = 0.6;
    let part_of = |d: &Dot| d.anchor.and_then(|a| puppet.segments.get(anchors[a as usize].seg)).and_then(|s| s.part.as_deref());
    let skin: Vec<(&str, V3)> = dots.iter().filter(|d| d.layer == Layer::Skin).filter_map(|d| Some((part_of(d)?, d.g.c))).collect();
    let limb = |part: &str| part.contains("Arm") || part.contains("Hand") || part.contains("Leg");
    let partner = |from: &str, to: &str| {
        if from.contains("Leg") {
            to.contains("Leg") && from.starts_with("left") != to.starts_with("left")
        } else {
            matches!(to, "chest" | "abdomen" | "back")
        }
    };
    let limbs: Vec<usize> = (0..skin.len()).filter(|&k| limb(skin[k].0)).collect();
    if limbs.is_empty() {
        return;
    }
    let seed = puppet.family.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    let mut made = 0;
    for k in 0..crate::skin::BIND_THREAD_COUNT * 4 {
        if made >= crate::skin::BIND_THREAD_COUNT {
            break;
        }
        let pick = (crate::skin::web_hash(k as i32, seed ^ 0x7E1A) * limbs.len() as f32) as usize;
        let (from_part, a) = skin[limbs[pick.min(limbs.len() - 1)]];
        let Some((_, b)) = skin.iter().filter(|(p, _)| partner(from_part, p)).map(|(p, q)| (p, *q)).min_by(|x, y| len(sub(x.1, a)).total_cmp(&len(sub(y.1, a)))) else { continue };
        let span = len(sub(b, a));
        if !(MIN_FT..=MAX_FT).contains(&span) {
            continue;
        }
        let dir = scale(sub(b, a), 1.0 / span);
        let side = norm_or(cross(dir, [0.0, 1.0, 0.0]), [1.0, 0.0, 0.0]);
        let up = cross(side, dir);
        let step = 0.012f32;
        let n = (span / step).ceil() as usize;
        for j in 0..=n {
            let c = lerp3(a, b, j as f32 / n as f32);
            dots.push(Dot { layer: Layer::Web, anchor: None, source: None, g: Gaussian { c, axes: [scale(dir, step * 1.2), scale(side, WEB_HALF_FT), scale(up, WEB_HALF_FT)], n: [0.0; 3], rgb: crate::skin::WEB_RGB, alpha: 0.8, glow: 0.0, wet: 0.0, film: true, rim: 0.0 } });
        }
        made += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Pose, PoseSet, SHIPPED_POSES};

    fn humanoid() -> Puppet {
        Puppet::from_baked_toml(crate::fixtures::HUMANOID).expect("shipped humanoid")
    }

    fn count(dots: &[Dot], layer: Layer) -> usize {
        dots.iter().filter(|d| d.layer == layer).count()
    }

    fn wounded<'a>(wounds: &'a [(&'a str, u8)]) -> LayerLook<'a> {
        LayerLook { look: Look { wounds, ..Look::default() }, ..LayerLook::default() }
    }

    /// Unhurt and uncoated, the cloud is the skin alone, a dot per anchor --
    /// in any pose.
    #[test]
    fn an_unhurt_body_is_all_skin() {
        let p = humanoid();
        let cloud = LayerCloud::build(&p, &SplatParams::default());
        assert!(cloud.len() > 5000, "{}", cloud.len());
        let rest = cloud.dots(&p.pose(&Pose::default()), &LayerLook::default());
        assert_eq!(count(&rest, Layer::Skin), rest.len());
        assert_eq!(rest.len(), cloud.len());
        let set = PoseSet::from_toml(SHIPPED_POSES.iter().find(|(n, _)| *n == "biped").unwrap().1).unwrap();
        let kneel = cloud.dots(&p.pose(set.get("kneel").unwrap()), &LayerLook::default());
        assert_eq!(kneel.len(), rest.len());
        assert!(rest.iter().zip(&kneel).all(|(a, b)| a.anchor == b.anchor), "the anchors keep their order");
        assert!(rest.iter().zip(&kneel).any(|(a, b)| len(sub(a.g.c, b.g.c)) > 0.2), "and move with the pose");
    }

    /// Two hurt parts that overlap (a chest's rings run on inside the
    /// belly's): with the rest of the body's pits handed to it, a hit
    /// part's arrival frame is the whole build's hit part, dot for dot --
    /// nothing of the chest left standing in the belly's open wound.
    #[test]
    fn a_hit_part_is_cleared_out_of_its_neighbours_open_wound() {
        let p = humanoid();
        let cloud = LayerCloud::build(&p, &SplatParams::default());
        let posed = p.pose(&Pose::default());
        let still = wounded(&[("chest", 3), ("abdomen", 3)]);
        let fresh = |age: f32| LayerLook { fresh: Some(Fresh { part: "chest", age }), ..still };
        let only = cloud.fresh_segments(&posed, &fresh(0.0));
        let (_, around) = cloud.dots_and_hollows(&posed, &still);
        let seg_of = |d: &Dot| d.anchor.map(|a| cloud.anchors()[a as usize].seg);
        let key = |d: &Dot| (d.anchor, d.layer, d.g.c.map(f32::to_bits));
        let k = ARRIVAL_AGES.iter().position(|a| *a >= 1.0).unwrap();
        let mut whole: Vec<_> = cloud.dots(&posed, &fresh(ARRIVAL_AGES[k])).iter().filter(|d| d.layer != Layer::Loose && seg_of(d).is_some_and(|s| only[s])).map(key).collect();
        whole.sort();
        let frame = |around: &Hollows| {
            let mut f: Vec<_> = cloud.arrival(&posed, &fresh(0.0), &only, around).frames[k].iter().map(key).collect();
            f.sort();
            f
        };
        let blind = frame(&Hollows::default());
        assert!(blind.len() > whole.len(), "without the belly's pits the chest shows in its wound: {} vs {}", blind.len(), whole.len());
        assert_eq!(frame(&around), whole);
    }

    /// A hit's arrival built once: a frame per age, each the hit part as it
    /// is then with nothing thrown; and particles that, moved to an age,
    /// are the thrown dots the part has at that age.
    #[test]
    fn an_arrival_is_its_frames_and_what_it_throws() {
        let p = humanoid();
        let cloud = LayerCloud::build(&p, &SplatParams::default());
        let posed = p.pose(&Pose::default());
        let hurt = wounded(&[("chest", 3)]);
        let look = |age: f32| LayerLook { causes: &[("chest", Cause::Fire)], fresh: Some(Fresh { part: "chest", age }), ..hurt };
        let only = cloud.fresh_segments(&posed, &look(0.0));
        let arrival = cloud.arrival(&posed, &look(0.0), &only, &Hollows::default());
        assert_eq!(arrival.frames.len(), ARRIVAL_AGES.len());
        for (frame, age) in arrival.frames.iter().zip(ARRIVAL_AGES) {
            let at: Vec<Dot> = cloud.dots_in(&posed, &look(age), &only).into_iter().filter(|d| d.layer != Layer::Loose).collect();
            assert_eq!(frame.len(), at.len(), "the frame at {age} s");
            assert!(frame.iter().all(|d| d.layer != Layer::Loose));
        }
        assert!(!arrival.particles.is_empty());
        // moved to an age once the wound is open, they are its thrown dots
        for age in [0.5f32, 1.5, 3.5] {
            let thrown: Vec<Gaussian> = cloud.dots_in(&posed, &look(age), &only).into_iter().filter(|d| d.layer == Layer::Loose).map(|d| d.g).collect();
            let moved: Vec<Gaussian> = arrival.particles.iter().filter_map(|q| q.at(age)).collect();
            assert_eq!(thrown.len(), moved.len(), "{age} s");
            assert!(thrown.iter().zip(&moved).all(|(a, b)| a.c == b.c && a.rgb == b.rgb && a.alpha == b.alpha), "{age} s");
        }
    }

    /// A hit arriving changes only its part: the rest built once, and the
    /// hit part built alone, are the whole -- dot for dot once the wound is
    /// open.
    #[test]
    fn a_hit_part_rebuilt_alone_completes_the_rest() {
        let p = humanoid();
        let cloud = LayerCloud::build(&p, &SplatParams::default());
        let posed = p.pose(&Pose::default());
        let still = wounded(&[("chest", 2), ("leftArm", 1)]);
        let fresh = LayerLook { fresh: Some(Fresh { part: "chest", age: 1.0 }), ..still };
        let mask = cloud.fresh_segments(&posed, &fresh);
        assert!(mask.iter().any(|m| *m) && mask.iter().enumerate().all(|(i, m)| !*m || p.segments[i].part.as_deref() == Some("chest")), "the chest's segments");
        assert!(cloud.fresh_segments(&posed, &still).iter().all(|m| !*m), "no hit: none");
        let seg_of = |d: &Dot| d.anchor.map(|a| cloud.anchors()[a as usize].seg);
        let whole = cloud.dots(&posed, &fresh);
        let rest: Vec<Dot> = cloud.dots(&posed, &still).into_iter().filter(|d| seg_of(d).is_none_or(|s| !mask[s])).collect();
        let hit = cloud.dots_in(&posed, &fresh, &mask);
        assert!(hit.iter().all(|d| seg_of(d).is_none_or(|s| mask[s])), "the hit part's dots alone");
        assert!(hit.iter().any(|d| d.layer == Layer::Loose), "with what the hit throws off");
        let key = |d: &Dot| (d.anchor, d.layer, d.g.c.map(f32::to_bits));
        let mut a: Vec<_> = whole.iter().map(key).collect();
        let mut b: Vec<_> = rest.iter().chain(&hit).map(key).collect();
        a.sort();
        b.sort();
        assert_eq!(a, b);
    }

    /// Pinned, a held figure's wrists lie against the sides of its hips,
    /// nearer than the pose file alone puts them.
    #[test]
    fn a_held_figure_has_its_arms_pinned() {
        let p = humanoid();
        let set = PoseSet::from_toml(SHIPPED_POSES.iter().find(|(n, _)| *n == "biped").unwrap().1).unwrap();
        let webbed = set.get("webbed").unwrap();
        let pinned = p.pin_arms(webbed);
        let wrists = |pose: &Pose| {
            let posed = p.pose(pose);
            ["arm_l", "arm_r"].map(|side| {
                let last = (0..p.segments.len()).filter(|&i| p.segments[i].bone == side).last().unwrap();
                posed.xf[last].point(p.segments[last].tail)
            })
        };
        let (before, after) = (wrists(webbed), wrists(&pinned));
        for k in 0..2 {
            assert!(after[k][0].abs() <= before[k][0].abs() + 0.02, "no further out: {:?} -> {:?}", before[k], after[k]);
            assert!(after[k][0].abs() < 0.9, "by the hips: {:?}", after[k]);
            assert!(after[k][2].abs() < 0.35, "not held out in front: {:?}", after[k]);
        }
        assert!(pinned.bone("arm_l").bend.iter().all(|b| *b == 0.0), "elbows straight");
    }

    /// Webbed and bound hold the figure tight (arms pinned, legs together)
    /// and wrap it whole in that pose: coils of rope round both legs at
    /// once and round the trunk and pinned arms, webs over it all -- in
    /// place of each part's own loops and strands.
    #[test]
    fn a_held_figure_is_wrapped_whole() {
        let p = humanoid();
        let cloud = LayerCloud::build(&p, &SplatParams { spacing: 0.015, ..SplatParams::default() });
        let set = PoseSet::from_toml(SHIPPED_POSES.iter().find(|(n, _)| *n == "biped").unwrap().1).unwrap();
        let held = p.pose(set.get("webbed").unwrap());
        let wrapped = |effects: crate::skin::Effects| cloud.dots(&held, &LayerLook { effects, held: Some(&held), ..LayerLook::default() });
        let rope = wrapped(crate::skin::Effects { bound: true, ..Default::default() });
        let coils: Vec<&Dot> = rope.iter().filter(|d| d.layer == Layer::Wrap).collect();
        assert!(coils.len() > 5000, "{} rope dots", coils.len());
        assert!(rope.iter().all(|d| d.layer != Layer::Status), "no loops part by part");
        // a coil at the knees goes round both legs: across the gap between them
        let knee = p.height * 0.27;
        let between = coils.iter().filter(|d| (d.g.c[1] - knee).abs() < 0.1 && d.g.c[0].abs() < 0.04).count();
        assert!(between > 20, "{between} rope dots between the knees");
        let web = wrapped(crate::skin::Effects { webbed: true, ..Default::default() });
        let silk = web.iter().filter(|d| d.layer == Layer::Wrap).count();
        assert!(silk > 20000, "{silk} silk dots");
        assert!(web.iter().all(|d| d.layer != Layer::Web), "no strands part by part");
        // each rides a part: on an anchor of the skin it lies nearest
        assert!(rope.iter().chain(&web).filter(|d| d.layer == Layer::Wrap).all(|d| d.anchor.is_some()));
        // not held, no wrap
        let loose = cloud.dots(&held, &LayerLook { effects: crate::skin::Effects { bound: true, ..Default::default() }, ..LayerLook::default() });
        assert!(loose.iter().all(|d| d.layer != Layer::Wrap));
    }

    /// What each status builds on the body: dots of its own, on anchors
    /// of the parts it belongs to, standing off the skin -- vines and roots
    /// on the legs and the ground, ropes on the trunk, arms and legs,
    /// boils, blisters, a veil, wisps. (Blood is its own layer: below.)
    #[test]
    fn statuses_build_on_the_body() {
        let p = humanoid();
        let cloud = LayerCloud::build(&p, &SplatParams { spacing: 0.015, ..SplatParams::default() });
        let posed = p.pose_as_drawn(&Pose::default());
        let part_of = |d: &Dot| d.anchor.and_then(|a| p.segments[cloud.anchors()[a as usize].seg].part.as_deref());
        let with = |effects: crate::skin::Effects, wounds: &'static [(&'static str, u8)]| {
            let dots = cloud.dots(&posed, &LayerLook { effects, ..wounded(wounds) });
            dots.into_iter().filter(|d| d.layer == Layer::Status).collect::<Vec<Dot>>()
        };
        let none = with(crate::skin::Effects::default(), &[]);
        assert!(none.is_empty());
        let rooted = with(crate::skin::Effects { rooted: true, ..Default::default() }, &[]);
        assert!(rooted.len() > 3000, "{} vine and root dots", rooted.len());
        assert!(rooted.iter().all(|d| part_of(d).is_some_and(|q| q.contains("Leg"))), "on the legs");
        assert!(rooted.iter().all(|d| d.g.c[1] < p.height * 0.55), "below mid-thigh");
        assert!(rooted.iter().any(|d| d.g.c[1] < 0.03), "and roots on the ground");
        let bound = with(crate::skin::Effects { bound: true, ..Default::default() }, &[]);
        assert!(bound.len() > 3000, "{} rope dots", bound.len());
        assert!(bound.iter().all(|d| part_of(d).is_some_and(|q| matches!(q, "chest" | "abdomen") || q.ends_with("Arm") || q.ends_with("Leg"))));
        assert!(bound.iter().any(|d| part_of(d).is_some_and(|q| q.ends_with("Leg"))), "the legs bound too");
        assert!(bound.iter().all(|d| d.g.glow > 0.1), "ropes of light");
        assert!(with(crate::skin::Effects { bleeding: true, ..Default::default() }, &[("chest", 3)]).is_empty(), "blood is not a status layer");
        // poison's veins and disease's boils cross parts: built round the
        // whole figure (`afflict`), from a bite or from the wounds
        let built = |effects: crate::skin::Effects, wounds: &'static [(&'static str, u8)]| cloud.dots(&posed, &LayerLook { effects, ..wounded(wounds) }).into_iter().filter(|d| d.layer == Layer::Wrap).collect::<Vec<Dot>>();
        let poisoned = crate::skin::Effects { poisoned: true, ..Default::default() };
        let bitten = built(poisoned, &[]);
        assert!(bitten.len() > 1500 && bitten.iter().any(|d| d.g.glow > 0.3), "{} vein and blister dots, aglow", bitten.len());
        let through_wounds = built(poisoned, &[("chest", 3)]);
        assert!(through_wounds.len() > 1500 && through_wounds.iter().map(|d| (d.g.c[1] * 100.0) as i32).collect::<std::collections::HashSet<_>>() != bitten.iter().map(|d| (d.g.c[1] * 100.0) as i32).collect(), "from the wound, not the bite");
        let sick = built(crate::skin::Effects { diseased: true, ..Default::default() }, &[]);
        assert!(sick.len() > 1500 && sick.iter().all(|d| d.anchor.is_some()), "{} boil, sore and blotch dots", sick.len());
        assert!(with(crate::skin::Effects { poisoned: true, diseased: true, ..Default::default() }, &[]).is_empty(), "nothing per part");
        // hidden: no veil round its parts, but its skin sunk in shadow (most
        // at its feet), a pool of shade round its feet, wisps rising
        let hidden = LayerLook { effects: crate::skin::Effects { hidden: true, ..Default::default() }, ..LayerLook::default() };
        assert!(with(hidden.effects, &[]).is_empty(), "no veil");
        let shaded = cloud.dots(&posed, &hidden);
        let plain_skin = cloud.dots(&posed, &LayerLook::default());
        let bright = |ds: &[Dot]| ds.iter().filter(|d| d.layer == Layer::Skin).map(|d| d.g.rgb.iter().sum::<f32>()).sum::<f32>() / ds.len().max(1) as f32;
        assert!(bright(&shaded) < bright(&plain_skin) * 0.5, "its skin in shadow");
        let built: Vec<&Dot> = shaded.iter().filter(|d| d.layer == Layer::Wrap).collect();
        assert!(built.iter().all(|d| d.g.film), "all shadow is film");
        let pool = built.iter().filter(|d| d.g.c[1] < 0.02 && d.g.axes[1][1].abs() < 0.01).count();
        assert!(pool > 200, "{pool} pool films, flat on the ground");
        let clinging = built.len() - pool;
        assert!(clinging > 1000 && built.iter().any(|d| d.g.c[1] > p.height * 0.8), "{clinging} films clinging round it, up to its head");
        // none over an open wound: its flesh stays red
        let cut = cloud.dots(&posed, &LayerLook { effects: hidden.effects, ..wounded(&[("chest", 3)]) });
        let flesh: Vec<V3> = cut.iter().filter(|d| d.layer == Layer::Flesh).map(|d| d.g.c).collect();
        assert!(!flesh.is_empty());
        let over = cut.iter().filter(|d| d.layer == Layer::Wrap && d.g.c[1] > 0.1).filter(|d| flesh.iter().any(|f| len(sub(*f, d.g.c)) < 0.05)).count();
        assert_eq!(over, 0, "no shadow over the wound");
        let wisps = cloud.motes(&posed, &hidden);
        assert!(wisps.len() > 60 && wisps.iter().all(|w| w.loops && w.glow == ParticleGlow::Shadow), "{} wisps", wisps.len());
        let wisp = (0..20).find_map(|k| wisps[0].at(k as f32 * 0.2));
        assert!(wisp.is_some_and(|g| g.film && g.glow == 0.0), "a wisp is a dark film");
        // possessed: no pall round its parts; glowing veins from its heart
        // onto its neck and face, its eyes burning, smoke and sparks rising
        let possessed = LayerLook { effects: crate::skin::Effects { possessed: true, ..Default::default() }, ..LayerLook::default() };
        assert!(with(possessed.effects, &[]).is_empty(), "no pall");
        let taken = cloud.dots(&posed, &possessed);
        let veins: Vec<&Dot> = taken.iter().filter(|d| d.layer == Layer::Wrap && d.anchor.is_some()).collect();
        assert!(veins.len() > 1000 && veins.iter().any(|d| d.g.glow > 0.3), "{} vein dots, aglow", veins.len());
        assert!(veins.iter().any(|d| part_of(d) == Some("neck")) && veins.iter().any(|d| part_of(d).is_some_and(|q| q.ends_with("Arm"))), "up the neck and down the arms");
        let eyes = taken.iter().filter(|d| d.layer == Layer::Wrap && d.anchor.is_none() && d.g.glow >= 1.0).count();
        assert_eq!(eyes, 2 * p.eyes.len(), "its eyes burning");
        let smoke = cloud.motes(&posed, &possessed);
        assert!(smoke.iter().any(|w| w.glow == ParticleGlow::Shadow) && smoke.iter().any(|w| w.glow == ParticleGlow::Spark));
        assert!(smoke.iter().all(|w| w.from[1] > p.height * 0.7), "off its head and shoulders");
        // blinded: a band of dark mist on the head at its eyes, mist curling off
        let blind = LayerLook { effects: crate::skin::Effects { blinded: true, ..Default::default() }, ..LayerLook::default() };
        let band: Vec<Dot> = cloud.dots(&posed, &blind).into_iter().filter(|d| d.layer == Layer::Wrap).collect();
        let eye_y = p.eyes.iter().map(|e| e.pos[1]).sum::<f32>() / p.eyes.len().max(1) as f32;
        assert!(band.len() > 30 && band.iter().all(|d| part_of(d) == Some("head") && (d.g.c[1] - eye_y).abs() < 0.12), "{} band dots, over the eyes", band.len());
        assert!(band.iter().any(|d| d.g.c[2] > 0.1), "across the face");
        assert!(cloud.motes(&posed, &blind).iter().all(|w| w.glow == ParticleGlow::Shadow && w.loops));
        // invisible: the skin a pale film seen only edge-on, glints rising off it
        let invisible = LayerLook { effects: crate::skin::Effects { invisible: true, ..Default::default() }, ..LayerLook::default() };
        let ghost = cloud.dots(&posed, &invisible);
        assert!(ghost.iter().filter(|d| d.layer == Layer::Skin).all(|d| d.g.rim > 0.9));
        let glints = cloud.motes(&posed, &invisible);
        assert!(glints.len() > 80 && glints.iter().all(|g| g.loops && g.rises.is_some()), "{} glints, on loops", glints.len());
        assert!(glints.iter().any(|g| g.from[1] > p.height * 0.75) && glints.iter().any(|g| g.from[1] < p.height * 0.3), "off the whole figure");
        assert!(cloud.motes(&posed, &LayerLook::default()).is_empty());
        // one on a loop is where it was a loop later, and in and out between
        let g = &glints[0];
        let life = g.rises.unwrap_or(1.0);
        let at = |age: f32| g.at(age).map(|d| (d.c, d.alpha));
        assert_eq!(at(0.4).map(|(c, _)| c.map(|v| (v * 1e3).round())), at(0.4 + life).map(|(c, _)| c.map(|v| (v * 1e3).round())));
        assert!((0..40).any(|k| at(k as f32 * life / 40.0).is_none()) && (0..40).any(|k| at(k as f32 * life / 40.0).is_some()));
    }

    /// A cut throat: a slash opened across the front of the neck, to its
    /// flesh, and bleeding down the figure though nothing else bleeds.
    #[test]
    fn a_cut_throat_opens_and_bleeds() {
        let p = humanoid();
        let cloud = LayerCloud::build(&p, &SplatParams { spacing: 0.015, ..SplatParams::default() });
        let posed = p.pose_as_drawn(&Pose::default());
        let seg_of = |d: &Dot| d.anchor.map(|a| cloud.anchors()[a as usize].seg);
        let neck = p.segments.iter().position(|s| s.part.as_deref() == Some("neck")).expect("a neck");
        let cut = cloud.dots(&posed, &LayerLook { effects: crate::skin::Effects { cutthroat: true, ..Default::default() }, ..LayerLook::default() });
        let flesh: Vec<&Dot> = cut.iter().filter(|d| d.layer == Layer::Flesh).collect();
        assert!(!flesh.is_empty() && flesh.iter().all(|d| seg_of(d) == Some(neck)), "{} flesh dots, in the neck", flesh.len());
        let (lo, hi) = flesh.iter().fold((f32::MAX, f32::MIN), |(lo, hi), d| (lo.min(d.g.c[0]), hi.max(d.g.c[0])));
        assert!(hi - lo > 0.2, "across the throat: {:.2} ft wide", hi - lo);
        assert!(flesh.iter().all(|d| d.g.c[2] > 0.0), "on its front");
        let blood = cut.iter().filter(|d| matches!(d.layer, Layer::Blood { .. })).count();
        assert!(blood > 1000, "{blood} blood dots");
        assert!(cloud.dots(&posed, &LayerLook::default()).iter().all(|d| d.layer == Layer::Skin));
    }

    /// Dead, its open wounds have bled out of it as it lies: the blood is
    /// there without bleeding, darkened and drying; alive and not bleeding,
    /// none.
    #[test]
    fn a_dead_figure_lies_in_old_blood() {
        let p = humanoid();
        let cloud = LayerCloud::build(&p, &SplatParams { spacing: 0.015, ..SplatParams::default() });
        let posed = p.pose_as_drawn(&Pose::default());
        let blood = |dead: bool| {
            let look = LayerLook { look: Look { dead, ..wounded(&[("chest", 3)]).look }, ..LayerLook::default() };
            cloud.dots(&posed, &look).into_iter().filter(|d| matches!(d.layer, Layer::Blood { .. })).collect::<Vec<Dot>>()
        };
        assert!(blood(false).is_empty());
        let old = blood(true);
        assert!(old.len() > 1000, "{} blood dots", old.len());
        assert!(old.iter().all(|d| d.g.wet <= 0.31 && d.g.rgb[0] < 140.0), "dark, and not wet");
    }

    /// Bleeding: each open wound wells up and runs down the figure as it
    /// stands, off its own part onto the ones below, to a pool on the
    /// ground -- and comes with a hit as it arrives: welling, running,
    /// then pooling.
    #[test]
    fn a_bleeding_wound_runs_down_the_figure() {
        use crate::math::{mul_mv, transpose};
        let p = humanoid();
        let cloud = LayerCloud::build(&p, &SplatParams { spacing: 0.015, ..SplatParams::default() });
        let posed = p.pose(&Pose::default());
        let blood_of = |posed: &Posed<'_>, look: &LayerLook| -> Vec<Dot> { cloud.dots(posed, look).into_iter().filter(|d| matches!(d.layer, Layer::Blood { .. })).collect() };
        let blood = |look: &LayerLook| blood_of(&posed, look);
        let bleeding = crate::skin::Effects { bleeding: true, ..Default::default() };
        let still = blood(&LayerLook { effects: bleeding, ..wounded(&[("chest", 3)]) });
        assert!(still.len() > 2000, "{} blood dots", still.len());
        // each from the chest's wound (the chest is a segment or two), on a part
        let from_chest = |d: &Dot| matches!(d.layer, Layer::Blood { from } if p.segments[from as usize].part.as_deref() == Some("chest"));
        assert!(still.iter().all(|d| from_chest(d) && d.anchor.is_some()));
        assert!(still.iter().filter(|d| d.g.wet >= 1.0).count() > 2000, "wet runs");
        // off the chest onto the parts below it, and down to the ground
        let part_of = |d: &Dot| p.segments[cloud.anchors()[d.anchor.unwrap() as usize].seg].part.as_deref();
        assert!(still.iter().any(|d| part_of(d).is_some_and(|q| q != "chest")), "it runs onto other parts");
        assert!(still.iter().any(|d| d.g.c[1] < 0.03), "a pool on the ground");
        // nothing without an open wound, or not bleeding, or from a crush
        assert!(blood(&LayerLook { effects: bleeding, ..wounded(&[]) }).is_empty());
        assert!(blood(&wounded(&[("chest", 3)])).is_empty());
        assert!(blood(&LayerLook { effects: bleeding, causes: &[("chest", Cause::Crush)], ..wounded(&[("chest", 3)]) }).is_empty());
        // as a hit arrives it wells up, runs and pools
        let at = |age: f32| blood(&LayerLook { effects: bleeding, fresh: Some(Fresh { part: "chest", age }), ..wounded(&[("chest", 3)]) }).len();
        let (none, early, later, done) = (at(0.0), at(0.5), at(1.4), at(FRESH_SECS));
        assert!(none == 0 && early > 0 && early < later && later < done, "{none} {early} {later} {done}");
        assert_eq!(done, still.len());
        // built as drawn and shown standing, it ran as it stands: carried
        // back into the standing pose part by part, it reaches the ground
        let drawn = p.pose_as_drawn(&Pose::default());
        let carried: Vec<V3> = blood_of(&drawn, &LayerLook { effects: bleeding, stands: Some(&posed), ..wounded(&[("chest", 3)]) })
            .iter()
            .map(|d| {
                let seg = cloud.anchors()[d.anchor.unwrap() as usize].seg;
                let (from, into) = (&drawn.xf[seg], &posed.xf[seg]);
                into.point(mul_mv(&transpose(&from.r), sub(d.g.c, from.t)))
            })
            .collect();
        assert!(carried.len() > 2000 && carried.iter().any(|c| c[1] < 0.03));
    }

    /// Webbed: strands of film over the skin, threads from limb to body,
    /// and the skin under them as it was.
    #[test]
    fn a_webbed_body_wears_strands_and_threads_of_film() {
        let p = humanoid();
        let cloud = LayerCloud::build(&p, &SplatParams { spacing: 0.015, ..SplatParams::default() });
        let posed = p.pose(&Pose::default());
        let whole = cloud.dots(&posed, &LayerLook::default());
        let effects = crate::skin::Effects { webbed: true, ..Default::default() };
        let webbed = cloud.dots(&posed, &LayerLook { effects, ..LayerLook::default() });
        let web: Vec<&Dot> = webbed.iter().filter(|d| d.layer == Layer::Web).collect();
        assert!(web.len() > 1000, "{} strand dots", web.len());
        assert!(web.iter().all(|d| d.g.film), "a web is a film");
        assert!(web.iter().any(|d| d.anchor.is_none()), "threads, off any anchor");
        assert_eq!(count(&webbed, Layer::Skin), count(&whole, Layer::Skin), "the skin under it is all there");
        assert_eq!(count(&whole, Layer::Web), 0);
    }

    /// Frozen: ice head to foot, two films deep, over skin that shines
    /// under it -- and icicles, frost needles and its feet locked in ice.
    #[test]
    fn a_frozen_body_is_iced_over_in_film() {
        let p = humanoid();
        let cloud = LayerCloud::build(&p, &SplatParams::default());
        let posed = p.pose(&Pose::default());
        let effects = crate::skin::Effects { frozen: true, ..Default::default() };
        let frozen = cloud.dots(&posed, &LayerLook { effects, ..LayerLook::default() });
        let ice: Vec<&Dot> = frozen.iter().filter(|d| d.layer == Layer::Coat).collect();
        // as much of it as a full coat of anything: all the skin that shows
        let stone = cloud.dots(&posed, &LayerLook { coat: Some(Coating::new(Material::Stone, 1.0)), ..LayerLook::default() });
        assert_eq!(ice.len(), 2 * count(&stone, Layer::Coat), "head to foot, two deep");
        assert!(ice.iter().all(|d| d.g.film), "ice is a film");
        assert!(stone.iter().filter(|d| d.layer == Layer::Coat).all(|d| !d.g.film), "stone is not");
        assert!(frozen.iter().filter(|d| d.layer == Layer::Skin).all(|d| d.g.wet >= ICE_SHINE), "under ice it shines");
        // built round it as it stands: icicles and frost on it, ice at its feet
        let cold: Vec<&Dot> = frozen.iter().filter(|d| d.layer == Layer::Wrap).collect();
        assert!(cold.len() > 3000 && cold.iter().all(|d| d.anchor.is_some()), "{} icicle, frost and block dots", cold.len());
        assert!(cold.iter().filter(|d| d.g.c[1] < 0.3 && d.g.film).count() > 200, "the feet locked in ice");
        assert!(cold.iter().any(|d| !d.g.film && d.g.c[1] > 1.0), "frost needles");
        // none while a hit's part is rebuilt alone: the rest keeps it
        let only: Vec<bool> = p.segments.iter().map(|s| s.part.as_deref() == Some("chest")).collect();
        assert!(cloud.dots_in(&posed, &LayerLook { effects, ..LayerLook::default() }, &only).iter().all(|d| d.layer != Layer::Wrap));
    }

    /// The marks sheet's wetness reaches the dots: whole, nothing is wet;
    /// an open wound's pit is wet, and so is the fresh blood round it --
    /// what the GPU cloud shines a lamp off (`cloud_gpu.wgsl`).
    #[test]
    fn a_wound_and_its_blood_are_wet() {
        let p = humanoid();
        let cloud = LayerCloud::build(&p, &SplatParams::default());
        let posed = p.pose(&Pose::default());
        let whole = cloud.dots(&posed, &LayerLook::default());
        assert!(whole.iter().all(|d| d.g.wet == 0.0), "unhurt: dry");
        let torn = cloud.dots(&posed, &wounded(&[("chest", 3)]));
        assert!(torn.iter().filter(|d| d.layer != Layer::Skin).all(|d| d.g.wet >= 0.5), "an open wound is wet inside");
        assert!(torn.iter().any(|d| d.layer == Layer::Skin && d.g.wet > 0.0), "and the blood on the skin round it");
    }

    /// Rank 1 scratches the skin; a rank-2 wound's gashes go through the fat
    /// to the flesh, a rank-3 one's to the bone -- pits sunk under where the
    /// skin was, on that part alone, the same from frame to frame.
    #[test]
    fn a_wound_opens_to_flesh_then_bone() {
        let p = humanoid();
        let cloud = LayerCloud::build(&p, &SplatParams::default());
        let posed = p.pose(&Pose::default());
        let whole = cloud.dots(&posed, &LayerLook::default());
        let light = cloud.dots(&posed, &wounded(&[("chest", 1)]));
        assert_eq!(count(&light, Layer::Skin), light.len(), "rank 1 is on the skin");
        assert!(light.iter().zip(&whole).any(|(a, b)| a.g.rgb != b.g.rgb), "but it marks it");
        let gashed = cloud.dots(&posed, &wounded(&[("chest", 2)]));
        assert!(count(&gashed, Layer::Fat) > 10 && count(&gashed, Layer::Flesh) > 10 && count(&gashed, Layer::Bone) == 0, "{} flesh", count(&gashed, Layer::Flesh));
        let torn = cloud.dots(&posed, &wounded(&[("chest", 3)]));
        assert!(count(&torn, Layer::Bone) > 10 && count(&torn, Layer::Flesh) > count(&gashed, Layer::Flesh));
        let skin_at: std::collections::HashMap<u32, &Dot> = whole.iter().map(|d| (d.anchor.unwrap(), d)).collect();
        for d in torn.iter().filter(|d| d.layer != Layer::Skin) {
            let skin = skin_at[&d.anchor.unwrap()];
            let under = dot(sub(skin.g.c, d.g.c), skin.g.n);
            assert!(under > 0.0, "{:?} is {under} ft under", d.layer);
            assert_eq!(p.segments[cloud.anchors()[d.anchor.unwrap() as usize].seg].part.as_deref(), Some("chest"));
        }
        // and no other part's skin shows in a pit: every dot left on the chest's
        // front between a pit's rim and its floor is the pit's own
        assert!(torn.len() < whole.len() + count(&torn, Layer::Fat) + count(&torn, Layer::Flesh) + count(&torn, Layer::Bone) + whole.len() / 4);
        // a pit's walls face inward: not every dot in it faces as the skin did
        assert!(torn.iter().filter(|d| d.layer != Layer::Skin).any(|d| dot(d.g.n, skin_at[&d.anchor.unwrap()].g.n) < 0.9));
        let again = cloud.dots(&posed, &wounded(&[("chest", 3)]));
        assert!(torn.len() == again.len() && torn.iter().zip(&again).all(|(a, b)| a.g.c == b.g.c));
    }

    /// What made a wound shapes it: a crush dents without opening, cold
    /// leaves a crust, a puncture bores in, fire chars and its cracks glow.
    #[test]
    fn a_cause_shapes_its_wound() {
        let p = humanoid();
        let cloud = LayerCloud::build(&p, &SplatParams::default());
        let posed = p.pose(&Pose::default());
        let whole = cloud.dots(&posed, &LayerLook::default());
        let by = |cause: Cause| cloud.dots(&posed, &LayerLook { causes: &[("chest", cause)], ..wounded(&[("chest", 3)]) });
        let opened = |dots: &[Dot]| count(dots, Layer::Fat) + count(dots, Layer::Flesh) + count(dots, Layer::Bone);
        assert!(opened(&by(Cause::Puncture)) > 10);
        let crushed = by(Cause::Crush);
        assert_eq!(opened(&crushed), 0, "a crush does not open the skin");
        let skin_at: std::collections::HashMap<u32, &Dot> = whole.iter().map(|d| (d.anchor.unwrap(), d)).collect();
        assert!(crushed.iter().any(|d| dot(sub(skin_at[&d.anchor.unwrap()].g.c, d.g.c), skin_at[&d.anchor.unwrap()].g.n) > 0.01), "it dents it");
        let frozen = by(Cause::Cold);
        assert!(opened(&frozen) == 0 && count(&frozen, Layer::Coat) > 10, "{} frost dots", count(&frozen, Layer::Coat));
        let burnt = by(Cause::Fire);
        let brightest = burnt.iter().map(|d| d.g.glow).fold(0.0, f32::max);
        assert!(opened(&burnt) == 0 && brightest > 0.05 && brightest < 0.5, "old embers glow a little: {brightest}");
        assert!(opened(&by(Cause::Lightning)) == 0);
        assert!(whole.iter().all(|d| d.g.glow == 0.0));
        assert_eq!(Cause::from_name("Fire"), Some(Cause::Fire));
    }

    /// Every damage type the game has marks the body its own way, by the
    /// game's name for it; a worse critical is a bigger wound; and the
    /// worst of a chest wound comes out of the back behind where it went in.
    #[test]
    fn the_games_damage_types() {
        let p = humanoid();
        let cloud = LayerCloud::build(&p, &SplatParams::default());
        let posed = p.pose(&Pose::default());
        let whole = cloud.dots(&posed, &LayerLook::default());
        let colours = |dots: &[Dot]| dots.iter().map(|d| d.g.rgb.map(|c| c as i32)).collect::<Vec<_>>();
        let mut seen = Vec::new();
        for cause in Cause::ALL {
            assert_eq!(Cause::from_name(cause.name()), Some(cause));
            let dots = cloud.dots(&posed, &LayerLook { causes: &[("chest", cause)], ..wounded(&[("chest", 3)]) });
            let look = colours(&dots);
            assert!(look != colours(&whole), "{} marks the chest", cause.name());
            assert!(!seen.contains(&look), "{} looks like no other", cause.name());
            seen.push(look);
            // and something leaves the body when it lands, a hold and a fall aside
            let hit = cloud.dots(&posed, &LayerLook { causes: &[("chest", cause)], fresh: Some(Fresh { part: "chest", age: 0.3 }), ..wounded(&[("chest", 3)]) });
            assert_eq!(count(&hit, Layer::Loose) > 0, !matches!(cause, Cause::Grapple | Cause::Unbalance), "{}", cause.name());
        }
        assert_eq!(Cause::from_name("ucs_kick"), Some(Cause::Kick));
        assert_eq!(Cause::from_name("Non-Corporeal"), Some(Cause::NonCorporeal));
        let opened = |crit: u8| {
            let dots = cloud.dots(&posed, &LayerLook { crits: &[("chest", crit)], ..wounded(&[("chest", 3)]) });
            count(&dots, Layer::Fat) + count(&dots, Layer::Flesh) + count(&dots, Layer::Bone)
        };
        assert!(opened(9) > opened(7), "{} vs {}", opened(9), opened(7));
        // through the body: pits on the back's side of the chest too
        let through = cloud.dots(&posed, &wounded(&[("chest", 3), ("back", 2)]));
        let rear = through.iter().filter(|d| d.layer == Layer::Flesh && d.anchor.is_some_and(|a| cloud.anchors()[a as usize].front < -0.35)).count();
        assert!(rear > 10, "{rear} flesh dots on the back");
    }

    /// A hit arrives: the wound opens in a flash and throws dots off, which
    /// fall and lie on the ground; once stale, nothing is loose.
    #[test]
    fn a_hit_arrives() {
        let p = humanoid();
        let cloud = LayerCloud::build(&p, &SplatParams::default());
        let posed = p.pose(&Pose::default());
        let at = |cause: Cause, age: Option<f32>| {
            cloud.dots(&posed, &LayerLook { causes: &[("chest", cause)], fresh: age.map(|age| Fresh { part: "chest", age }), ..wounded(&[("chest", 2)]) })
        };
        let settled = at(Cause::Slash, None);
        assert_eq!(count(&settled, Layer::Loose), 0);
        let just = at(Cause::Slash, Some(0.03));
        let opened = |dots: &[Dot]| count(dots, Layer::Fat) + count(dots, Layer::Flesh);
        assert!(opened(&just) < opened(&settled), "it is still opening: {} of {}", opened(&just), opened(&settled));
        assert!(just.iter().any(|d| d.g.glow > 0.5), "in a flash");
        let flying: Vec<&Dot> = just.iter().filter(|d| d.layer == Layer::Loose).collect();
        assert!(flying.len() > 20 && flying.iter().all(|d| d.g.c[1] > 1.0), "{} loose dots, off the chest", flying.len());
        let later = at(Cause::Slash, Some(2.0));
        assert!(later.iter().all(|d| d.g.glow < 0.01), "the flash is over");
        let landed: Vec<&Dot> = later.iter().filter(|d| d.layer == Layer::Loose).collect();
        assert!(landed.len() > 20 && landed.iter().all(|d| d.g.c[1] < 0.05), "and they lie on the ground");
        assert_eq!(count(&at(Cause::Slash, Some(FRESH_SECS)), Layer::Loose), 0);
        // only the part that was hit
        let other = cloud.dots(&posed, &LayerLook { fresh: Some(Fresh { part: "head", age: 0.03 }), ..wounded(&[("chest", 2)]) });
        assert!(count(&other, Layer::Loose) == 0 && other.iter().all(|d| d.g.glow == 0.0));
        // fire: white hot, then embers
        let hot = at(Cause::Fire, Some(0.2)).iter().map(|d| d.g.glow).fold(0.0, f32::max);
        assert!(hot > 0.6, "{hot}");
    }

    /// A fitted cloud worn as the skin: every Gaussian of it shows, as it
    /// is, on an unhurt body; a wound opens it where it would open the made
    /// skin, and a severed arm takes its Gaussians with it.
    #[test]
    fn a_fitted_cloud_is_worn_as_the_skin() {
        let p = humanoid();
        let cloud = LayerCloud::build(&p, &SplatParams::default());
        let posed = p.pose(&Pose::default());
        // stand-in for a fitted cloud: the made skin itself as drawn (a cloud
        // is bound in the puppet's rest space), in odd colours, a little off
        // the surface
        let made = cloud.dots(&p.pose_as_drawn(&Pose::default()), &LayerLook::default());
        let stand_in: Vec<Gaussian> = made.iter().step_by(2).map(|d| Gaussian { c: add(d.g.c, scale(d.g.n, 0.01)), rgb: [10.0, 200.0, 30.0], n: [0.0; 3], ..d.g.clone() }).collect();
        let skin = cloud.fit(&p, stand_in.clone());
        assert_eq!(skin.len(), stand_in.len());
        let worn = cloud.dots(&posed, &LayerLook { fitted: Some(&skin), ..LayerLook::default() });
        assert_eq!(worn.len(), stand_in.len(), "each Gaussian once");
        assert!(worn.iter().all(|d| d.layer == Layer::Skin && d.g.rgb == [10.0, 200.0, 30.0] && d.g.n == [0.0; 3]));
        let torn = cloud.dots(&posed, &LayerLook { fitted: Some(&skin), ..wounded(&[("chest", 3)]) });
        // the wound is made dots: its pit in place of the cloud's Gaussians
        // there, its blood laid over the ones round it, which keep their colour
        let kept = torn.iter().filter(|d| d.g.rgb == [10.0, 200.0, 30.0]).count();
        assert!(count(&torn, Layer::Flesh) > 10 && kept < worn.len() && kept > worn.len() * 9 / 10, "{kept} of {}", worn.len());
        assert!(torn.iter().filter(|d| d.layer == Layer::Skin).any(|d| d.g.rgb != [10.0, 200.0, 30.0] && d.g.alpha < 0.9), "bloodied round the wound");
        let cut = cloud.dots(&posed, &LayerLook { fitted: Some(&skin), ..wounded(&[("leftArm", 3)]) });
        assert!(cut.iter().filter(|d| d.g.rgb == [10.0, 200.0, 30.0]).count() < worn.len() - 100, "the arm's Gaussians go with it");
    }

    /// A coating climbs the body and stands off it: nothing at 0, the legs
    /// at a half, all of it at 1, and the figure wider for it.
    #[test]
    fn a_coating_builds_outward() {
        let p = humanoid();
        let cloud = LayerCloud::build(&p, &SplatParams::default());
        let posed = p.pose(&Pose::default());
        // (stone: one dot a point; ice is two films deep)
        let coated = |progress: f32| cloud.dots(&posed, &LayerLook { coat: Some(Coating::new(Material::Stone, progress)), ..LayerLook::default() });
        let (none, half, full) = (coated(0.0), coated(0.5), coated(1.0));
        assert_eq!(count(&none, Layer::Coat), 0);
        assert!(count(&half, Layer::Coat) > 500 && count(&full, Layer::Coat) > count(&half, Layer::Coat));
        // all of the skin that shows: none on skin buried in another part
        let showing = cloud.anchors().iter().filter(|a| a.inside.is_none()).count();
        assert!(showing < cloud.len() && count(&full, Layer::Coat) * 10 > showing * 9 && count(&full, Layer::Coat) <= showing + showing / 50, "{} of {showing}", count(&full, Layer::Coat));
        // and it goes with the skin: cut through over an open wound
        let cut = cloud.dots(&posed, &LayerLook { coat: Some(Coating::new(Material::Ice, 1.0)), ..wounded(&[("chest", 3)]) });
        // (a coat dot and a pit's dot may share an anchor -- it is drawn in
        // smaller pieces near a wound -- but never the same place on it)
        let mut pits: std::collections::HashMap<u32, Vec<&Dot>> = std::collections::HashMap::new();
        cut.iter().filter(|d| d.layer == Layer::Flesh).for_each(|d| pits.entry(d.anchor.unwrap()).or_default().push(d));
        assert!(!pits.is_empty());
        for coat in cut.iter().filter(|d| d.layer == Layer::Coat) {
            for pit in pits.get(&coat.anchor.unwrap()).into_iter().flatten() {
                let v = sub(coat.g.c, pit.g.c);
                let aside = len(sub(v, scale(coat.g.n, dot(v, coat.g.n))));
                assert!(aside > 0.003, "ice {aside} ft from over an open wound");
            }
        }
        // frozen AFTER the wound, the ice lies over it whole
        let iced = cloud.dots(&posed, &LayerLook { coat: Some(Coating::new(Material::Ice, 1.0).over_wounds(true)), ..wounded(&[("chest", 3)]) });
        assert!(count(&iced, Layer::Coat) > count(&cut, Layer::Coat) && count(&iced, Layer::Flesh) == count(&cut, Layer::Flesh));
        assert_eq!(Coating::parse("ice:0.6:over"), Some(Coating::new(Material::Ice, 0.6).over_wounds(true)));
        let top = |dots: &[Dot]| dots.iter().filter(|d| d.layer == Layer::Coat).map(|d| d.g.c[1]).fold(0.0, f32::max);
        assert!(top(&half) < p.height * 0.75 && top(&full) > p.height * 0.9, "{} then {}", top(&half), top(&full));
        let skin_at: std::collections::HashMap<u32, &Dot> = full.iter().filter(|d| d.layer == Layer::Skin).map(|d| (d.anchor.unwrap(), d)).collect();
        for d in full.iter().filter(|d| d.layer == Layer::Coat) {
            let skin = skin_at[&d.anchor.unwrap()];
            assert!(dot(sub(d.g.c, skin.g.c), skin.g.n) > 0.0);
        }
        let reach = |dots: &[Dot]| dots.iter().map(|d| d.g.c[0].abs()).fold(0.0, f32::max);
        assert!(reach(&full) > reach(&none) + COAT_FT * 0.2, "{} vs {}", reach(&full), reach(&none));
        assert_eq!(Coating::parse("ice:0.6"), Some(Coating::new(Material::Ice, 0.6)));
        assert_eq!(Coating::parse("Stone").map(|c| (c.material, c.progress)), Some((Material::Stone, 1.0)));
        assert_eq!(Coating::parse("mud:1"), None);
    }

    /// A severed arm ends in a cross-section, flesh round bone, and the rest
    /// of it is gone.
    #[test]
    fn a_stump_shows_its_inside() {
        let p = humanoid();
        let cloud = LayerCloud::build(&p, &SplatParams::default());
        let posed = p.pose(&Pose::default());
        let whole = cloud.dots(&posed, &LayerLook::default());
        let cut = cloud.dots(&posed, &wounded(&[("leftArm", 3)]));
        let cap: Vec<&Dot> = cut.iter().filter(|d| d.anchor.is_none()).collect();
        assert!(cap.iter().any(|d| d.layer == Layer::Bone) && cap.iter().any(|d| d.layer == Layer::Flesh), "{} cap dots", cap.len());
        assert!(count(&cut, Layer::Skin) < count(&whole, Layer::Skin) - 200, "the arm below the cut is gone");
    }
}
