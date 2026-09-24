//! Clothing: a small kit of garment pieces a creature can wear -- a robe, a
//! hood, pauldrons, a cloak -- built onto its puppet as extra segments, like
//! held props, but painted and part of its outline.
//!
//! The kit is data: `data/outfits/kit.toml` (see its header) names each
//! piece, the [`Cut`] it is built as and its defaults; an [`Outfit`] lists
//! the pieces one creature wears, with its own tweaks (`outfits.toml`).
//! [`Puppet::wearing`] builds them from the puppet's own measured body -- a
//! robe's waist is its pelvis, its hem as wide as its legs there -- as
//! [`Kind::Garment`] segments after the body, and hides the body segments a
//! garment covers (the thighs and shins under a long robe), which are then
//! never drawn, outlined or given depth. Those still stand on the floor:
//! garments never do ([`Puppet::pose`] grounds the body alone).
//!
//! Rigid pieces (hood, pauldrons) follow the segment they hang from. Hanging
//! ones are aimed every pose ([`Hang`]): a robe at the knees and then the
//! ankles, so it drapes over the lap sitting and follows the legs lying; a
//! cloak straight down, or along the body as it lies down.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::math::{add, axis_angle, cross, dot, len, lerp, norm_or, scale, sub, Xf, V3};
use crate::pose::Posed;
use crate::puppet::{Kind, Puppet, Segment, Station};

/// The clothing kit this crate ships, embedded.
pub const SHIPPED_KIT: &str = include_str!("../../../data/outfits/kit.toml");

/// How a piece is built (`shape` in the kit file).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Cut {
    /// Hangs from the waist to a hem: a robe, a tunic, a skirt.
    Hem,
    /// A shell over the head, set back so the face shows.
    Hood,
    /// A dome over each shoulder.
    Pauldron,
    /// Hangs from the shoulders behind the body.
    Cloak,
}

/// One piece of the kit, or as one creature wears it.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Piece {
    pub shape: Cut,
    /// Hem: how far it reaches from the waist toward the floor (1 = the
    /// floor). Cloak: the same, from the shoulders.
    #[serde(default = "one")]
    pub length: f32,
    /// How much wider than what it covers it is at the hem.
    #[serde(default = "one")]
    pub flare: f32,
    /// Hood, pauldron: size as a multiple of the head / the shoulder.
    #[serde(default = "one")]
    pub size: f32,
}

fn one() -> f32 {
    1.0
}

/// A creature's own settings for a piece; unset ones keep the kit's.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Tweak {
    length: Option<f32>,
    flare: Option<f32>,
    size: Option<f32>,
}

/// The pieces there are.
#[derive(Debug, Clone, Default)]
pub struct Kit {
    pieces: BTreeMap<String, Piece>,
}

/// What one creature wears: kit pieces, with its tweaks applied.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outfit {
    pub pieces: Vec<(String, Piece)>,
}

impl Kit {
    pub fn from_toml(text: &str) -> Result<Kit, String> {
        let pieces: BTreeMap<String, Piece> = toml::from_str(text).map_err(|e| format!("kit: {e}"))?;
        for (name, p) in &pieces {
            if !(p.length > 0.0 && p.flare > 0.0 && p.size > 0.0) {
                return Err(format!("kit: piece '{name}' needs a positive length, flare and size"));
            }
        }
        Ok(Kit { pieces })
    }

    /// The kit this crate ships.
    pub fn shipped() -> &'static Kit {
        static KIT: std::sync::OnceLock<Kit> = std::sync::OnceLock::new();
        KIT.get_or_init(|| Kit::from_toml(SHIPPED_KIT).expect("the shipped kit parses"))
    }

    pub fn piece(&self, name: &str) -> Option<&Piece> {
        self.pieces.get(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.pieces.keys().map(String::as_str)
    }

    /// The outfit of these pieces, as the kit makes them.
    pub fn outfit(&self, names: &[&str]) -> Result<Outfit, String> {
        names
            .iter()
            .map(|n| self.piece(n).map(|p| (n.to_string(), *p)).ok_or_else(|| format!("no piece '{n}' in the kit")))
            .collect::<Result<_, _>>()
            .map(|pieces| Outfit { pieces })
    }
}

