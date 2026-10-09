//! gs_puppet: a poseable 3D figure from a rig and two silhouettes, drawn
//! with plain 2D shapes. No modelling, no GPU 3D, no engine.
//!
//! A creature family has two silhouettes of the same figure at the same
//! scale (feet, ground at y = 0): `<family>-front` and `<family>` (side).
//! An artist clicks named joints onto ONE of them (`rig::Rig`). From the rig
//! alone comes the skeleton (bones -> segments, `parent` -> the tree); from
//! the two silhouettes comes the body: along every segment a run of measured
//! elliptical cross-sections ("stations"), so a belly, a chest, a tapering
//! limb or a muzzle is whatever the family's own art says it is.
//!
//! # World convention (the only one)
//!
//! * `x` = the creature's LEFT-right, + toward its left;
//! * `y` = up, the ground is y = 0;
//! * `z` = front-back, the creature FACES +z.
//!
//! Units are feet. A FRONT rig authors `(x, y)` -- the creature faces the
//! viewer, so its left is screen-right and `_r` joints have negative x --
//! and needs z, read from the side silhouette (`z` = that silhouette's x:
//! side views face screen-right). A SIDE rig authors `(z, y)` and needs x,
//! read from the front silhouette; `_r` is the near limb, at negative x.
//! Silhouette coordinates are used as they are (both views are centred on
//! x = 0), so a `[depth]` value in a rig file is a position an editor can
//! draw straight onto the other silhouette.
//!
//! # The pieces
//!
//! ```text
//! Puppet::build(&rig, &outlines)?      rasterise + measure, once (editor / bake time)
//! puppet.to_baked_toml()?              -> <family>.puppet.toml for a game client
//! Puppet::from_baked_toml(text)?       the client's way in: no silhouettes, no rig
//! PoseSet::load(dir, body_type)?       data/poses/<body_type>.toml (or PoseSet::shipped)
//! let posed = puppet.pose(&pose);      FK + rest the lowest surface on y = 0
//! posed.bounds()                       world AABB, for placement
//! posed.draw_list(&project, &look)     egui-free primitives, in paint order
//! posed.skin(&project, &look)          the same body as one smooth surface, RGBA
//! paint(&painter, offset, &prims)      the only egui in the crate
//! ```
//!
//! Plain data and functions throughout; everything but `paint` is egui-free.

mod afflict;
mod blind;
mod calm;
mod daze;
mod exposed;
mod fetter;
mod heat;
mod hush;
pub mod baked;
mod blood;
pub mod camera;
pub mod depth;
pub mod draw;
pub mod fall;
pub mod fit;
mod frost;
mod ghost;
mod possess;
mod shade;
pub mod hang;
pub mod hung;
pub mod joints;
pub mod layers;
pub mod math;
pub mod measure;
pub mod mesh;
pub mod outfit;
pub mod paint;
pub mod pose;
pub mod prefill;
pub mod prop;
pub mod build;
pub mod model;
pub mod raster;
pub mod shadow;
pub mod skin;
mod skin_field;
pub mod splat;
mod status_layers;
mod wraps;
pub mod uv;

/// Test inputs for this crate's tests and the crates over it (their
/// dev-dependencies turn on the `fixtures` feature): a baked humanoid
/// puppet, frozen as published, so a test needs no data pool, no rig and
/// no other crate's files.
#[cfg(any(test, feature = "fixtures"))]
pub mod fixtures {
    /// The humanoid puppet as `gs_field` shipped it (2026-10-09).
    pub const HUMANOID: &str = include_str!("../tests/data/humanoid.puppet.toml");
}
mod wounds;
pub use wounds::MarksSheet;

pub use baked::{BakedEye, BakedPuppet, BakedSegment, BAKED_VERSION};
pub use camera::Camera;
pub use depth::{depth_of, guess_depths, guess_depths_mask, joints3, snap_limb_joints, DepthGuess, GuessSource};
pub use fall::Fall;
pub use draw::{ground_grid, Look, Prim, Rgba, Shape, BODY_DEAD, BODY_DEFAULT, WOUND_COLORS};
pub use fit::{rest_iou, rest_prims};
pub use outfit::{Cut, Hang, Kit, Outfit, Outfits, Piece, SHIPPED_KIT};
pub use paint::paint;
pub use prefill::prefill_normals;
pub use uv::bump_sheet;
pub use pose::{flinch_envelope, BonePose, FamilyPoses, Idle, IdleBone, Limit, Pose, PoseOverride, PoseSet, Posed, FLINCH_REGIONS, FLINCH_SECS, SHIPPED_POSES};
pub use prefill::{depth_guide, dressed_silhouette, normal_guide, prefill, project_view, puppet_eyes, side_silhouette, PaintView, Painting, Paintings, GRAZING};
pub use prop::{Grip, Hand, Prop, PropSet, Weapon, SHIPPED_PROPS};
pub use shadow::{Shadow, ShadowCaster};
pub use skin::{Beam, Effects, Lamp, SceneLight, Skin, SkinCorner, SkinMesh, SkinShading, SkinSurface, SKIN_SS};
pub use uv::{sheet_side_for, silhouette_template, Texture, UvLayout, LAYOUT_KEYWORD, LAYOUT_SLACK_PX, PACKED_ARC_SAMPLES, SHEET_SIDE, SHEET_SIDES};
pub use build::{plan_shift, side_for_plan, turned_head_up, MASK_PX};
pub use model::{Eye, Kind, Puppet, PuppetError, Segment, Station, View};

#[cfg(test)]
mod pose_tests;
#[cfg(test)]
mod skin_tests;
#[cfg(test)]
mod tests;
