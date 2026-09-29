//! gs_field_egui — draws the GemStone IV creature field with egui.
//!
//! `gs_field` lays the field out (solver, card resolution, asset formats);
//! this crate turns that into pixels: [`image_store`] decodes art into egui
//! textures, [`creature_art`] is the lazily filled creature-art cache and its
//! per-frame prepare passes, and [`creature_field`] is the stage renderer.
//!
//! Per frame, a host does: `CreatureArtStore::prepare_creature_art` (and
//! `prepare_scene_art` when it has a scene) -> `gs_field::cards::sync_field`
//! -> [`render_creature_field_content`].
//!
//! Ported from VellumFE (GPL-3.0-or-later) at commit 60c84a79. `PORTING.md`
//! next to this crate's `Cargo.toml` maps every item back to its source and
//! lists each deliberate difference.

// Ported code is kept byte-close to VellumFE so the switch-over diff stays
// reviewable; these lints fire on the originals' style, not on bugs.
#![allow(
    // paint_creature_card / paint_card_overlays take what they took.
    clippy::too_many_arguments,
    // CreatureFieldView's boxed closures.
    clippy::type_complexity,
    // creature_field.rs: `ellipse_points` sits after `mod stage_map_tests`.
    clippy::items_after_test_module,
    // creature_field.rs: `x % n == 0` kept as written.
    clippy::manual_is_multiple_of
)]

pub mod creature_art;
/// Addition since the port (not in VellumFE): the no-art creature doll.
pub mod creature_doll;
pub mod creature_field;
pub mod creature_indicators;
pub mod creature_light;
/// Addition since the port (not in VellumFE): the no-art creature puppet,
/// preferred over the flat doll whenever a baked puppet resolves.
pub mod creature_puppet;
/// Addition since the port (not in VellumFE): the injury doll -- one
/// puppet in a window of its own, lit by the sky round the player.
pub mod doll_view;
/// Addition since the port (not in VellumFE): the v2 lighting pass -- local
/// lights and their shadows on the GPU, over each layer's v1 result.
pub mod field_gpu;
/// Addition since the port (not in VellumFE): [`field_gpu`] on the stage.
pub mod field_gpu_stage;
/// Addition since the port (not in VellumFE): the puppet's smooth skin
/// drawn on the GPU into a [`field_gpu`] layer.
pub mod skin_gpu;
pub mod field_lights;
pub mod field_occluders;
pub mod field_weather;
pub mod image_store;

pub use creature_art::{
    CreatureArt, CreatureArtCache, CreatureArtStore, SharedCreatureArt, WantedCreature,
};
pub use creature_field::{
    creature_field_stage_pos, render_creature_field_content, CreatureFieldClick,
    CreatureFieldView,
};
