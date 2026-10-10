//! The creature field's skin: the `[creature_card]` and `[creature_field]`
//! sections of a `skin.toml` (the card template, its overlays and
//! variants, the solver's camera), the body's part table the cards' wounds
//! are placed by ([`DOLL_PARTS`]), the severity keys, and how a skin's image
//! paths resolve.
//!
//! Ported from VellumFE's skin manifest; its GUI skin -- window
//! backgrounds and borders, edges, icons, sheets, frames, the compass, the
//! injury doll, the palette, the controls, the scaffold writer -- was
//! deleted in the crate split (docs/crate-split-plan.md, S9): serde ignores
//! those tables in a manifest that still has them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Parsed skin.toml: the creature field's two sections. (VellumFE's GUI
/// skin -- windows, edges, icons, sheets, frames, the compass, the injury
/// doll, the palette, the controls -- is not here: serde ignores those
/// tables.)
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SkinManifest {
    /// Creature cards: the shared card template for the creaturefield
    /// widget. One template serves every creature; per-creature art comes
    /// from the resolve cascade, per-creature state from `<crtrStatus>`.
    #[serde(default)]
    pub creature_card: CreatureCardSkin,
    /// Creature field ground-plane tuning: the solver's camera. Every field
    /// optional; unset values keep the built-in defaults, out-of-range
    /// values clamp with a warning (a bad focal degrades, never drops the
    /// widget). Lives in the skin so art can ship a matched camera, and so
    /// tuning rides the skin hot-reload instead of restart-per-guess.
    #[serde(default)]
    pub creature_field: CreatureFieldSkin,
}

/// One body part's manifest entry: overlay art per state, plus options.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DollPartSpec {
    /// Skip this part entirely (no overlay, no dot, at any severity)
    /// while the condition holds. Lets the skin encode anatomical
    /// dependencies without the client hardcoding an anatomy tree — e.g.
    /// hide a healthy hand while its arm is severed, so the hand doesn't
    /// float next to the stump:
    ///
    /// ```toml
    /// [injury_doll.leftHand]
    /// hidden_when = { type = "injury", area = "leftArm", cmp = ">=", level = 3 }
    /// healthy = "doll/leftHand_ok.png"
    /// ```
    #[serde(default)]
    pub hidden_when: Option<crate::condition::Condition>,
    /// State key (healthy / injury1-3 / scar1-3) -> image path.
    #[serde(flatten)]
    pub overlays: HashMap<String, String>,
}

/// Creature card template, deliberately the injury doll's shape so the
/// condition evaluator, variant matcher, and calibration UI are shared
/// rather than forked. Differences from `[injury_doll]`:
///
/// - One template serves EVERY creature; the base image resolves per
///   creature through `resolve` ({noun}/{family} placeholders).
/// - Conditions here are creature-scoped: `crtr_status` leaves test the
///   card's creature; player-scoped leaves still work and read the player.
/// - Creatures take wounds only: injury1-3 + healthy. Scar states
///   (scar1-3) are intentionally unused — the loader drops them — but the
///   key space stays reserved so a future reversal is a content change.
/// - Status overlay art is SHARED across families (scaled to the card's
///   alpha bbox or placed by anchor fraction). Never author per-family
///   status art; that is what keeps the asset count linear.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CreatureCardSkin {
    /// Final fallback when nothing in `resolve` exists on disk.
    #[serde(default)]
    pub base: Option<String>,
    /// Base-image resolution cascade, first existing file wins. Templates:
    /// `{noun}` (from the room-objs parse) and `{family}` (when known).
    /// Candidates whose placeholders can't be filled are skipped. Empty =
    /// the built-in default cascade.
    #[serde(default)]
    pub resolve: Vec<String>,
    /// Card anchor points as fractions (0-1) of the base image: "head",
    /// "mouth", "feet", and "saddle" (mount-capable families). Missing
    /// anchors fall back to `CREATURE_ANCHOR_DEFAULTS`.
    #[serde(default)]
    pub anchors: HashMap<String, [f32; 2]>,
    /// Status/effect layers, evaluated per creature in declaration order;
    /// every matching overlay draws (unlike variants, these stack).
    #[serde(default)]
    pub overlays: Vec<CardOverlay>,
    /// Named alternate cards selected by creature-scoped condition, first
    /// match wins (posture swaps: downed, airborne). A matched variant
    /// replaces base/anchors/parts wholesale, doll-style; its `lift`
    /// offsets the drawn card without moving its floor footprint.
    #[serde(default)]
    pub variants: Vec<CardVariant>,
    /// Per-part injury overlay art, identical shape to the doll's part
    /// tables (scar keys dropped at load).
    #[serde(flatten)]
    pub parts: HashMap<String, DollPartSpec>,
}