/// `outfits.toml`: one table per creature (bestiary key), each with `wear`
/// and, optionally, a table of tweaks per piece:
///
/// ```toml
/// [arachne_priest]
/// wear = ["robe", "hood"]
/// robe = { flare = 1.3 }
/// ```
#[derive(Debug, Clone, Default)]
pub struct Outfits {
    by_creature: BTreeMap<String, Outfit>,
}

impl Outfits {
    pub fn from_toml(text: &str, kit: &Kit) -> Result<Outfits, String> {
        let tables: BTreeMap<String, toml::Table> = toml::from_str(text).map_err(|e| format!("outfits: {e}"))?;
        let mut by_creature = BTreeMap::new();
        for (creature, table) in tables {
            let wear: Vec<String> = match table.get("wear") {
                Some(v) => v.clone().try_into().map_err(|e| format!("outfits: {creature}.wear: {e}"))?,
                None => return Err(format!("outfits: {creature} has no `wear` list")),
            };
            let mut pieces = Vec::new();
            for name in &wear {
                let mut piece = *kit.piece(name).ok_or_else(|| format!("outfits: {creature} wears '{name}', which the kit has not"))?;
                if let Some(t) = table.get(name) {
                    let t: Tweak = t.clone().try_into().map_err(|e| format!("outfits: {creature}.{name}: {e}"))?;
                    piece.length = t.length.unwrap_or(piece.length);
                    piece.flare = t.flare.unwrap_or(piece.flare);
                    piece.size = t.size.unwrap_or(piece.size);
                }
                pieces.push((name.clone(), piece));
            }
            for key in table.keys().filter(|k| *k != "wear" && !wear.contains(k)) {
                return Err(format!("outfits: {creature} tweaks '{key}' but does not wear it"));
            }
            by_creature.insert(creature, Outfit { pieces });
        }
        Ok(Outfits { by_creature })
    }

    pub fn get(&self, creature: &str) -> Option<&Outfit> {
        self.by_creature.get(creature)
    }
}

/// How a garment segment follows a pose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hang {
    /// Exactly as the segment it hangs from.
    Rigid,
    /// Turned about its head to point at the midpoint of two segments' far
    /// ends: a robe at the knees, then the ankles.
    Toward { a: usize, b: usize },
    /// Straight down, or, as the body lies down, along it (from `top`'s
    /// head toward `bottom`'s): a cloak.
    Down { top: usize, bottom: usize },
}

/// Stations along a hanging piece, per segment.
const HEM_STATIONS: usize = 7;
/// A robe's waist sits this share of the way from the hips up the pelvis
/// segment, and is this much wider than the body there.
const WAIST_UP: f32 = 0.3;
const EASE: f32 = 1.08;
/// A hanging piece lying flat is squashed to this much less than its height.
const SETTLE: f32 = 0.5;
/// A hood sits this many head depths back from the head, and a little up,
/// so the face stands out of its opening.
const HOOD_BACK: f32 = 0.55;
const HOOD_UP: f32 = 0.06;

impl Puppet {
    /// This puppet wearing `outfit` (see the module docs). A piece the body
    /// has no place for (a robe without two legs, a hood without a head) is
    /// left off. Wear before [`Puppet::holding`]: garments come after the
    /// body and before held props.
    pub fn wearing(&self, outfit: &Outfit) -> Puppet {
        let mut out = self.clone();
        debug_assert!(out.segments.iter().all(|s| !s.kind.is_prop()), "wear before holding");
        for (name, piece) in &outfit.pieces {
            match piece.shape {
                Cut::Hem => hem(&mut out, name, piece),
                Cut::Hood => hood(&mut out, name, piece),
                Cut::Pauldron => pauldrons(&mut out, name, piece),
                Cut::Cloak => cloak(&mut out, name, piece),
            }
        }
        out
    }
}

/// The body's legs (`*Leg` parts): each bone's chain, root first.
fn legs(p: &Puppet) -> Vec<Vec<usize>> {
    let mut by_bone: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, s) in p.segments.iter().enumerate() {
        if s.kind.is_body() && s.part.as_deref().is_some_and(|x| x.ends_with("Leg")) {
            by_bone.entry(&s.bone).or_default().push(i);
        }
    }
    by_bone.into_values().collect()
}

