//! The depth-editing pass: the coordinate the rig's own view cannot see.
//!
//! A rig is clicked onto ONE silhouette, so every joint has only two of its
//! three coordinates. `gs_puppet::guess_depths` reads the third off the
//! family's OTHER silhouette; an artist corrects the guesses here, and only
//! the CORRECTIONS are written to the rig file's `[depth]` table, so a later,
//! better guess still applies to everything left alone.
//!
//! In DEPTH view the other silhouette is drawn as it is and a joint sits at
//! `(depth, the height authored in the primary view)`: the off-plane axis is
//! horizontal, y is vertical. Dragging moves a joint along that horizontal
//! axis only -- y belongs to the primary view and is never touched here.
//!
//! No egui in this module (see `super`): it is all rules and numbers.

use std::collections::BTreeMap;

use gs_puppet::{guess_depths_mask, DepthGuess};
use rig::{Pt, Rig, View};
use rig_bake::Mask;

use super::state::Editor;
use super::view::{Canvas, ViewTransform};
use super::warnings::inside_silhouette;

/// Undo tag for a depth gesture: one drag (or one run of DragValue ticks) is
/// one undo step. Bone-radius drags tag by bone index and the eye radius
/// takes `u64::MAX`, so depth takes the one below it.
pub const DEPTH_TAG: u64 = u64::MAX - 1;

/// Which of the two silhouettes the editor is working on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditView {
    /// The rig's own silhouette: placing and moving joints, as ever.
    Primary,
    /// The OTHER silhouette: each joint at (its depth, its authored height).
    Depth,
}

impl EditView {
    pub const ALL: [EditView; 2] = [EditView::Primary, EditView::Depth];

    pub fn label(self) -> &'static str {
        match self {
            EditView::Primary => "PRIMARY",
            EditView::Depth => "DEPTH",
        }
    }

}

/// [`axis_label`] for a whole rig: one drawn from above is missing its height.
pub fn axis_label_of(rig: &Rig) -> &'static str {
    if rig.from_above {
        "height (the side silhouette is shown turned head up: higher = further left)"
    } else {
        axis_label(rig.view)
    }
}

/// What the off-plane axis MEANS for a rig, for the UI to say out loud.
pub fn axis_label(view: View) -> &'static str {
    match view {
        // A front rig authors (x, y); the missing coordinate is front-back.
        View::Front => "z (front-back, + = the creature's front)",
        // A side rig authors (z, y); the missing coordinate is left-right.
        View::Side => "x (left-right, + = the creature's left)",
    }
}

/// The view whose silhouette DEPTH mode draws.
pub fn other_view(view: View) -> View {
    match view {
        View::Front => View::Side,
        View::Side => View::Front,
    }
}

/// One joint's off-plane coordinate, as the panel and the canvas show it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JointDepth {
    /// The value in use: the artist's override if there is one, else the guess.
    pub value: f32,
    /// True = the artist set it (draw it filled and bright, write it on Save).
    pub overridden: bool,
    /// What the guess would be. None = nothing to guess from.
    pub guess: Option<f32>,
}

impl JointDepth {
    /// True when the override and the guess agree to within a hair; resetting
    /// such a joint changes nothing visible but still drops it from the file.
    pub fn matches_guess(&self) -> bool {
        self.guess.is_some_and(|g| (g - self.value).abs() < 1e-4)
    }
}

/// The guesses for one rig revision, plus the other view's rasterized
/// silhouette (kept so the "outside the other silhouette" warning and the
/// canvas can both use it without rasterizing twice).
#[derive(Debug, Clone, Default)]
pub struct DepthModel {
    pub guesses: BTreeMap<String, DepthGuess>,
    /// None = this family has no second-view outline: DEPTH mode is off.
    pub other_mask: Option<Mask>,
}

impl DepthModel {
    /// Guess every placed joint's depth from an already rasterized OTHER view.
    /// `other` = None is a family with only one silhouette.
    pub fn new(rig: &Rig, other: Option<Mask>) -> DepthModel {
        // Drawn from above, the other (side) silhouette is worked on turned
        // head up, as the builder reads it: heights run left-right.
        let other = other.map(|m| if rig.from_above { gs_puppet::turned_head_up(&m) } else { m });
        DepthModel { guesses: guess_depths_mask(rig, other.as_ref()), other_mask: other }
    }

