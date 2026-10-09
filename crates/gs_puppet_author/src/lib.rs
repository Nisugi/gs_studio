//! gs_puppet_author: building gs_puppet's puppets -- authoring, gs-studio's
//! alone; a game client loads baked puppets with `gs_puppet` and never
//! builds one (docs/crate-split-plan.md).
//!
//! ```text
//! use gs_puppet_author::prelude::*;
//! Puppet::build(&rig, &outlines)?      rasterise + measure, once (editor / bake time)
//! puppet.to_baked_toml()?              -> <family>.puppet.toml for a game client (gs_puppet)
//! prefill(&puppet, &layout, &paintings) a texture sheet from front/side/back paintings
//! bound.measured(&puppet)              a modelled body's rings measured from its mesh
//! paint(&painter, offset, &prims)      an editor's preview, the only egui here
//! ```
//!
//! The builder adds to gs_puppet's types through traits ([`prelude`]):
//! [`build::Build`], [`hung::HangDrawn`], [`mesh_measure::MeasureMesh`],
//! [`uv_sheet::UvSheet`].

pub mod build;
pub mod depth;
pub mod fit;
pub mod hung;
pub mod measure;
pub mod mesh_measure;
pub mod paint;
pub mod prefill;
pub mod uv_sheet;

/// The traits that add building to gs_puppet's types: in scope, the calls
/// read as they did when they were gs_puppet's own (`Puppet::build(..)`).
pub mod prelude {
    pub use crate::build::Build;
    pub use crate::hung::HangDrawn;
    pub use crate::mesh_measure::MeasureMesh;
    pub use crate::uv_sheet::UvSheet;
}

pub use build::{plan_shift, side_for_plan, turned_head_up, view_of, MASK_PX};
pub use depth::{depth_of, guess_depths, guess_depths_mask, joints3, snap_limb_joints, DepthGuess, GuessSource};
pub use fit::{rest_iou, rest_prims};
pub use paint::paint;
pub use prefill::{depth_guide, dressed_silhouette, normal_guide, prefill, prefill_normals, project_view, puppet_eyes, side_silhouette, PaintView, Painting, Paintings, GRAZING};
pub use uv_sheet::{sheet_side_for, silhouette_template};

#[cfg(test)]
mod tests;