/// `[creature_field]` — ground-plane tuning for the creaturefield solver.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CreatureFieldSkin {
    #[serde(default)]
    pub camera: CreatureFieldCamera,
    #[serde(default)]
    pub solver: CreatureFieldSolver,
}

/// `[creature_field.camera]` — the six ground-plane parameters. All
/// optional; `None` keeps the solver's built-in default. Names here are
/// the authoring vocabulary and deliberately read plainer than the
/// solver's short field names (`eye_height` → `cam_h`, etc.).
///
/// Serializes too: scenes embed a camera (`StageScene::camera`) and the
/// appearance store carries a per-character override, both written by
/// editors. Unset keys are omitted so a partial override stays partial.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CreatureFieldCamera {
    /// Lens; larger = flatter, less convergence. Default 420.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focal: Option<f32>,
    /// Camera height in card-heights above the plane. Default 1.6.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eye_height: Option<f32>,
    /// Distance to the front row. Default 2.4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub near_depth: Option<f32>,
    /// Spacing between rows. Default 1.5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_depth: Option<f32>,
    /// Vanishing line, px from stage top. Default 96.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizon: Option<f32>,
    /// Lateral column spacing. Default 1.15. Note this one is not purely a
    /// camera value — it also feeds placement scoring, so changing it
    /// affects how future arrivals fit, never where placed units stand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell_width: Option<f32>,
}

impl CreatureFieldCamera {
    /// No key set — serde skip helper and "nothing to apply" check.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// `[creature_field.solver]` — placement tunables for the creature-field
/// solver. All optional; `None` keeps the solver's built-in default. Same
/// authoring-vocabulary discipline as the camera table: names read plainer
/// than the solver's field names, and bad values clamp with a warning in
/// `FieldParams::apply_solver`. Serializes for the same reason the camera
/// does: scenes embed one, the appearance store can override it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CreatureFieldSolver {
    /// Spawn zone shape: "ellipse" (inscribed spawn ellipse, default) or
    /// "grid" (margin columns, no ellipse).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,
    /// Ellipse shrink from the floor edge. Default 0.10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone_inset: Option<f32>,
    /// Radial centre pull (squared falloff). Default 0.45.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub centre_pull: Option<f32>,
    /// Depth bases sampled per square. Default 9.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth_samples: Option<u32>,
    /// Depth jitter amplitude, in row depths. Default 0.22.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth_jitter: Option<f32>,
    /// Lateral jitter amplitude, in cell widths. Default 0.12.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lateral_jitter: Option<f32>,
    /// Repulsion from neighbours' world depth. Default 0.70.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth_spread: Option<f32>,
    /// Repulsion from neighbours' foot screen y. Default 1.60.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_band_push: Option<f32>,
    /// Row-band kernel width in stage pixels. Default 28.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_band_px: Option<f32>,
    /// Max identity-region coverage a candidate may cause. Default 0.18.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occlusion_cap: Option<f32>,
    /// Soft score noise. Default 0.35.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variation: Option<f32>,
    /// First arrival into an empty field goes front and centre. Default
    /// true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_front: Option<bool>,
    /// Fall-envelope overlap cost weight. Default 0.9.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fall_reserve: Option<f32>,
    /// Whether the worst envelope overlap is a hard bound. Default true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fall_reserve_hard: Option<bool>,
    /// Separation width basis: "contact" (default) or "card".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub separation_basis: Option<String>,
    /// Constraint-loosening notches at the column cap. Default 4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relax_steps: Option<u32>,
    /// Shuffle the candidate list so ties don't bias. Default true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shuffle_ties: Option<bool>,
    /// How the floor makes room for a crowd: "wide" (columns join, the
    /// field shrinks to fit), "deep" (the camera stays, rows join at the
    /// back) or "auto" (deep over a painted backdrop). Default "auto".
    /// ADDITION (not in VellumFE).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grow: Option<String>,
    /// Deep growth's farthest row edge, world units: where the painted
    /// floor ends. Default 12. ADDITION.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<f32>,
}

