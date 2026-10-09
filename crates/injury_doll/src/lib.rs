//! The 3D injury doll a game client draws (docs/crate-split-plan.md).
//!
//! A client (Hydra) depends on this crate alone. It tells the doll what the
//! character is ([`state::DollState::from_game`]), picks a figure from the
//! playable races installed in its data folder ([`roster::races`]), lights
//! it ([`sky`]) and draws it ([`view::paint_in_view`]) with the GPU pass set
//! up once ([`gpu::init`]).
//!
//! For now every item here is re-exported from `gs_field` / `gs_field_egui`
//! under the path it keeps: the code moves in behind these paths (the
//! split's S6 and S7), and the client's imports do not change again.

/// The doll's figures: the playable races' modelled bodies installed in the
/// data folder (`global/bodies/<form>/`, each with its `doll.toml`).
pub mod roster {
    pub use gs_field::doll_roster::{arms_hang, figure, races, races_of, read_figure, roster, scan, DollFigure, DollRace, FigureDraw, FigureKind};
}

/// The light and background the doll stands in: a steady display, a day,
/// or the game's own sky at an hour.
pub mod sky {
    pub use gs_field::doll_sky::{display, doll_sky, game_doll_sky, game_doll_sky_at, hex_linear, sun_arc, DollSky, DollSkyLight};
    pub use gs_field::lighting::Sky;
}

/// What the character is, in the game's words: wounds and scars by part,
/// and the statuses it is under.
pub mod state {
    pub use gs_field::doll_state::{DollState, Status, Statuses, PRECEDENCE};
}

/// What the doll shares with the rest of gs_studio's runtime: where its
/// files are (a client sets its data folder), and the game's clock.
pub mod base {
    pub use gs_field::{data_dirs, elanthian_time};
}

/// Drawing the doll: its key from a state, painted into a rect, each doll
/// shown at once its own view of the GPU pass.
pub mod view {
    pub use gs_field_egui::creature_puppet::{PuppetCache as DollCache, PuppetKey as DollKey};
    pub use gs_field_egui::doll_view::{key_for, paint, paint_in_view, paint_state, DOLL_TURN_DEG, DOLL_VIEW};
}

/// The GPU pass the doll is drawn by: set up once as the window opens; a
/// doll drawn without it is unlit on grey.
pub mod gpu {
    pub use gs_field_egui::field_gpu::{available, init};
}
