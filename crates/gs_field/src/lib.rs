//! gs_field — the GemStone IV "creature field" without the drawing.
//!
//! The creature field is a pseudo-3D stage where a room's creatures stand as
//! cards. This crate holds everything a client needs to lay that out and
//! describe it: the wire-side creature types, card/art resolution, the
//! placement solver and its camera, the shared card geometry, the bundled
//! bestiary facts that size a card, and the on-disk asset formats (skins,
//! scenes, the shared image pool and its sidecars, PNG-embedded metadata).
//! No egui, no image decoding, no desktop-only APIs — it builds for Android.
//!
//! Ported from VellumFE (GPL-3.0-or-later) at commit 60c84a79. `PORTING.md`
//! next to this crate's `Cargo.toml` maps every item back to its source and
//! lists each deliberate difference.
//!
//! Condition EVALUATION is not here: [`condition`] is the vocabulary (types
//! and serde) only. Hosts pass an evaluator closure to
//! [`cards::resolve_card_with`].

// Ported code is kept byte-close to VellumFE so the switch-over diff stays
// reviewable; these lints fire on the originals' style, not on bugs.
#![allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::doc_lazy_continuation,
    clippy::field_reassign_with_default,
    clippy::int_plus_one,
    clippy::needless_borrow,
    clippy::unnecessary_sort_by
)]

pub mod bestiary;
pub mod cards;
pub mod condition;
pub mod creature;
pub mod data_dirs;
pub mod doll_roster;
pub mod doll_sky;
pub mod doll_state;
pub mod dolls;
pub mod elanthian_time;
pub mod field_config;
pub mod geometry;
pub mod naming;
pub mod png_meta;
pub mod pool;
pub mod puppets;
pub mod lighting;
pub mod sky_light;
pub mod scenes;
pub mod skins;
pub mod sky;
pub mod stars;
pub mod walk;
pub mod solver;

pub use condition::Condition;
pub use creature::{Creature, CreatureFlags};