/// A chain's rest cross-section at height `y`: its centre and half extents
/// along x and z (the ellipse's reach along each world axis).
fn section_at(p: &Puppet, chain: &[usize], y: f32) -> Option<(V3, [f32; 2])> {
    let stations: Vec<(&Segment, &Station)> = chain.iter().flat_map(|&i| p.segments[i].stations.iter().map(move |st| (&p.segments[i], st))).collect();
    stations.windows(2).find_map(|w| {
        let ((sa, a), (sb, b)) = (w[0], w[1]);
        let (lo, hi) = (a.c[1].min(b.c[1]), a.c[1].max(b.c[1]));
        if y < lo || y > hi || hi - lo < 1e-5 {
            return None;
        }
        let t = (y - a.c[1]) / (b.c[1] - a.c[1]);
        let reach = |s: &Segment, st: &Station, k: usize| st.w * s.axis_w[k].abs() + st.d * s.axis_d[k].abs();
        let ext = [0, 2].map(|k| reach(sa, a, k) + (reach(sb, b, k) - reach(sa, a, k)) * t);
        Some((lerp(a.c, b.c, t), ext))
    })
}

#[allow(clippy::too_many_arguments)]
fn garment(out: &mut Puppet, name: &str, bone: &str, parent: usize, hang: Hang, chain_index: usize, stations: Vec<Station>, group: usize) -> usize {
    let (head, tail) = (stations[0].c, stations[stations.len() - 1].c);
    out.segments.push(Segment {
        name: name.to_owned(),
        bone: bone.to_owned(),
        part: None,
        parent: Some(parent),
        group,
        kind: Kind::Garment { hang },
        chain_index,
        head,
        tail,
        axis_w: [1.0, 0.0, 0.0],
        axis_d: [0.0, 0.0, 1.0],
        hinge: [1.0, 0.0, 0.0],
        stations,
        bare: 0,
        hidden: false,
    });
    out.segments.len() - 1
}

fn new_group(out: &mut Puppet, name: &str) -> usize {
    out.groups.push(format!("garment_{name}"));
    out.groups.len() - 1
}

/// Waist to hem, as wide as the legs (and the hips) it covers, flaring
/// toward the hem; split at the knees so it can bend there.
fn hem(out: &mut Puppet, name: &str, piece: &Piece) {
    let legs = legs(out);
    if legs.len() != 2 || legs.iter().any(|l| l.len() < 2) {
        return;
    }
    let Some(pelvis) = out.segments[legs[0][0]].parent else { return };
    let ps = &out.segments[pelvis];
    let hip = legs.iter().map(|l| out.segments[l[0]].head[1]).fold(f32::MIN, f32::max);
    let knee = legs.iter().map(|l| out.segments[l[0]].tail[1]).sum::<f32>() / 2.0;
    let waist = hip + WAIST_UP * (ps.tail[1] - ps.head[1]).abs();
    let hem_y = (waist * (1.0 - piece.length.clamp(0.05, 1.0))).max(0.0);
    let Some((wc, wext)) = section_at(out, &[pelvis], waist) else { return };

    // Half extents of the covered body at a height: the hips' at the
    // waist, the two legs' together below.
    let covered = |y: f32| -> Option<(V3, [f32; 2])> {
        let secs: Vec<(V3, [f32; 2])> = legs.iter().filter_map(|l| section_at(out, l, y)).collect();
        if secs.len() < 2 {
            return None;
        }
        let c = scale(add(secs[0].0, secs[1].0), 0.5);
        let half = |k: usize, e: usize| secs.iter().map(|(p, ext)| (p[k] - c[k]).abs() + ext[e]).fold(0.0f32, f32::max);
        Some(([c[0], y, c[2]], [half(0, 0), half(2, 1)]))
    };
    let station_at = |y: f32| -> Station {
        let t = ((waist - y) / (waist - hem_y).max(1e-3)).clamp(0.0, 1.0);
        let flare = 1.0 + (piece.flare - 1.0) * t.powf(1.2);
        // Cloth falls straight from the widest of the hips, then flares:
        // never narrower than the hips, even in profile.
        let (c, ext) = match covered(y) {
            Some((c, legs_ext)) => (c, [legs_ext[0].max(wext[0]), legs_ext[1].max(wext[1])]),
            None => ([wc[0], y, wc[2]], wext),
        };
        Station { c: [c[0], y, c[2]], w: ext[0] * EASE * flare, d: ext[1] * EASE * flare }
    };
    let run = |from: f32, to: f32| -> Vec<Station> {
        let mut v: Vec<Station> = (0..HEM_STATIONS).map(|k| station_at(from + (to - from) * k as f32 / (HEM_STATIONS - 1) as f32)).collect();
        // It only widens on the way down.
        for k in 1..v.len() {
            v[k].w = v[k].w.max(v[k - 1].w);
            v[k].d = v[k].d.max(v[k - 1].d);
        }
        v
    };
    let (thighs, shins) = ((legs[0][0], legs[1][0]), (legs[0][1], legs[1][1]));
    let mut upper = run(waist, hem_y.max(knee));
    let top = upper[0];
    // A lip at the waist, tucked into the body.
    upper.insert(0, Station { c: add(top.c, [0.0, 0.02, 0.0]), w: top.w * 0.85, d: top.d * 0.85 });
    let lower = (hem_y < knee - 0.05).then(|| {
        let mut lower = run(knee, hem_y);
        lower[0] = *upper.last().expect("stations");
        for k in 1..lower.len() {
            lower[k].w = lower[k].w.max(lower[k - 1].w);
            lower[k].d = lower[k].d.max(lower[k - 1].d);
        }
        lower
    });
    let ankle = legs.iter().map(|l| out.segments[l[1]].tail[1]).sum::<f32>() / 2.0;

    let group = new_group(out, name);
    let first = garment(out, &format!("{name}.0"), name, pelvis, Hang::Toward { a: thighs.0, b: thighs.1 }, 0, upper, group);
    if let Some(lower) = lower {
        garment(out, &format!("{name}.1"), name, first, Hang::Toward { a: shins.0, b: shins.1 }, 1, lower, group);
    }
    // What it covers is not drawn: the thighs under anything to the knees,
    // the shins under anything to the ankles. Feet always show.
    for leg in &legs {
        if hem_y <= knee + 0.05 * (waist - knee) {
            out.segments[leg[0]].hidden = true;
        }
        if hem_y <= ankle + 0.25 * (knee - ankle) {
            out.segments[leg[1]].hidden = true;
        }
    }
}

