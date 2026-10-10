//! The calibrator tabs' rules, without egui: which calibrator a tab hosts,
//! what a calibrator's outcome asks of the rest of the app, and the
//! "Rescan pool" confirmation. The calibrators
//! themselves live in `gs_calibrators`; `tools/calibrators.rs` hosts them.

/// One hosted calibrator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalibKind {
    Frames,
    Creatures,
    Scenery,
}

/// What this frame's calibrator outcomes ask of the rest of the app.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HostEffects {
    /// A calibrator saved: art and sidecars changed on disk. The stage's
    /// art store must reload and gs_field's calibration store re-seed
    /// (VellumFE: `drain_outcome` -> `skin_state.force_reload()`).
    pub reload_art: bool,
    /// The SCENERY calibrator saved: prop exclusion spans read the sidecar,
    /// so the stage's solver obstacles are stale too (VellumFE: the scenery
    /// tail's `stage.obstacle_scene = None` / `reload_creature_field_files`).
    pub invalidate_obstacles: bool,
}

impl HostEffects {
    /// Fold one calibrator's `reload_art` in.
    pub fn absorb(&mut self, kind: CalibKind, reload_art: bool) {
        if reload_art {
            self.reload_art = true;
            if kind == CalibKind::Scenery {
                self.invalidate_obstacles = true;
            }
        }
    }
}

/// "Rescan pool" re-opens a calibrator, which discards its unsaved edits.
/// A calibrator that can say it has some gets a confirmation first.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RescanGate {
    asking: bool,
}

impl RescanGate {
    pub fn asking(&self) -> bool {
        self.asking
    }

    /// The button was pressed. `unsaved` is the calibrator's own answer
    /// (`None`: it has no such flag, so there is nothing to ask about).
    /// True = rescan now.
    pub fn request(&mut self, unsaved: Option<bool>) -> bool {
        self.asking = unsaved == Some(true);
        !self.asking
    }

    /// The confirmation was answered. True = rescan now.
    pub fn answer(&mut self, discard: bool) -> bool {
        let go = self.asking && discard;
        self.asking = false;
        go
    }

    /// The state went away (re-opened, pool changed): nothing to ask.
    pub fn reset(&mut self) {
        self.asking = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_scenery_save_invalidates_obstacles() {
        let mut fx = HostEffects::default();
        fx.absorb(CalibKind::Scenery, false);
        assert_eq!(fx, HostEffects::default());
        fx.absorb(CalibKind::Creatures, true);
        assert_eq!(fx, HostEffects { reload_art: true, invalidate_obstacles: false });
        fx.absorb(CalibKind::Scenery, true);
        assert_eq!(fx, HostEffects { reload_art: true, invalidate_obstacles: true });
        let mut fx = HostEffects::default();
        fx.absorb(CalibKind::Frames, true);
        assert_eq!(fx, HostEffects { reload_art: true, invalidate_obstacles: false });
    }

    #[test]
    fn rescan_asks_only_when_the_calibrator_reports_unsaved_edits() {
        let mut gate = RescanGate::default();
        assert!(gate.request(None), "no dirty flag: rescan straight away");
        assert!(gate.request(Some(false)));
        assert!(!gate.asking());
        assert!(!gate.request(Some(true)));
        assert!(gate.asking());
        assert!(!gate.answer(false), "cancelled");
        assert!(!gate.asking());
        assert!(!gate.answer(true), "an answer nobody asked for does nothing");
        assert!(!gate.request(Some(true)));
        assert!(gate.answer(true));
        assert!(!gate.asking());
        gate.request(Some(true));
        gate.reset();
        assert!(!gate.asking());
    }
}