    /// False = there is no second silhouette, so there is nothing to edit
    /// against and DEPTH mode stays disabled.
    pub fn available(&self) -> bool {
        self.other_mask.is_some()
    }

    /// Why DEPTH mode is unavailable, for the disabled toggle's hover text.
    pub fn unavailable_reason(&self, rig: &Rig) -> String {
        format!(
            "No {} silhouette for '{}': depth is guessed from the family's OTHER view, and there is not one. \
             Add '{}' to outlines.json to edit depths.",
            other_view(rig.view).as_str(),
            rig.family,
            match other_view(rig.view) {
                View::Front => format!("{}-front", rig.family),
                View::Side => rig.family.clone(),
            }
        )
    }

    pub fn guess(&self, joint: &str) -> Option<f32> {
        self.guesses.get(joint).map(|g| g.value).filter(|v| v.is_finite())
    }

    /// How the guess was arrived at, in words, for a tooltip.
    pub fn guess_source(&self, joint: &str) -> Option<&'static str> {
        use gs_puppet::GuessSource::*;
        Some(match self.guesses.get(joint)?.source {
            SpanCentre => "centre of the other view's widest span on this row",
            ForwardEnd => "the forward end of the ground span (a toe)",
            LimbSpan => "centre of the limb-width span on this row, on this joint's side",
            Inherited => "taken from the nearest joint of the same bone that had a reading",
            CentreLine => "a centre-line joint: 0",
            Fallback => "nothing to read off the other view",
        })
    }

    /// The value in use for a joint, with where it came from.
    pub fn joint(&self, rig: &Rig, joint: &str) -> JointDepth {
        let guess = self.guess(joint);
        match rig.depth.get(joint).copied().filter(|v| v.is_finite()) {
            Some(value) => JointDepth { value, overridden: true, guess },
            None => JointDepth { value: guess.unwrap_or(0.0), overridden: false, guess },
        }
    }

    /// Where a joint is drawn in DEPTH view: the off-plane coordinate
    /// horizontally, the height authored in the primary view vertically.
    /// None = the joint is not placed, so it has no height yet.
    pub fn plot(&self, rig: &Rig, joint: &str) -> Option<Pt> {
        let p = rig.joints.get(joint)?;
        Some([self.joint(rig, joint).value, p[1]])
    }

    /// Placed joints whose depth puts them off the other silhouette, in name
    /// order. These are what the DEPTH-mode warning names.
    pub fn joints_off_other(&self, rig: &Rig) -> Vec<String> {
        let Some(mask) = &self.other_mask else {
            return Vec::new();
        };
        rig.joints
            .keys()
            .filter(|j| self.plot(rig, j).is_some_and(|p| !inside_silhouette(mask, p)))
            .cloned()
            .collect()
    }

    /// (overridden, placed) for the panel's "n of m overridden".
    pub fn override_counts(&self, rig: &Rig) -> (usize, usize) {
        let placed: Vec<&String> = rig.joints.keys().collect();
        let over = placed.iter().filter(|j| rig.depth.contains_key(**j)).count();
        (over, placed.len())
    }

    /// Warnings DEPTH mode adds to the rig's own list.
    pub fn warnings(&self, rig: &Rig) -> Vec<String> {
        let mut out = Vec::new();
        let off = self.joints_off_other(rig);
        if !off.is_empty() {
            out.push(format!(
                "depth puts {} joint(s) outside the {} silhouette: {}",
                off.len(),
                other_view(rig.view).as_str(),
                off.join(", ")
            ));
        }
        out
    }
}

/// A depth drag in progress. Only the off-plane coordinate moves.
#[derive(Debug, Clone)]
pub(super) struct DepthDrag {
    pub joint: String,
    /// Pointer-to-joint offset along the off-plane axis at grab time, in feet.
    pub grab_offset: f32,
    /// The depth in use when the joint was grabbed -- the guess, if it had no
    /// override yet. A drag that lands back here records nothing.
    pub at_grab: f32,
}

impl Editor {
    pub fn edit_view(&self) -> EditView {
        self.edit_view
    }

    /// Switching view is pure UI: it is not a rig change and is not undone.
    /// It does cancel any drag in flight and seal the undo step, so the next
    /// gesture starts fresh.
    pub fn set_edit_view(&mut self, view: EditView) {
        if self.edit_view != view {
            self.edit_view = view;
            self.drag = None;
            self.depth_drag = None;
            self.seal_history();
        }
    }