/// A cowl: from the neck base up round the head to a peak behind the crown,
/// set back so the face stands out of its opening.
fn hood(out: &mut Puppet, name: &str, piece: &Piece) {
    let Some(head) = out.segments.iter().position(|s| s.kind.is_body() && s.part.as_deref() == Some("head")) else { return };
    let s = out.segments[head].clone();
    let up = norm_or(sub(s.tail, s.head), [0.0, 1.0, 0.0]);
    let long = len(sub(s.tail, s.head));
    let (hw, hd) = s.stations.iter().fold((0.0f32, 0.0f32), |(w, d), st| (w.max(st.w), d.max(st.d)));
    let size = piece.size;
    // (along the head, chin 0 -> crown 1; width and depth as multiples of
    // the head's; how far back, in head depths)
    let profile: [(f32, f32, f32, f32); 8] = [
        (-0.35, 1.55, 1.35, 0.35),
        (0.0, 1.32, 1.35, HOOD_BACK),
        (0.3, 1.3, 1.3, HOOD_BACK),
        (0.6, 1.28, 1.28, HOOD_BACK),
        (0.9, 1.2, 1.22, HOOD_BACK),
        (1.05, 1.0, 1.02, HOOD_BACK + 0.05),
        (1.16, 0.7, 0.75, HOOD_BACK + 0.15),
        (1.23, 0.3, 0.35, HOOD_BACK + 0.3),
    ];
    let stations = profile
        .iter()
        .map(|&(t, ws, ds, back)| Station {
            c: add(add(s.head, scale(up, t * long)), [0.0, HOOD_UP * long, -back * hd]),
            w: hw * ws * size,
            d: hd * ds * size,
        })
        .collect();
    let group = new_group(out, name);
    let i = garment(out, name, name, head, Hang::Rigid, 0, stations, group);
    let g = &mut out.segments[i];
    (g.axis_w, g.axis_d, g.hinge) = (s.axis_w, s.axis_d, s.hinge);
}

