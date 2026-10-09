//! The 3D injury doll a game client draws (docs/crate-split-plan.md).
//!
//! A client (Hydra) depends on this crate alone. It tells the doll what the
//! character is ([`state::DollState::from_game`]), picks a figure from the
//! playable races installed in its data folder ([`roster::races`]), lights
//! it ([`sky`]) and draws it ([`view::paint_in_view`]) with the GPU pass set
//! up once ([`gpu::init`]).
//!
//! The doll's own code is here: its figures ([`roster`]) and where its
//! cache reads them ([`source`]), its light ([`sky`]), the character's
//! state ([`state`]) and the view ([`view`]).
//! What it shares with the creature field -- the base data, the puppet
//! cache, the GPU pass -- it reaches in `gs_field` / `gs_field_egui` until
//! those leave for crates of their own (the split's S7); the paths here
//! stay.

pub mod roster;
pub mod sky;
pub mod source;
pub mod state;
pub mod view;

/// What the doll shares with the rest of gs_studio's runtime: where its
/// files are (a client sets its data folder), and the game's clock.
pub mod base {
    pub use gs_field::{data_dirs, elanthian_time};
}

/// The GPU pass the doll is drawn by: set up once as the window opens; a
/// doll drawn without it is unlit on grey.
pub mod gpu {
    pub use gs_field_egui::field_gpu::{available, init};
}
