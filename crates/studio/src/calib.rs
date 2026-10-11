//! The calibrator tabs' rules, without egui: which calibrator a tab hosts,
//! what a calibrator's outcome asks of the rest of the app, the doll
//! picker's selection, and the "Rescan pool" confirmation. The calibrators
//! themselves live in `gs_calibrators`; `tools/calibrators.rs` hosts them.

/// One hosted calibrator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalibKind {
    Frames,
    Dolls,
    Creatures,
    Scenery,
}

/// The doll the Dolls tab targets: a pick out of the pool's doll list
/// that survives a rescan by PATH, not by index.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DollPicker {
    choices: Vec<String>,
    pick: usize,
}

impl DollPicker {
    pub fn choices(&self) -> &[String] {
        &self.choices
    }

    pub fn pick(&self) -> usize {
        self.pick
    }

    /// The picked pool path; None when the pool has no dolls.
    pub fn current(&self) -> Option<&str> {
        self.choices.get(self.pick).map(String::as_str)
    }

    /// Pick by index. True when that changed the target.
    pub fn select(&mut self, index: usize) -> bool {
        if index >= self.choices.len() || index == self.pick {
            return false;
        }
        self.pick = index;
        true
    }

    /// A fresh pool listing: keep the picked doll if it is still there,
    /// else fall back to the first.
    pub fn refresh(&mut self, choices: Vec<String>) {
        let kept = self.current().and_then(|path| choices.iter().position(|c| c == path));
        self.choices = choices;
        self.pick = kept.unwrap_or(0);
    }
}

/// What this frame's calibrator outcomes ask of the rest of the app.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HostEffects {
    /// A calibrator saved: art and sidecars changed on disk. The stage's
    /// art store must reload and creature_field's calibration store re-seed
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
        for kind in [CalibKind::Frames, CalibKind::Dolls] {
            let mut fx = HostEffects::default();
            fx.absorb(kind, true);
            assert_eq!(fx, HostEffects { reload_art: true, invalidate_obstacles: false });
        }
    }

    #[test]
    fn the_doll_pick_survives_a_rescan_by_path() {
        let mut picker = DollPicker::default();
        assert_eq!(picker.current(), None);
        assert!(!picker.select(0), "nothing to select in an empty list");
        picker.refresh(vec!["dolls/a.png".into(), "dolls/b.png".into(), "dolls/c.png".into()]);
        assert_eq!(picker.current(), Some("dolls/a.png"));
        assert!(picker.select(2));
        assert!(!picker.select(2), "same pick is not a re-target");
        assert!(!picker.select(9));
        // A new doll sorts in ahead of it: the pick follows the path.
        picker.refresh(vec!["dolls/0.png".into(), "dolls/a.png".into(), "dolls/c.png".into()]);
        assert_eq!((picker.pick(), picker.current()), (2, Some("dolls/c.png")));
        // It is gone: first doll.
        picker.refresh(vec!["dolls/a.png".into()]);
        assert_eq!((picker.pick(), picker.current()), (0, Some("dolls/a.png")));
        picker.refresh(vec![]);
        assert_eq!(picker.current(), None);
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