impl CreatureFieldSolver {
    /// No key set — serde skip helper and "nothing to apply" check.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// One status/effect layer on the card.
#[derive(Debug, Clone, Deserialize)]
pub struct CardOverlay {
    /// Image path (skin-relative). May contain `{severity}` for ranked
    /// message-derived effects (expanded 1-3 at render).
    pub image: String,
    /// Where the layer lives: warped with the card quad, or flat in screen
    /// space (head FX, reticules — never warped into the floor plane).
    #[serde(default)]
    pub space: OverlaySpace,
    /// Anchor name this layer is placed at ("head", "mouth", ...). None =
    /// body-wrap: scaled to the card's alpha bbox.
    #[serde(default)]
    pub anchor: Option<String>,
    /// Draw order within the card's layer stack (higher = nearer).
    #[serde(default)]
    pub layer: i32,
    /// Authority tier: feed-derived flags never go stale; message-derived
    /// layers must carry `timeout_s` so a missed end message can't leave
    /// the layer stuck.
    #[serde(default)]
    pub source: OverlaySource,
    /// Seconds after which a message-derived layer expires unrefreshed.
    #[serde(default)]
    pub timeout_s: Option<u32>,
    /// Data-driven motion (orbit, pulse, ...), rendered from wall clock.
    #[serde(default)]
    pub animate: Option<AnimateSpec>,
    /// Creature-scoped activation condition (`crtr_status` + the shared
    /// vocabulary).
    pub when: crate::condition::Condition,
}

/// Layer coordinate space.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlaySpace {
    #[default]
    Quad,
    Screen,
}

/// Layer authority tier (which system may activate/expire it).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlaySource {
    /// `<crtrStatus>` flags: authoritative, no expiry needed.
    #[default]
    Feed,
    /// Combat-message inference: lossy, requires `timeout_s`.
    Message,
}

/// Data-driven motion for a card overlay. All primitives take their phase
/// from the wall clock, so an idle room renders zero frames.
#[derive(Debug, Clone, Deserialize)]
pub struct AnimateSpec {
    pub kind: AnimateKind,
    /// Instances drawn (orbit stars, drift motes).
    #[serde(default = "default_animate_count")]
    pub count: u32,
    /// Orbit x-radius as a fraction of the anchor width.
    #[serde(default = "default_animate_rx")]
    pub rx: f32,
    /// Orbit y-radius as a fraction of the anchor width.
    #[serde(default = "default_animate_ry")]
    pub ry: f32,
    /// One full cycle, in milliseconds.
    #[serde(default = "default_animate_period")]
    pub period_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnimateKind {
    Orbit,
    Pulse,
    Drift,
    Shake,
    Flicker,
    Spin,
}

fn default_animate_count() -> u32 {
    1
}
fn default_animate_rx() -> f32 {
    0.42
}
fn default_animate_ry() -> f32 {
    0.14
}
fn default_animate_period() -> u32 {
    2400
}

/// One conditional card variant: a replacement card set plus its
/// creature-scoped activation condition.
#[derive(Debug, Clone, Deserialize)]
pub struct CardVariant {
    pub name: String,
    pub when: crate::condition::Condition,
    #[serde(default)]
    pub skin: CardSet,
}

/// A complete card set as carried by a variant (no nesting, doll-style).
/// `base: None` keeps the resolved ground pose — an airborne variant can be
/// pure lift with no dedicated art.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CardSet {
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub anchors: HashMap<String, [f32; 2]>,
    /// Screen-space lift for airborne variants: the card rises, the
    /// contact shadow stays at the floor footprint and softens.
    #[serde(default)]
    pub lift: Option<LiftSpec>,
    /// part -> its overlay art and options.
    #[serde(flatten)]
    pub parts: HashMap<String, DollPartSpec>,
}

