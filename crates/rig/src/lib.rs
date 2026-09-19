//! Creature rigs: the file format, the body-type templates a rig starts
//! from, and baking a rig against a silhouette into the files a game client
//! reads. No UI in here -- the studio app and any command-line tool share it.
//!
//! Coordinates are FEET, x centred, y up, ground at 0 (the outline
//! extractor's convention). Front views face the viewer, so the creature's
//! right (`_r`) is screen-left, at negative x.

mod bake;
mod model;
mod outlines;
mod templates;

pub use bake::{
    bake, bake_mask, decode_png, encode_png, write_baked, Baked, BakedBone, BakedMeta, BoneMeta,
    DecodedMap, MapSize, EYE_MIN_PX_AT_512, EYE_RADIUS_SHARE,
};
pub use model::{Anchors, Eyes, HierarchyIssue, Rig, RigBone, RigError, View, RIG_VERSION};
pub use outlines::{OutlineEntry, Outlines};
pub use templates::{
    family_default, mirror_name, template, templates, AnchorDef, BoneDef, FamilyDefault, Template,
    AXES, FAMILIES,
};

pub use rig_bake::{Pt, PARTS};
