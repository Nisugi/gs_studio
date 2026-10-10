//! The calibrator tabs: Frames, Creatures, Scenery. Each hosts a
//! `gs_calibrators` state through its `ui_contents(ui)` body, so the
//! calibrator fills the tab. The floating `ui(ctx)` form is never used here.
//!
//! A state is opened lazily the first time its tab is shown and then KEPT:
//! switching tabs never drops it (unsaved calibration work survives). It is
//! re-opened only by "Rescan pool" and when the pool directory changes. The
//! rules (what an outcome asks of the app, the rescan confirmation) are in
//! `crate::calib`.

use gs_calibrators::{CalibrationOutcome, CreatureCalibrationState, FrameCalibrationState, SceneryCalibrationState};

use crate::calib::{CalibKind, HostEffects, RescanGate};
use crate::status_log::StatusLog;

/// What the host needs from a calibrator state.
trait Hosted {
    fn body(&mut self, ui: &mut egui::Ui) -> CalibrationOutcome;
    /// `Some` only for a calibrator that exposes an unsaved-edits flag.
    fn unsaved(&self) -> Option<bool> {
        None
    }
}

impl Hosted for FrameCalibrationState {
    fn body(&mut self, ui: &mut egui::Ui) -> CalibrationOutcome {
        self.ui_contents(ui)
    }
}

impl Hosted for CreatureCalibrationState {
    fn body(&mut self, ui: &mut egui::Ui) -> CalibrationOutcome {
        self.ui_contents(ui)
    }
}

impl Hosted for SceneryCalibrationState {
    fn body(&mut self, ui: &mut egui::Ui) -> CalibrationOutcome {
        self.ui_contents(ui)
    }
}

/// One calibrator's place in the app.
struct Slot<S> {
    state: Option<S>,
    /// False until the tab is first shown (and again after the pool moves).
    opened: bool,
    /// Why there is no state: the message `open` (or a close) gave.
    note: Option<String>,
    gate: RescanGate,
}

impl<S> Default for Slot<S> {
    fn default() -> Self {
        Slot { state: None, opened: false, note: None, gate: RescanGate::default() }
    }
}

impl<S: Hosted> Slot<S> {
    fn ensure_open(&mut self, log: &mut StatusLog, open: impl FnOnce() -> (Option<S>, CalibrationOutcome)) {
        if !self.opened {
            self.reopen(log, open);
        }
    }

    /// Drop whatever is there and open afresh.
    fn reopen(&mut self, log: &mut StatusLog, open: impl FnOnce() -> (Option<S>, CalibrationOutcome)) {
        let (state, outcome) = open();
        self.note = if state.is_none() { outcome.messages.last().cloned() } else { None };
        self.state = state;
        self.opened = true;
        self.gate.reset();
        for message in outcome.messages {
            log.push(message);
        }
    }

    /// The pool moved: forget everything; the tab re-opens on next entry.
    fn forget(&mut self) {
        *self = Slot::default();
    }

    /// The "Rescan pool" button and, when the calibrator reports unsaved
    /// edits, its confirmation. True = re-open now.
    fn rescan_controls(&mut self, ui: &mut egui::Ui) -> bool {
        let mut go = false;
        if ui
            .button("Rescan pool")
            .on_hover_text("Re-read the pool directory and re-open this calibrator (picks up added or removed images)")
            .clicked()
        {
            go = self.gate.request(self.state.as_ref().and_then(Hosted::unsaved));
        }
        if self.gate.asking() {
            ui.colored_label(ui.visuals().warn_fg_color, "Unsaved edits will be lost.");
            if ui.button("Discard and rescan").clicked() {
                go = self.gate.answer(true);
            }
            if ui.button("Cancel").clicked() {
                self.gate.answer(false);
            }
        }
        go
    }

    /// Draw the calibrator (or why there is none). Returns its `reload_art`.
    fn body(&mut self, ui: &mut egui::Ui, log: &mut StatusLog) -> bool {
        let Some(state) = self.state.as_mut() else {
            if let Some(note) = &self.note {
                ui.label(note);
            }
            ui.weak("Nothing to calibrate here yet. After adding images to the pool, press Rescan pool.");
            return false;
        };
        let outcome = state.body(ui);
        if outcome.closed {
            // a calibrator that closed itself in the tab
            self.note = outcome.messages.last().cloned();
            self.state = None;
            self.gate.reset();
        }
        for message in outcome.messages {
            log.push(message);
        }
        outcome.reload_art
    }
}

#[derive(Default)]
pub struct CalibratorsTool {
    frames: Slot<FrameCalibrationState>,
    creatures: Slot<CreatureCalibrationState>,
    scenery: Slot<SceneryCalibrationState>,
    log: StatusLog,
}

/// A calibrator with nothing to choose before it opens: the strip is just
/// the rescan button.
fn simple_tab<S: Hosted>(
    slot: &mut Slot<S>,
    ui: &mut egui::Ui,
    log: &mut StatusLog,
    open: fn() -> (Option<S>, CalibrationOutcome),
) -> bool {
    slot.ensure_open(log, open);
    let mut rescan = false;
    ui.horizontal_wrapped(|ui| rescan = slot.rescan_controls(ui));
    ui.separator();
    if rescan {
        gs_field::pool::invalidate_cache();
        slot.reopen(log, open);
    }
    slot.body(ui, log)
}

impl CalibratorsTool {
    pub fn new() -> CalibratorsTool {
        CalibratorsTool::default()
    }

    /// The pool directory changed (gs_field already points at the new one):
    /// drop every state; each tab re-opens against the new pool when shown.
    pub fn pool_changed(&mut self) {
        self.frames.forget();
        self.creatures.forget();
        self.scenery.forget();
        self.log.push("Pool changed: calibrators re-open against the new pool");
    }

    /// One line for the smoke-run log.
    pub fn summary(&self) -> String {
        format!(
            "frames={} creatures={} scenery={}, {} status lines (last: {:?})",
            self.frames.state.is_some(),
            self.creatures.state.is_some(),
            self.scenery.state.is_some(),
            self.log.lines().len(),
            self.log.lines().last()
        )
    }

    pub fn status(&self, ui: &mut egui::Ui) {
        let lines = self.log.lines();
        match lines.last() {
            Some(message) => ui.label(message),
            None => ui.weak("Ready"),
        }
        .on_hover_ui(|ui| {
            for message in lines {
                ui.label(message);
            }
        });
    }

    /// Draw one calibrator tab.
    pub fn ui(&mut self, ui: &mut egui::Ui, tab: CalibKind) -> HostEffects {
        let mut effects = HostEffects::default();
        egui::CentralPanel::default().show(ui, |ui| {
            let log = &mut self.log;
            match tab {
                CalibKind::Frames => {
                    let reload = simple_tab(&mut self.frames, ui, log, || FrameCalibrationState::open(None));
                    effects.absorb(tab, reload);
                }
                CalibKind::Creatures => {
                    let reload = simple_tab(&mut self.creatures, ui, log, CreatureCalibrationState::open);
                    effects.absorb(tab, reload);
                }
                CalibKind::Scenery => {
                    let reload = simple_tab(&mut self.scenery, ui, log, SceneryCalibrationState::open);
                    effects.absorb(tab, reload);
                }
            }
        });
        effects
    }
}