/// Airborne offset: fractions of card height / shadow multipliers, so the
/// values survive the card-shrink that happens as the floor grows columns.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct LiftSpec {
    /// Vertical screen offset as a fraction of card height (negative = up).
    pub offset_y: f32,
    #[serde(default = "default_shadow_scale")]
    pub shadow_scale: f32,
    #[serde(default = "default_shadow_opacity")]
    pub shadow_opacity: f32,
}

fn default_shadow_scale() -> f32 {
    0.55
}
fn default_shadow_opacity() -> f32 {
    0.4
}

/// Built-in base-image cascade when the manifest's `resolve` is empty.
pub const CREATURE_RESOLVE_DEFAULT: &[&str] = &[
    "creatures/{noun}.png",
    "creatures/{family}.png",
    "creatures/default.png",
];

/// Default card anchors as fractions of the base image, used when the skin
/// hasn't calibrated one. Head/feet are also derivable from the sprite's
/// alpha bounds at render; these are the resting positions.
pub const CREATURE_ANCHOR_DEFAULTS: &[(&str, [f32; 2])] = &[
    ("head", [0.50, 0.06]),
    ("mouth", [0.50, 0.16]),
    ("feet", [0.50, 0.98]),
    ("saddle", [0.50, 0.35]),
];

/// Built-in anchor for a creature-card anchor name (case-insensitive).
pub fn default_creature_anchor(name: &str) -> Option<[f32; 2]> {
    CREATURE_ANCHOR_DEFAULTS
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, anchor)| *anchor)
}

/// Canonical body parts: (protocol key, display name, default anchor as
/// fractions of the doll image). Order is the calibrator's click-through
/// order. Back and nervous system have no spot on a front silhouette; by
/// convention they sit in the bottom corners (matching the vector doll's
/// "B"/"N" letters), eyes above the head line.
pub const DOLL_PARTS: &[(&str, &str, [f32; 2])] = &[
    ("head", "head", [0.50, 0.09]),
    ("leftEye", "left eye", [0.44, 0.06]),
    ("rightEye", "right eye", [0.56, 0.06]),
    ("neck", "neck", [0.50, 0.20]),
    ("chest", "chest", [0.50, 0.30]),
    ("abdomen", "abdomen", [0.50, 0.45]),
    ("back", "back", [0.12, 0.92]),
    ("leftArm", "left arm", [0.31, 0.36]),
    ("rightArm", "right arm", [0.69, 0.36]),
    ("leftHand", "left hand", [0.25, 0.53]),
    ("rightHand", "right hand", [0.75, 0.53]),
    ("leftLeg", "left leg", [0.42, 0.75]),
    ("rightLeg", "right leg", [0.58, 0.75]),
    ("nsys", "nervous system", [0.88, 0.92]),
];

/// Built-in anchor for a body part (matched case-insensitively), used when
/// the skin hasn't calibrated one.
pub fn default_doll_anchor(part: &str) -> Option<[f32; 2]> {
    DOLL_PARTS
        .iter()
        .find(|(key, _, _)| key.eq_ignore_ascii_case(part))
        .map(|(_, _, anchor)| *anchor)
}

/// Severity level for an injury-doll overlay key: healthy -> 0,
/// injury1-3 -> 1-3, scar1-3 -> 4-6.
pub fn severity_level_from_key(key: &str) -> Option<u8> {
    match key {
        "healthy" => Some(0),
        "injury1" => Some(1),
        "injury2" => Some(2),
        "injury3" => Some(3),
        "scar1" => Some(4),
        "scar2" => Some(5),
        "scar3" => Some(6),
        _ => None,
    }
}