/// A dome over the top of each upper arm.
fn pauldrons(out: &mut Puppet, name: &str, piece: &Piece) {
    let arms: Vec<usize> = (0..out.segments.len())
        .filter(|&i| {
            let s = &out.segments[i];
            s.kind.is_body() && s.chain_index == 0 && s.part.as_deref().is_some_and(|p| p.ends_with("Arm"))
        })
        .collect();
    if arms.is_empty() {
        return;
    }
    let group = new_group(out, name);
    for arm in arms {
        let s = out.segments[arm].clone();
        let axis = norm_or(sub(s.tail, s.head), [0.0, -1.0, 0.0]);
        let long = len(sub(s.tail, s.head));
        let r0 = s.stations.iter().take(s.stations.len() / 3 + 1).map(|st| st.w.max(st.d)).fold(0.0f32, f32::max);
        let r = r0 * 1.45 * piece.size;
        // From above the shoulder to a third of the way down, rounded at
        // the top, full at the rim.
        let (from, to) = (-0.12 * long, 0.33 * long);
        let stations: Vec<Station> = (0..9)
            .map(|k| {
                let u = k as f32 / 8.0;
                let along = from + (to - from) * u;
                let bulge = (1.0 - (1.0 - u.min(0.6) / 0.6).powi(2)).max(0.05).sqrt();
                Station { c: add(s.head, scale(axis, along)), w: r * bulge, d: r * bulge * 0.9 }
            })
            .collect();
        let side = if s.bone.ends_with("_l") { "_l" } else { "_r" };
        let i = garment(out, &format!("{name}{side}"), &format!("{name}{side}"), arm, Hang::Rigid, 0, stations, group);
        let g = &mut out.segments[i];
        (g.axis_w, g.axis_d, g.hinge) = (s.axis_w, s.axis_d, s.hinge);
    }
}

/// From the shoulders down behind the body, as wide as the shoulders and
/// wrapping round the sides of the back.
fn cloak(out: &mut Puppet, name: &str, piece: &Piece) {
    let Some(neck) = out.segments.iter().position(|s| s.kind.is_body() && s.part.as_deref() == Some("neck")) else { return };
    let Some(chest) = out.segments[neck].parent else { return };
    let legs = legs(out);
    let Some(pelvis) = legs.first().and_then(|l| out.segments[l[0]].parent) else { return };
    let top_y = out.segments[neck].head[1];
    let hem_y = (top_y * (1.0 - piece.length.clamp(0.05, 1.0))).max(0.0);
    let shoulders = out
        .segments
        .iter()
        .filter(|s| s.kind.is_body() && s.chain_index == 0 && s.part.as_deref().is_some_and(|p| p.ends_with("Arm")))
        .map(|s| s.head[0].abs() + s.stations.iter().map(|st| st.w.max(st.d)).fold(0.0f32, f32::max) * 0.6)
        .fold(0.0f32, f32::max);
    if shoulders <= 0.0 {
        return;
    }
    // The body's back and depth at a height: the trunk above the hips, the
    // legs below.
    let trunk: Vec<usize> = (0..out.segments.len()).filter(|&i| out.segments[i].kind == Kind::Trunk && out.segments[i].part.as_deref() != Some("head")).collect();
    let back_at = |y: f32| -> Option<(f32, f32)> {
        let mut secs: Vec<(V3, [f32; 2])> = trunk.iter().filter_map(|&i| section_at(out, &[i], y)).collect();
        if secs.is_empty() {
            secs = legs.iter().filter_map(|l| section_at(out, l, y)).collect();
        }
        let back = secs.iter().map(|(c, e)| c[2] - e[1]).fold(f32::INFINITY, f32::min);
        let front = secs.iter().map(|(c, e)| c[2] + e[1]).fold(f32::NEG_INFINITY, f32::max);
        back.is_finite().then_some((back, front - back))
    };
    let mut stations: Vec<Station> = (0..10)
        .map(|k| {
            let t = k as f32 / 9.0;
            let y = top_y + (hem_y - top_y) * t;
            let (back, depth) = back_at(y).unwrap_or((-0.2, 0.4));
            let d = depth * (0.55 - 0.25 * t);
            let w = shoulders * 1.05 * (1.0 + (piece.flare - 1.0) * t);
            // Its front face a third of the body's depth in front of the
            // back, so it wraps the sides and the front stays open -- but
            // over the shoulders it comes most of the way forward.
            let over = (1.0 - t / 0.3).clamp(0.0, 1.0);
            Station { c: [0.0, y, back + depth * (0.33 + 0.4 * over) - d], w, d }
        })
        .collect();
    // A rounded top over the shoulders, not a flat edge.
    let top = stations[0];
    let rise = 0.3 * top.d;
    for k in 1..=3 {
        let phi = k as f32 / 3.0 * std::f32::consts::FRAC_PI_2 * 0.94;
        stations.insert(0, Station { c: add(top.c, [0.0, rise * phi.sin(), 0.0]), w: top.w * phi.cos(), d: top.d * phi.cos() });
    }
    let group = new_group(out, name);
    garment(out, name, name, chest, Hang::Down { top: neck, bottom: pelvis }, 0, stations, group);
}

