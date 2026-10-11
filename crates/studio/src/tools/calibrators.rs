//! The calibrator tabs: Frames, Dolls, Creatures, Scenery. Each hosts a
//! `gs_calibrators` state through its `ui_contents(ui)` body, so the
//! calibrator fills the tab. The floating `ui(ctx)` form is never used here.
//!
//! A state is opened lazily the first time its tab is shown and then KEPT:
//! switching tabs never drops it (unsaved calibration work survives). It is
//! re-opened only by "Rescan pool", by re-targeting the doll picker, and
//! when the pool directory changes. The rules (what an outcome asks of the
//! app, the doll pick, the rescan confirmation) are in `crate::calib`.

use gs_calibrators::{
    doll_calibration, CalibrationOutcome, CreatureCalibrationState, DollCalibrationState, DollCalibrationTarget,
    FrameCalibrationState, SceneryCalibrationState,
};

use crate::calib::{CalibKind, DollPicker, HostEffects, RescanGate};
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
    fn unsaved(&self) -> Option<bool> {
        Some(self.has_unsaved_changes())
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

impl Hosted for DollCalibrationState {
    fn body(&mut self, ui: &mut egui::Ui) -> CalibrationOutcome {
        self.ui_contents(ui)
    }
    fn unsaved(&self) -> Option<bool> {
        Some(self.has_unsaved_changes())
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
    dolls: Slot<DollCalibrationState>,
    doll_picker: DollPicker,
    log: StatusLog,
}

const NO_DOLLS: &str = "No doll pictures in the pool (global/images/dolls/). Drop PNGs there, then press Rescan pool.";

/// Open the doll calibrator on the picked doll.
fn open_doll(picker: &DollPicker) -> (Option<DollCalibrationState>, CalibrationOutcome) {
    match picker.current() {
        Some(path) => DollCalibrationState::open(Some(DollCalibrationTarget::from_pool_path(path))),
        None => (None, CalibrationOutcome { messages: vec![NO_DOLLS.to_string()], ..Default::default() }),
    }
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
        creature_field::pool::invalidate_cache();
        slot.reopen(log, open);
    }
    slot.body(ui, log)
}

impl CalibratorsTool {
    pub fn new() -> CalibratorsTool {
        CalibratorsTool::default()
    }

    /// The pool directory changed (creature_field already points at the new one):
    /// drop every state; each tab re-opens against the new pool when shown.
    pub fn pool_changed(&mut self) {
        self.frames.forget();
        self.creatures.forget();
        self.scenery.forget();
        self.dolls.forget();
        self.doll_picker = DollPicker::default();
        self.log.push("Pool changed: calibrators re-open against the new pool");
    }

    /// One line for the smoke-run log.
    pub fn summary(&self) -> String {
        format!(
            "frames={} dolls={} of {} creatures={} scenery={}, {} status lines (last: {:?})",
            self.frames.state.is_some(),
            self.dolls.state.is_some(),
            self.doll_picker.choices().len(),
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
                CalibKind::Dolls => self.dolls_body(ui, &mut effects),
            }
        });
        effects
    }

    /// The Dolls tab: a picker over the pool's doll pictures, the
    /// calibrator on the picked one.
    fn dolls_body(&mut self, ui: &mut egui::Ui, effects: &mut HostEffects) {
        let CalibratorsTool { dolls, doll_picker, log, .. } = self;
        // Before the strip draws, so the picker is never empty on frame 1.
        if !dolls.opened {
            doll_picker.refresh(doll_calibration::pool_dolls());
            dolls.reopen(log, || open_doll(doll_picker));
        }
        let mut rescan = false;
        let mut retarget = false;
        ui.horizontal_wrapped(|ui| {
            ui.label("Doll:");
            let current = doll_picker.current().unwrap_or("(no pool dolls)").to_string();
            let mut picked = None;
            egui::ComboBox::from_id_salt("doll_pick").selected_text(current).width(220.0).show_ui(ui, |ui| {
                for (index, path) in doll_picker.choices().iter().enumerate() {
                    if ui.selectable_label(doll_picker.pick() == index, path).clicked() {
                        picked = Some(index);
                    }
                }
            });
            if let Some(index) = picked {
                retarget = doll_picker.select(index);
            }
            rescan = dolls.rescan_controls(ui);
        });
        ui.separator();
        if rescan {
            creature_field::pool::invalidate_cache();
            doll_picker.refresh(doll_calibration::pool_dolls());
        }
        if rescan || retarget {
            // Re-target = drop the state and open again.
            dolls.reopen(log, || open_doll(doll_picker));
        }
        let reload = dolls.body(ui, log);
        effects.absorb(CalibKind::Dolls, reload);
    }
}
