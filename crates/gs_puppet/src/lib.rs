//! gs_puppet -- PROTOTYPE.
//!
//! A creature family has two silhouettes of the same figure at the same
//! scale (feet, ground at y = 0): a FRONT view and a SIDE view. An artist
//! clicks named joints onto one of them (`rig::Rig`). From that this crate
//! builds a genuinely 3D, poseable figure: every joint gets (x, y, z), every
//! bone segment becomes a rounded capsule whose width comes from the front
//! silhouette and depth from the side one, poses are 3D joint rotations, and
//! the figure is drawn with flat circles and quads through a perspective
//! camera. No GPU 3D.
//!
//! Axes: x = the creature's left (screen right in a front view), y = up,
//! z = the creature's front. Everything but `paint` is egui-free.

pub mod camera;
pub mod draw;
pub mod math;
pub mod measure;
pub mod paint;
pub mod pose;
pub mod raster;
pub mod skeleton;

pub use camera::Camera;
pub use draw::{build_draw_list, ground_grid, ground_shadow, Prim, Shape, Style};
pub use pose::{solve, Pose, Posed, PRESETS};
pub use skeleton::{guess_depths, joints3, BoneRule, DepthGuess, GuessSource, Segment, Skeleton, BIPED};

#[cfg(test)]
mod tests;
