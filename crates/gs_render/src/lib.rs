//! gs_render -- what the injury doll and the creature field both draw with
//! (docs/crate-split-plan.md, S7): the puppet cache ([`puppet_cache`], over
//! a source of puppets each host gives it), the v2 GPU lighting pass
//! ([`field_gpu`]), the GPU skins and their relight ([`skin_gpu`],
//! [`relight_gpu`]), and a figure as the pass's layer image
//! ([`figure_image`]).
//!
//! It reaches neither the doll nor the field. The modules came out of
//! `gs_field_egui`, which re-exports them under their old paths; their
//! history before the split is in `gs_field_egui/PORTING.md`.

// The allowances these modules had in gs_field_egui.
#![allow(clippy::too_many_arguments, clippy::type_complexity)]

pub mod field_gpu;
pub mod figure_image;
pub mod puppet_cache;
pub mod relight_gpu;
pub mod skin_gpu;
