//! The GemStone IV art calibrators as embeddable egui windows.
//!
//! Each calibrator is a self-contained state struct with
//! `open(..) -> (Option<Self>, CalibrationOutcome)` and
//! `ui(&mut self, &egui::Context) -> CalibrationOutcome`. The host keeps the
//! state in an `Option`, surfaces `outcome.messages`, re-resolves art when
//! `outcome.reload_art` is set, and drops the state when `outcome.closed`.
//!
//! Ported from VellumFE (GPL-3.0-or-later) @60c84a79,
//! `src/frontend/gui/app/editors/`. See PORTING.md.

// Style lints that fire on the ported code as written; the code is kept
// textually close to VellumFE so the diff stays reviewable.
#![allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::items_after_test_module,
    clippy::collapsible_if,
    // The window bodies stay closures called in place (`(|ui| { .. })(ui)`):
    // they `return` early, and the text stays diffable against VellumFE.
    clippy::redundant_closure_call,
    clippy::doc_lazy_continuation,
    clippy::obfuscated_if_else,
    clippy::manual_clamp,
    clippy::unnecessary_lazy_evaluations
)]

pub mod color;
pub mod condition_ui;
pub mod creature_calibration;
pub mod doll_calibration;
pub mod frame_calibration;
pub mod frame_recipe;
pub mod frame_template;
pub mod scenery_calibration;
pub mod skin_paint;

pub use creature_calibration::CreatureCalibrationState;
pub use doll_calibration::{DollCalibrationState, DollCalibrationTarget};
pub use frame_calibration::FrameCalibrationState;
pub use scenery_calibration::SceneryCalibrationState;

/// What a hosted calibrator did this frame: system-message text for the
/// host to surface, whether saved art must be re-resolved, and whether the
/// user dismissed the window. Lets the calibrators run under both the game
/// GUI and Vellum Studio.
#[derive(Default)]
pub struct CalibrationOutcome {
    pub messages: Vec<String>,
    pub reload_art: bool,
    pub closed: bool,
    /// *Export pack*: what the tab holds, saved, to pack -- the Frames
    /// tab's recipe, the Dolls tab's picture (its stem). The host knows
    /// where packs go (gs-studio: `packs::export_frame`, `export_doll`).
    pub export_pack: Option<String>,
}