/// Inverse of `severity_level_from_key`: 0-6 -> the manifest overlay key.
pub fn severity_key_from_level(level: u8) -> Option<&'static str> {
    match level {
        0 => Some("healthy"),
        1 => Some("injury1"),
        2 => Some("injury2"),
        3 => Some("injury3"),
        4 => Some("scar1"),
        5 => Some("scar2"),
        6 => Some("scar3"),
        _ => None,
    }
}

/// Reserved frame-override value meaning "draw no skin frame".
pub const NO_FRAME: &str = "none";

/// Marker comment identifying a skin.toml written by the harmony generator.
/// `write_harmony_skin` only overwrites manifests carrying it, so a
/// regenerate can never clobber a hand-authored skin under the same name.
pub const HARMONY_SKIN_MARKER: &str = "generated by .harmony";

// `write_harmony_skin` (VellumFE, `#[cfg(feature = "gui")]`) is NOT ported: it
// PNG-encodes through the `image` crate, which this crate must not depend on.
// It stays in VellumFE, built on `pool_dirs::skins_dir`, `write_atomic` and
// `HARMONY_SKIN_MARKER` from here.

/// Resolve a manifest image path to a filesystem path. Absolute paths are
/// taken as-is; relative paths resolve against the skin directory first,
/// then the shared image pool (`~/.vellum-fe/global/images/`) — so a skin
/// can reference pooled art ("icons/rogue.png", "frames/brass.png",
/// "dolls/human.png", ...) without copying it. When neither location has
/// the file, the skin-local path is returned so the caller's error names
/// the natural spot.
pub fn resolve_image_path(root: &Path, image: &str) -> PathBuf {
    let raw = Path::new(image);
    if raw.is_absolute() {
        return raw.to_path_buf();
    }
    let local = root.join(raw);
    if local.is_file() {
        return local;
    }
    if let Ok(pool) = crate::pool_dirs::global_images_dir() {
        let pooled = pool.join(raw);
        if pooled.is_file() {
            return pooled;
        }
    }
    local
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_anchors_cover_every_part_within_unit_bounds() {
        assert_eq!(DOLL_PARTS.len(), 14);
        for (key, _, anchor) in DOLL_PARTS {
            let resolved = default_doll_anchor(key).unwrap();
            assert!(
                (0.0..=1.0).contains(&resolved[0]) && (0.0..=1.0).contains(&resolved[1]),
                "{key} anchor out of bounds"
            );
            assert_eq!(resolved, *anchor);
        }
        // Case-insensitive on the protocol key, None for unknown parts.
        assert!(default_doll_anchor("LEFTARM").is_some());
        assert!(default_doll_anchor("tail").is_none());
    }

    #[test]
    fn severity_levels_map_injuries_then_scars() {
        assert_eq!(severity_level_from_key("healthy"), Some(0));
        assert_eq!(severity_level_from_key("injury1"), Some(1));
        assert_eq!(severity_level_from_key("injury3"), Some(3));
        assert_eq!(severity_level_from_key("scar1"), Some(4));
        assert_eq!(severity_level_from_key("scar3"), Some(6));
        assert_eq!(severity_level_from_key("injury4"), None);
        assert_eq!(severity_level_from_key("base"), None);
        // Round-trip: every level 0-6 maps to a key and back.
        for level in 0..=6u8 {
            let key = severity_key_from_level(level).unwrap();
            assert_eq!(severity_level_from_key(key), Some(level));
        }
        assert_eq!(severity_key_from_level(7), None);
    }

}

#[cfg(test)]
mod doll_parts_tests {
    /// The 2D doll's anchors are for the body's parts, in their order.
    #[test]
    fn doll_parts_are_the_bodys_parts() {
        let ours: Vec<(&str, &str)> = super::DOLL_PARTS.iter().map(|(key, name, _)| (*key, *name)).collect();
        assert_eq!(ours, crate::body::PARTS.to_vec());
    }
}