/// Aim every hanging garment for this pose. Runs after grounding, over the
/// garments in order, each from its parent's hung (not settled) frame.
pub(crate) fn hang(posed: &mut Posed) {
    let p = posed.puppet;
    let mut hung: Vec<Option<Xf>> = vec![None; p.segments.len()];
    for (i, s) in p.segments.iter().enumerate() {
        let Kind::Garment { hang } = s.kind else { continue };
        // A garment's bone is in no pose: it rides its parent.
        let base = s.parent.map(|q| hung[q].unwrap_or(posed.xf[q])).unwrap_or(Xf::IDENTITY);
        let head = base.point(s.head);
        let now = norm_or(base.dir(sub(s.tail, s.head)), [0.0, -1.0, 0.0]);
        let tip = |j: usize| posed.xf[j].point(p.segments[j].tail);
        let want = match hang {
            Hang::Rigid => None,
            Hang::Toward { a, b } => Some(norm_or(sub(scale(add(tip(a), tip(b)), 0.5), head), now)),
            Hang::Down { top, bottom } => {
                let along = norm_or(sub(posed.xf[bottom].point(p.segments[bottom].head), posed.xf[top].point(p.segments[top].head)), [0.0, -1.0, 0.0]);
                let down = [0.0, -1.0, 0.0];
                let upright = dot(along, down).clamp(0.0, 1.0);
                Some(norm_or(add(scale(down, upright), scale(along, 1.0 - upright)), down))
            }
        };
        let Some(w) = want else {
            hung[i] = Some(base);
            posed.xf[i] = base;
            continue;
        };
        let aimed = Xf::about(head, turn_between(now, w)).then_inner(&base);
        hung[i] = Some(aimed);
        // Lying down, cloth settles: squashed toward the floor, up to
        // SETTLE of its height when it lies flat.
        let flat = 1.0 - SETTLE * (1.0 - w[1].abs());
        let squash = [[1.0, 0.0, 0.0], [0.0, flat, 0.0], [0.0, 0.0, 1.0]];
        posed.xf[i] = Xf::about([head[0], 0.0, head[2]], squash).then_inner(&aimed);
    }
}