    pub fn is_depth_dragging(&self) -> bool {
        self.depth_drag.is_some()
    }

    /// Set one joint's depth override. Consecutive calls coalesce into one
    /// undo step until [`Editor::end_continuous_edit`].
    pub fn set_joint_depth(&mut self, joint: &str, depth_ft: f32) -> bool {
        if !depth_ft.is_finite() || !self.rig.joint_order.iter().any(|j| j == joint) {
            return false;
        }
        if self.rig.depth.get(joint) == Some(&depth_ft) {
            return false;
        }
        self.edit(Some(DEPTH_TAG)).depth.insert(joint.to_owned(), depth_ft);
        true
    }

    /// Drop one joint's override so the guess applies again.
    pub fn reset_joint_depth(&mut self, joint: &str) -> bool {
        if !self.rig.depth.contains_key(joint) {
            return false;
        }
        self.edit(None).depth.remove(joint);
        true
    }

    /// Drop every override: the whole rig goes back to the guesses.
    pub fn reset_all_depths(&mut self) -> usize {
        let n = self.rig.depth.len();
        if n > 0 {
            self.edit(None).depth.clear();
        }
        n
    }

    /// Overrides on PLACED joints: what "n of m overridden" counts. (An
    /// override on an unplaced joint is kept -- re-placing the joint brings
    /// it back -- but there is nothing on screen to count it against.)
    pub fn depth_override_count(&self) -> usize {
        self.rig.joints.keys().filter(|j| self.rig.depth.contains_key(*j)).count()
    }

