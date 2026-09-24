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

pub mod baked;
pub mod camera;
pub mod depth;
pub mod draw;
pub mod fit;
pub mod math;
pub mod measure;
pub mod paint;
pub mod pose;
pub mod prefill;
pub mod puppet;
pub mod raster;
pub mod skin;
pub mod uv;

pub use baked::{BakedEye, BakedPuppet, BakedSegment, BAKED_VERSION};
pub use camera::Camera;
pub use depth::{depth_of, guess_depths, guess_depths_mask, joints3, snap_limb_joints, DepthGuess, GuessSource};
pub use draw::{ground_grid, Look, Prim, Rgba, Shape, BODY_DEAD, BODY_DEFAULT, WOUND_COLORS};
pub use fit::{rest_iou, rest_prims};
pub use paint::paint;
pub use pose::{BonePose, FamilyPoses, Pose, PoseOverride, PoseSet, Posed, SHIPPED_POSES};
pub use prefill::{prefill, Painting, Paintings};
pub use skin::{Effects, Skin};
pub use uv::{Texture, UvLayout};
pub use puppet::{Eye, Kind, Puppet, PuppetError, Segment, Station, MASK_PX};

#[cfg(test)]
mod pose_tests;
#[cfg(test)]
mod skin_tests;
#[cfg(test)]
mod tests;