/// The smallest rotation taking unit `a` to unit `b`.
fn turn_between(a: V3, b: V3) -> crate::math::M3 {
    let axis = cross(a, b);
    let s = len(axis);
    let c = dot(a, b).clamp(-1.0, 1.0);
    if s < 1e-6 {
        let other = if a[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 0.0, 1.0] };
        let perp = norm_or(cross(a, other), [0.0, 0.0, 1.0]);
        return axis_angle(perp, if c > 0.0 { 0.0 } else { 180.0 });
    }
    axis_angle(scale(axis, 1.0 / s), s.atan2(c).to_degrees())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Pose, PoseSet};

    fn humanoid() -> Puppet {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/rigs/humanoid.puppet.toml")).unwrap();
        Puppet::from_baked_toml(&text).unwrap()
    }

    #[test]
    fn the_shipped_kit_parses_and_outfits_resolve() {
        let kit = Kit::shipped();
        for n in ["robe", "robe_short", "skirt", "hood", "pauldrons", "cloak"] {
            assert!(kit.piece(n).is_some(), "the kit has {n}");
        }
        let outfits = Outfits::from_toml("[priest]\nwear = [\"robe\", \"hood\"]\nrobe = { flare = 1.4 }\n", kit).unwrap();
        let o = outfits.get("priest").unwrap();
        assert_eq!(o.pieces.len(), 2);
        assert_eq!(o.pieces[0].1.flare, 1.4);
        assert!(Outfits::from_toml("[x]\nwear = [\"tophat\"]\n", kit).unwrap_err().contains("tophat"));
        assert!(Outfits::from_toml("[x]\nwear = [\"robe\"]\nhood = { size = 2.0 }\n", kit).unwrap_err().contains("does not wear"));
    }

    /// A long robe covers the legs to the floor: thighs and shins hidden,
    /// feet not, and the figure still stands where it stood.
    #[test]
    fn a_robe_hides_the_legs_and_leaves_the_feet() {
        let p = humanoid();
        let robed = p.wearing(&Kit::shipped().outfit(&["robe"]).unwrap());
        assert!(robed.segments.len() > p.segments.len());
        for s in robed.segments.iter().filter(|s| s.part.as_deref().is_some_and(|x| x.ends_with("Leg"))) {
            let foot = s.chain_index == 2;
            assert_eq!(s.hidden, !foot, "{}: hidden = {}", s.name, s.hidden);
        }
        let stand = PoseSet::shipped("biped").unwrap().get("stand").unwrap().clone();
        let (bare, worn) = (p.pose(&stand), robed.pose(&stand));
        assert!((bare.lift - worn.lift).abs() < 1e-5, "a robe does not lift the figure");
        let hem = worn.bounds_where(|s| s.kind.is_garment())[0][1];
        assert!(hem.abs() < 0.15, "a long robe reaches the floor: hem at {hem:.2}");
    }

    /// Sitting, the robe follows the thighs forward over the lap instead of
    /// hanging straight through the seat.
    #[test]
    fn a_robe_drapes_over_the_lap_sitting() {
        let robed = humanoid().wearing(&Kit::shipped().outfit(&["robe"]).unwrap());
        let sit = PoseSet::shipped("biped").unwrap().get("sit").unwrap().clone();
        let posed = robed.pose(&sit);
        let i = robed.segments.iter().position(|s| s.name == "robe.0").unwrap();
        let s = &robed.segments[i];
        let dir = norm_or(posed.xf[i].dir(sub(s.tail, s.head)), [0.0; 3]);
        assert!(dir[2] > 0.5, "the robe's upper part points forward sitting: {dir:?}");
        let rest = robed.pose(&Pose::default());
        let rdir = norm_or(rest.xf[i].dir(sub(s.tail, s.head)), [0.0; 3]);
        assert!(rdir[1] < -0.9, "and straight down standing: {rdir:?}");
    }

    /// A cloak hangs down while standing, and along the body lying down.
    #[test]
    fn a_cloak_hangs_down_or_along_the_body() {
        let cloaked = humanoid().wearing(&Kit::shipped().outfit(&["cloak"]).unwrap());
        let i = cloaked.segments.iter().position(|s| s.name == "cloak").unwrap();
        let s = &cloaked.segments[i];
        let set = PoseSet::shipped("biped").unwrap();
        let dir = |pose: &Pose| {
            let posed = cloaked.pose(pose);
            norm_or(posed.xf[i].dir(sub(s.tail, s.head)), [0.0; 3])
        };
        assert!(dir(set.get("stand").unwrap())[1] < -0.95, "straight down standing");
        let lying = dir(set.get("prone").unwrap());
        assert!(lying[1] > -0.5, "along the body lying, not into the floor: {lying:?}");
    }

    #[test]
    fn a_hood_and_pauldrons_ride_their_bones() {
        let p = humanoid();
        let worn = p.wearing(&Kit::shipped().outfit(&["hood", "pauldrons"]).unwrap());
        let names: Vec<&str> = worn.segments[p.segments.len()..].iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["hood", "pauldrons_r", "pauldrons_l"]);
        assert!(worn.segments.iter().all(|s| !s.hidden), "nothing is covered");
        let arms_up = PoseSet::shipped("biped").unwrap().get("arms_up").unwrap().clone();
        let posed = worn.pose(&arms_up);
        let arm = worn.segments.iter().position(|s| s.name == "arm_r.0").unwrap();
        let pauldron = worn.segments.iter().position(|s| s.name == "pauldrons_r").unwrap();
        assert_eq!(posed.xf[pauldron].r, posed.xf[arm].r, "a pauldron turns with its arm");
    }
}