    /// Nearest placed joint within [`super::state::HIT_RADIUS_PX`] of a screen
    /// position, in DEPTH view's plot (depth across, height up).
    pub fn hit_joint_depth(
        &self,
        model: &DepthModel,
        view: &ViewTransform,
        canvas: &Canvas,
        screen: Pt,
    ) -> Option<String> {
        self.rig
            .joints
            .keys()
            .filter_map(|name| {
                let s = view.world_to_screen(canvas, model.plot(&self.rig, name)?);
                Some((name, (s[0] - screen[0]).hypot(s[1] - screen[1])))
            })
            .filter(|(_, d)| *d <= super::state::HIT_RADIUS_PX)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(name, _)| name.clone())
    }

    /// Pointer went down on a joint in DEPTH view. Selects it; nothing is
    /// recorded until the depth actually changes.
    pub fn begin_depth_drag(&mut self, model: &DepthModel, joint: &str, pointer_world: Pt) {
        if !self.rig.joints.contains_key(joint) {
            return;
        }
        let at = model.joint(&self.rig, joint).value;
        self.selected = Some(joint.to_owned());
        self.seal_history();
        self.depth_drag =
            Some(DepthDrag { joint: joint.to_owned(), grab_offset: at - pointer_world[0], at_grab: at });
    }

    /// Move the dragged joint's DEPTH only. `pointer_world[1]` is ignored:
    /// y is the height authored in the primary view and is locked here.
    ///
    /// A press that does not move records nothing: the target is compared
    /// against the value in USE (guess included), not against the override
    /// table, so merely grabbing a guessed joint does not override it.
    pub fn depth_drag_to(&mut self, pointer_world: Pt) {
        let Some(drag) = self.depth_drag.as_ref() else {
            return;
        };
        let (joint, target) = (drag.joint.clone(), pointer_world[0] + drag.grab_offset);
        if (drag.at_grab - target).abs() < f32::EPSILON && !self.rig.depth.contains_key(&joint) {
            return;
        }
        self.set_joint_depth(&joint, target);
    }

    /// Pointer released: the next gesture is a new undo step, and the full
    /// bake runs.
    pub fn end_depth_drag(&mut self) {
        if self.depth_drag.take().is_some() {
            self.end_continuous_edit();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::{template, Template};
    use rig_bake::{rasterize, Outline};

    /// A 2 x 6 ft block: what both views of the test family look like.
    fn block(half_w: f32) -> Mask {
        rasterize(
            &Outline {
                subpaths: vec![vec![[-half_w, 0.0], [half_w, 0.0], [half_w, 6.0], [-half_w, 6.0]]],
            },
            120,
        )
    }

    fn biped() -> (Editor, Template) {
        let t = template("biped").unwrap();
        let mut e = Editor::new(t.new_rig("humanoid", View::Front, 6.0, "crown"), t.clone());
        // Two joints on the centre line, plus a sided pair.
        for (j, p) in [("crown", [0.0, 5.9]), ("chin", [0.0, 5.2]), ("hip_r", [-0.4, 3.0]), ("hip_l", [0.4, 3.0])] {
            e.set_current(j);
            e.mirror = false;
            e.place_current(p);
        }
        (e, t)
    }

    fn model(e: &Editor) -> DepthModel {
        DepthModel::new(e.rig(), Some(block(0.5)))
    }

    /// How many undo steps deep the history is, on a throwaway copy (placing
    /// the fixture's joints already put four steps on it).
    fn undo_steps(e: &Editor) -> usize {
        let mut probe = e.clone();
        let mut n = 0;
        while probe.undo() {
            n += 1;
        }
        n
    }

    /// The fixture's own four placements.
    const FIXTURE_STEPS: usize = 4;

    #[test]
    fn a_fresh_rig_is_all_guesses() {
        let (e, _) = biped();
        let m = model(&e);
        assert!(m.available());
        assert_eq!(m.override_counts(e.rig()), (0, 4));
        let d = m.joint(e.rig(), "crown");
        assert!(!d.overridden && d.guess.is_some());
        // A front rig reads the side block's centre: 0 for a symmetric block.
        assert!(d.value.abs() < 0.01, "{d:?}");
        assert!(m.guess_source("crown").is_some());
        assert!(e.rig().depth.is_empty(), "guesses are never stored");
    }

    #[test]
    fn a_family_with_one_silhouette_disables_depth_mode() {
        let (e, _) = biped();
        let m = DepthModel::new(e.rig(), None);
        assert!(!m.available());
        assert!(m.unavailable_reason(e.rig()).contains("side"));
        assert!(m.joints_off_other(e.rig()).is_empty(), "no mask, no off-silhouette warning");
        // Everything still answers, at the fallback value.
        assert_eq!(m.joint(e.rig(), "crown").value, 0.0);
    }

    #[test]
    fn a_depth_drag_changes_only_the_depth_and_is_one_undo_step() {
        let (mut e, _) = biped();
        let m = model(&e);
        let before = e.rig().joints["crown"];
        e.set_edit_view(EditView::Depth);
        e.begin_depth_drag(&m, "crown", [0.1, 99.0]);
        assert!(e.is_depth_dragging());
        // Pointer y is nonsense on purpose: it must not reach the rig.
        e.depth_drag_to([0.2, -30.0]);
        e.depth_drag_to([0.35, 400.0]);
        e.end_depth_drag();
        assert!(!e.is_depth_dragging());
        assert_eq!(e.rig().joints["crown"], before, "y and x belong to the primary view");
        let d = m.joint(e.rig(), "crown");
        assert!(d.overridden && (d.value - 0.25).abs() < 1e-5, "{d:?}");
        assert_eq!(undo_steps(&e), FIXTURE_STEPS + 1, "the whole drag is ONE step");
        assert!(e.undo());
        assert!(!m.joint(e.rig(), "crown").overridden, "one gesture, one undo step");
    }

    #[test]
    fn two_gestures_are_two_undo_steps() {
        let (mut e, _) = biped();
        let m = model(&e);
        for (from, to) in [(0.0, 0.3), (0.3, 0.7)] {
            e.begin_depth_drag(&m, "chin", [from, 0.0]);
            e.depth_drag_to([to, 0.0]);
            e.end_depth_drag();
        }
        assert!((m.joint(e.rig(), "chin").value - 0.7).abs() < 1e-5);
        assert_eq!(undo_steps(&e), FIXTURE_STEPS + 2);
        assert!(e.undo());
        assert!((m.joint(e.rig(), "chin").value - 0.3).abs() < 1e-5);
        assert!(e.undo());
        assert!(!m.joint(e.rig(), "chin").overridden);
    }

    #[test]
    fn a_press_without_movement_records_nothing() {
        let (mut e, _) = biped();
        let m = model(&e);
        let rev = e.revision();
        e.begin_depth_drag(&m, "chin", [0.0, 0.0]);
        e.depth_drag_to([0.0, 0.0]);
        e.end_depth_drag();
        assert_eq!(undo_steps(&e), FIXTURE_STEPS, "a press that moves nothing is not an edit");
        assert!(e.rig().depth.is_empty());
        assert!(e.revision() >= rev);
    }

    #[test]
    fn reset_restores_the_guess_and_drops_the_override() {
        let (mut e, _) = biped();
        let m = model(&e);
        let guess = m.joint(e.rig(), "hip_r").value;
        assert!(e.set_joint_depth("hip_r", guess + 1.25));
        e.end_continuous_edit();
        assert_eq!(m.override_counts(e.rig()), (1, 4));
        assert!(e.reset_joint_depth("hip_r"));
        assert!(!e.reset_joint_depth("hip_r"), "nothing left to reset");
        let d = m.joint(e.rig(), "hip_r");
        assert!(!d.overridden && (d.value - guess).abs() < 1e-6);
        assert!(e.undo(), "a reset is undoable");
        assert!(m.joint(e.rig(), "hip_r").overridden);
    }

    #[test]
    fn reset_all_clears_every_override_in_one_step() {
        let (mut e, _) = biped();
        let m = model(&e);
        for j in ["crown", "chin", "hip_r"] {
            e.set_joint_depth(j, 0.9);
            e.end_continuous_edit();
        }
        assert_eq!(e.depth_override_count(), 3);
        assert_eq!(m.override_counts(e.rig()), (3, 4));
        assert_eq!(e.reset_all_depths(), 3);
        assert_eq!(e.depth_override_count(), 0);
        assert_eq!(e.reset_all_depths(), 0, "nothing to do twice");
        assert!(e.undo());
        assert_eq!(e.depth_override_count(), 3, "one undo brings all three back");
    }

    #[test]
    fn only_overrides_are_written_and_guesses_are_not() {
        let (mut e, _) = biped();
        let m = model(&e);
        e.set_joint_depth("chin", 0.42);
        e.end_continuous_edit();
        let text = e.rig().to_toml().unwrap();
        let round = rig::Rig::from_toml(&text).unwrap();
        assert_eq!(round.depth.keys().collect::<Vec<_>>(), ["chin"]);
        assert!((round.depth["chin"] - 0.42).abs() < 1e-4);
        assert!(text.contains("[depth]"));
        for guessed in ["crown", "hip_r", "hip_l"] {
            assert!(!round.depth.contains_key(guessed), "{guessed} was a guess; it must not be written");
            assert!(m.guess(guessed).is_some(), "but it still has a guess to fall back on");
        }
        // Clearing every override drops the table from the file entirely.
        e.reset_all_depths();
        assert!(!e.rig().to_toml().unwrap().contains("[depth]"));
    }

    #[test]
    fn plot_puts_depth_across_and_the_authored_height_up() {
        let (mut e, _) = biped();
        let m = model(&e);
        e.set_joint_depth("hip_r", 0.3);
        let p = m.plot(e.rig(), "hip_r").unwrap();
        assert_eq!(p, [0.3, 3.0]);
        assert_eq!(m.plot(e.rig(), "never_placed"), None);
    }

    #[test]
    fn a_depth_off_the_other_silhouette_warns() {
        let (mut e, _) = biped();
        let m = model(&e);
        assert!(m.warnings(e.rig()).is_empty());
        e.set_joint_depth("chin", 9.0);
        let w = m.warnings(e.rig());
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].contains("outside the side silhouette") && w[0].contains("chin"), "{w:?}");
        assert_eq!(m.joints_off_other(e.rig()), ["chin"]);
    }

    #[test]
    fn hit_testing_works_in_the_depth_plot() {
        let (mut e, _) = biped();
        let m = model(&e);
        e.set_joint_depth("crown", 0.4);
        let view = ViewTransform { centre: [0.0, 5.9], zoom: 100.0 };
        let canvas = Canvas { min: [0.0, 0.0], size: [400.0, 400.0] };
        // crown plots at (0.4, 5.9) -> screen x = 200 + 40.
        assert_eq!(e.hit_joint_depth(&m, &view, &canvas, [240.0, 200.0]).as_deref(), Some("crown"));
        assert_eq!(e.hit_joint_depth(&m, &view, &canvas, [200.0, 200.0]), None, "nothing at the old x");
    }

    #[test]
    fn switching_view_is_not_a_rig_change() {
        let (mut e, _) = biped();
        e.mark_saved();
        let steps = undo_steps(&e);
        e.set_edit_view(EditView::Depth);
        assert_eq!(e.edit_view(), EditView::Depth);
        assert!(!e.is_dirty(), "a view toggle changes no rig data");
        assert_eq!(undo_steps(&e), steps, "and puts nothing on the undo stack");
        e.set_edit_view(EditView::Primary);
        assert!(!e.is_dirty());
    }

    #[test]
    fn matches_guess_spots_an_override_that_says_nothing_new() {
        let (mut e, _) = biped();
        let m = model(&e);
        let g = m.joint(e.rig(), "crown").guess.unwrap();
        e.set_joint_depth("crown", g);
        let d = m.joint(e.rig(), "crown");
        assert!(d.overridden && d.matches_guess());
        e.set_joint_depth("crown", g + 0.5);
        assert!(!m.joint(e.rig(), "crown").matches_guess());
    }

    #[test]
    fn a_side_rig_reads_its_depth_off_the_front_view() {
        let t = template("quadruped").unwrap();
        let mut e = Editor::new(t.new_rig("canine", View::Side, 3.0, "withers"), t);
        for (j, p) in [("shoulder_r", [0.6, 2.0]), ("shoulder_l", [0.6, 2.0])] {
            e.set_current(j);
            e.place_current(p);
        }
        let m = DepthModel::new(e.rig(), Some(block(1.0)));
        assert_eq!(axis_label(View::Side), "x (left-right, + = the creature's left)");
        assert_eq!(other_view(View::Side), View::Front);
        // The two limbs share one clicked point and are separated by depth.
        let (r, l) = (m.joint(e.rig(), "shoulder_r").value, m.joint(e.rig(), "shoulder_l").value);
        assert!(r < 0.0 && l > 0.0, "r {r}, l {l}");
        assert_eq!(m.plot(e.rig(), "shoulder_r").unwrap()[1], 2.0);
    }

    /// Opt-in: run the guesses over the workspace's REAL outlines and saved
    /// rigs and print what comes out, so the numbers can be eyeballed.
    /// `cargo test -p studio real_rigs -- --ignored --nocapture`
    #[test]
    #[ignore = "needs the workspace's data/ directory"]
    fn depth_guesses_against_the_real_rigs() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let Ok(outlines) = rig::Outlines::load(&root.join("outlines.json")) else {
            return;
        };
        let mut seen = 0;
        for entry in std::fs::read_dir(root.join("rigs")).unwrap() {
            let path = entry.unwrap().path();
            if !path.to_string_lossy().ends_with(".rig.toml") {
                continue;
            }
            let r = rig::Rig::load(&path).unwrap();
            let mask =
                outlines.get(&r.family, other_view(r.view)).map(|o| rig_bake::rasterize(&o.outline(), 512));
            let m = DepthModel::new(&r, mask);
            let (over, placed) = m.override_counts(&r);
            let off = m.joints_off_other(&r);
            println!(
                "{:12} {:5} placed={placed:3} over={over} off-silhouette={}",
                r.family,
                r.view.as_str(),
                off.len()
            );
            for (name, _) in r.joints.iter().take(6) {
                let d = m.joint(&r, name);
                println!("    {name:14} {:+.3} ft  {}", d.value, m.guess_source(name).unwrap_or("-"));
            }
            assert!(m.available(), "{} should have a second silhouette", r.family);
            for name in r.joints.keys() {
                assert!(m.joint(&r, name).value.is_finite(), "{}/{name} is not a number", r.family);
            }
            seen += 1;
        }
        assert!(seen > 0, "no saved rigs to check");
    }

    #[test]
    fn unknown_joints_and_nonsense_values_are_refused() {
        let (mut e, _) = biped();
        let steps = undo_steps(&e);
        assert!(!e.set_joint_depth("no_such_joint", 1.0));
        assert!(!e.set_joint_depth("crown", f32::NAN));
        assert!(!e.set_joint_depth("crown", f32::INFINITY));
        assert!(e.rig().depth.is_empty());
        assert_eq!(undo_steps(&e), steps, "a refused edit records nothing");
    }
}
