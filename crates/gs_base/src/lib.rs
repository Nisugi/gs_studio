//! gs_base -- what the injury doll and the creature field both stand on
//! (docs/crate-split-plan.md, S7): where the data is ([`data_dirs`]), the
//! game's clock ([`elanthian_time`]) and sky ([`sky`], [`stars`]), the sky
//! as the lights read it ([`sky_light`]), the body's parts ([`body`]), the
//! statuses ([`status`]), PNG metadata ([`png_meta`]).
//!
//! It reaches neither the doll nor the field, and draws nothing: no egui.
//! The modules came out of `gs_field`, which re-exports them under their
//! old paths; their history before the split is in `gs_field/PORTING.md`.

// Ported code is kept byte-close to VellumFE (gs_field's allowances).
#![allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::doc_lazy_continuation,
    clippy::field_reassign_with_default,
    clippy::int_plus_one,
    clippy::needless_borrow,
    clippy::unnecessary_sort_by
)]

pub mod body;
pub mod data_dirs;
pub mod elanthian_time;
pub mod png_meta;
pub mod sky;
pub mod sky_light;
pub mod stars;
pub mod status;
