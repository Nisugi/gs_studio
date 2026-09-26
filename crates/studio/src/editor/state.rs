//! The rig being edited and the rules for placing and moving joints. The
//! rules for changing the rig's STRUCTURE (adding, renaming, deleting and
//! re-ordering joints and bones, the hierarchy, anchors, eyes) are in
//! `structure.rs`.

use std::collections::BTreeMap;

use rig::{mirror_name, Pt, Rig, Template, View};

use super::depth::{DepthDrag, EditView};
use super::undo::UndoStack;
use super::view::{Canvas, ViewTransform};

/// How close (screen px) a click must be to grab a joint.
pub const HIT_RADIUS_PX: f32 = 9.0;

/// Undo tags for a bone's depth and tip thickness drags (plus the bone's
/// index): apart from its width's (the bare index) and from each other.
const DEPTH_RADIUS_TAG: u64 = 1 << 32;
const TIP_RADIUS_TAG: u64 = 2 << 32;

#[derive(Debug, Clone)]
pub(super) struct Drag {
    joint: String,
    /// Pointer-to-joint offset at grab time, in feet, so the joint does not
    /// jump to the cursor.
    grab_offset: Pt,
    /// Snapshot to record once the joint actually moves.
    before: Option<Rig>,
}

#[derive(Debug, Clone)]
pub struct Editor {
    pub(super) rig: Rig,
    /// Joint the next canvas click places.
    pub(super) current: Option<String>,
    /// Joint Delete acts on.
    pub(super) selected: Option<String>,
    /// Bone whose details (parent, pivot, chain) are open. UI state: not
    /// part of the rig, not undone.
    pub(super) selected_bone: Option<usize>,
    /// Bone whose chain canvas clicks on joints are appending to.
    pub(super) chain_pick: Option<usize>,
    /// Which silhouette is being worked on: the rig's own, or the other one
    /// with each joint at its depth. UI state: not part of the rig, not undone.
    pub(super) edit_view: EditView,
    pub mirror: bool,
    /// The rig as last saved / loaded / created: dirty = differs from this.
    baseline: Rig,
    history: UndoStack<Rig>,
    pub(super) drag: Option<Drag>,
    /// A DEPTH-view drag: it moves the off-plane coordinate only.
    pub(super) depth_drag: Option<DepthDrag>,
    /// Bumped on every rig change; the baker re-runs when it moves.
    revision: u64,
}

impl Editor {
    /// Edit an existing rig. The rig is migrated first (`Rig::migrate`), so
    /// its `joint_order` lists every joint it mentions; `template` fills the
    /// gaps of a file from before joints and parents were stored.
    pub fn new(mut rig: Rig, template: Template) -> Editor {
        rig.migrate(Some(&template));
        let mut e = Editor {
            baseline: rig.clone(),
            rig,
            current: None,
            selected: None,
            selected_bone: None,
            chain_pick: None,
            edit_view: EditView::Primary,
            mirror: true,
            history: UndoStack::default(),
            drag: None,
            depth_drag: None,
            revision: 0,
        };
        e.current = e.first_unplaced();
        e
    }

    pub fn rig(&self) -> &Rig {
        &self.rig
    }

    pub fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }

    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn is_dirty(&self) -> bool {
        self.rig != self.baseline
    }

    pub fn mark_saved(&mut self) {
        self.baseline = self.rig.clone();
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// Mirroring only means something face-on, where x = 0 is the body's
    /// centre line.
    pub fn mirror_applies(&self) -> bool {
        self.mirror && self.rig.view == View::Front
    }

    /// Click-through order: the rig's own joint list. (`Rig::migrate` seeds
    /// it from the template and appends anything else the rig mentions.)
    pub fn joint_order(&self) -> Vec<String> {
        self.rig.joint_order.clone()
    }

    pub fn is_placed(&self, joint: &str) -> bool {
        self.rig.joints.contains_key(joint)
    }

    /// Deliberately left out of this family (see [`Rig::unused`]).
    pub fn is_unused(&self, joint: &str) -> bool {
        self.rig.unused.contains(joint)
    }

    /// Still to be placed: not placed, and not marked unused.
    fn wants_placing(&self, joint: &str) -> bool {
        !self.is_placed(joint) && !self.is_unused(joint)
    }

    pub(super) fn first_unplaced(&self) -> Option<String> {
        self.joint_order().into_iter().find(|j| self.wants_placing(j))
    }

    /// Mark a joint as one this family does not have (it is un-placed and the
    /// click-through skips it), or bring it back. One undo step.
    pub fn set_unused(&mut self, joint: &str, unused: bool) {
        if self.is_unused(joint) == unused || !self.rig.joint_order.iter().any(|j| j == joint) {
            return;
        }
        let rig = self.edit(None);
        if unused {
            rig.unused.insert(joint.to_owned());
            rig.joints.remove(joint);
        } else {
            rig.unused.remove(joint);
        }
        if self.selected.as_deref() == Some(joint) {
            self.selected = None;
        }
        self.current = if unused { self.first_unplaced() } else { Some(joint.to_owned()) };
    }

    /// The next unplaced joint after `from` in click order, wrapping round.
    fn next_unplaced_after(&self, from: &str) -> Option<String> {
        let order = self.joint_order();
        let start = order.iter().position(|j| j == from).map_or(0, |i| i + 1);
        (0..order.len()).map(|k| &order[(start + k) % order.len()]).find(|j| self.wants_placing(j)).cloned()
    }

    /// Clicking a joint in the list: it becomes current (and selected, if it
    /// is on the canvas).
    pub fn set_current(&mut self, joint: &str) {
        self.current = Some(joint.to_owned());
        self.selected = self.is_placed(joint).then(|| joint.to_owned());
    }

    pub(super) fn edit(&mut self, tag: Option<u64>) -> &mut Rig {
        self.history.record(self.rig.clone(), tag);
        self.revision += 1;
        &mut self.rig
    }

    /// Click on empty canvas: place the current joint at `world` (moving it
    /// if it was already placed), mirror it if the rule applies, advance to
    /// the next unplaced joint. Returns the joints that were placed.
    pub fn place_current(&mut self, world: Pt) -> Vec<String> {
        let Some(joint) = self.current.clone() else {
            return Vec::new();
        };
        let twin = mirror_name(&joint)
            .filter(|m| self.mirror_applies() && !self.is_placed(m) && self.joint_order().contains(m));
        let rig = self.edit(None);
        rig.joints.insert(joint.clone(), world);
        // Placing a joint says the family has it after all.
        rig.unused.remove(&joint);
        let mut placed = vec![joint.clone()];
        if let Some(twin) = twin {
            rig.joints.insert(twin.clone(), [-world[0], world[1]]);
            placed.push(twin);
        }
        self.selected = Some(joint.clone());
        self.current = self.next_unplaced_after(&joint);
        placed
    }

    /// Nearest placed joint within `HIT_RADIUS_PX` of a screen position.
    pub fn hit_joint(&self, view: &ViewTransform, canvas: &Canvas, screen: Pt) -> Option<String> {
        self.rig
            .joints
            .iter()
            .map(|(name, p)| {
                let s = view.world_to_screen(canvas, *p);
                (name, (s[0] - screen[0]).hypot(s[1] - screen[1]))
            })
            .filter(|(_, d)| *d <= HIT_RADIUS_PX)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(name, _)| name.clone())
    }

    /// Plain click on a joint: select it (Delete then un-places it).
    pub fn select(&mut self, joint: &str) {
        self.selected = self.is_placed(joint).then(|| joint.to_owned());
    }

    /// Pointer went down on a joint. Selects it; nothing is recorded until
    /// it actually moves.
    pub fn begin_drag(&mut self, joint: &str, pointer_world: Pt) {
        let Some(p) = self.rig.joints.get(joint) else {
            return;
        };
        self.selected = Some(joint.to_owned());
        self.drag = Some(Drag {
            joint: joint.to_owned(),
            grab_offset: [p[0] - pointer_world[0], p[1] - pointer_world[1]],
            before: Some(self.rig.clone()),
        });
    }

    pub fn drag_to(&mut self, pointer_world: Pt) {
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        let target = [pointer_world[0] + drag.grab_offset[0], pointer_world[1] + drag.grab_offset[1]];
        if self.rig.joints.get(&drag.joint) == Some(&target) {
            return;
        }
        if let Some(before) = drag.before.take() {
            self.history.record(before, None);
        }
        self.rig.joints.insert(drag.joint.clone(), target);
        self.revision += 1;
    }

    /// Pointer released. Bumps the revision so the full-size bake runs.
    pub fn end_drag(&mut self) {
        if let Some(drag) = self.drag.take() {
            if drag.before.is_none() {
                self.revision += 1;
            }
        }
    }

    /// Right-click / Delete: take a joint off the canvas. It becomes current
    /// so the next click puts it back.
    pub fn unplace(&mut self, joint: &str) -> bool {
        if !self.is_placed(joint) {
            return false;
        }
        (self.drag, self.depth_drag) = (None, None);
        self.edit(None).joints.remove(joint);
        self.selected = None;
        self.current = Some(joint.to_owned());
        true
    }

    pub fn unplace_selected(&mut self) -> bool {
        match self.selected.clone() {
            Some(j) => self.unplace(&j),
            None => false,
        }
    }

    /// `part` = None is "(none)": the bone stays but labels nothing.
    pub fn set_bone_part(&mut self, bone: usize, part: Option<&str>) {
        let part = part.map(str::to_owned);
        if self.rig.bones.get(bone).is_some_and(|b| b.part != part) {
            self.edit(None).bones[bone].part = part;
        }
    }

    /// `radius_ft` = None is Auto. Consecutive changes to the same bone are
    /// one undo step until [`Editor::end_continuous_edit`].
    pub fn set_bone_radius(&mut self, bone: usize, radius_ft: Option<f32>) {
        let radius_ft = radius_ft.map(|r| r.max(0.0));
        if self.rig.bones.get(bone).is_some_and(|b| b.radius_ft != radius_ft) {
            self.edit(Some(bone as u64)).bones[bone].radius_ft = radius_ft;
        }
    }

    /// Whether the bone is a membrane (`RigBone::membrane`): a skin
    /// stretched between bones (a wing), built flat in the plane of its
    /// joints instead of as a tube.
    pub fn set_bone_membrane(&mut self, bone: usize, on: bool) {
        if self.rig.bones.get(bone).is_some_and(|b| b.membrane != on) {
            self.edit(None).bones[bone].membrane = on;
        }
    }

    /// The bone's off-plane half-thickness (`Rig::radius_depth_ft`); None
    /// is Auto (measured from the other silhouette). Drags coalesce as
    /// [`Editor::set_bone_radius`].
    pub fn set_bone_depth_radius(&mut self, bone: usize, radius_ft: Option<f32>) {
        self.set_bone_map(bone, radius_ft, DEPTH_RADIUS_TAG, |r| &mut r.radius_depth_ft);
    }

    /// The width the bone tapers to at its last joint
    /// (`Rig::radius_tip_ft`); None is no taper.
    pub fn set_bone_tip_radius(&mut self, bone: usize, radius_ft: Option<f32>) {
        self.set_bone_map(bone, radius_ft, TIP_RADIUS_TAG, |r| &mut r.radius_tip_ft);
    }

    fn set_bone_map(&mut self, bone: usize, radius_ft: Option<f32>, tag: u64, map: fn(&mut Rig) -> &mut BTreeMap<String, f32>) {
        let Some(name) = self.rig.bones.get(bone).map(|b| b.name.clone()) else {
            return;
        };
        // Zero means "not set" to the builder: keep a set value above it.
        let radius_ft = radius_ft.map(|r| r.max(0.01));
        if map(&mut self.rig).get(&name).copied() == radius_ft {
            return;
        }
        let m = map(self.edit(Some(tag + bone as u64)));
        match radius_ft {
            Some(r) => m.insert(name, r),
            None => m.remove(&name),
        };
    }

    /// A slider / drag-value was released.
    pub fn end_continuous_edit(&mut self) {
        self.history.seal();
        self.revision += 1;
    }

    /// End the current undo step without claiming anything changed (so no
    /// re-bake): the next edit starts a new step even with the same tag.
    pub(super) fn seal_history(&mut self) {
        self.history.seal();
    }

    pub fn set_axis(&mut self, axis: &str) {
        if self.rig.axis != axis {
            self.edit(None).axis = axis.to_owned();
        }
    }

    /// Whether the rig's silhouette is a drawing from above (`Rig::from_above`).
    pub fn set_from_above(&mut self, on: bool) {
        if self.rig.from_above != on {
            self.edit(None).from_above = on;
        }
    }

    pub fn undo(&mut self) -> bool {
        (self.drag, self.depth_drag) = (None, None);
        let done = self.history.undo(&mut self.rig);
        self.after_history_jump(done)
    }

    pub fn redo(&mut self) -> bool {
        (self.drag, self.depth_drag) = (None, None);
        let done = self.history.redo(&mut self.rig);
        self.after_history_jump(done)
    }

    fn after_history_jump(&mut self, done: bool) -> bool {
        if done {
            self.revision += 1;
            if self.selected.as_deref().is_some_and(|j| !self.is_placed(j)) {
                self.selected = None;
            }
            // Bone indices may have shifted under the UI's feet.
            let bones = self.rig.bones.len();
            self.selected_bone = self.selected_bone.filter(|b| *b < bones);
            self.chain_pick = None;
            // Keep clicking through: never leave "current" on a placed joint
            // when something earlier has just become unplaced.
            self.current = self.first_unplaced();
        }
        done
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::template;

    fn biped() -> Editor {
        let t = template("biped").unwrap();
        Editor::new(t.new_rig("humanoid", View::Front, 6.0, "crown"), t)
    }

    fn quadruped() -> Editor {
        let t = template("quadruped").unwrap();
        Editor::new(t.new_rig("canine", View::Side, 3.0, "withers"), t)
    }

    #[test]
    fn clicks_walk_the_template_order() {
        let mut e = biped();
        assert_eq!(e.current(), Some("crown"));
        assert_eq!(e.place_current([0.0, 5.9]), ["crown"]);
        assert_eq!(e.current(), Some("chin"));
        assert_eq!(e.selected(), Some("crown"));
        assert!(e.is_dirty() && e.can_undo());
    }

    #[test]
    fn mirror_places_the_twin_and_skips_it() {
        let mut e = biped();
        e.set_current("shoulder_r");
        assert_eq!(e.place_current([-0.55, 4.8]), ["shoulder_r", "shoulder_l"]);
        assert_eq!(e.rig().joints["shoulder_l"], [0.55, 4.8]);
        assert_eq!(e.current(), Some("elbow_r"));
        // One undo removes both.
        assert!(e.undo());
        assert!(!e.is_placed("shoulder_r") && !e.is_placed("shoulder_l"));
    }

    #[test]
    fn mirror_never_overwrites_a_placed_twin() {
        let mut e = biped();
        e.set_current("hip_l");
        e.mirror = false;
        e.place_current([0.4, 3.0]);
        e.mirror = true;
        e.set_current("hip_r");
        assert_eq!(e.place_current([-0.3, 3.1]), ["hip_r"]);
        assert_eq!(e.rig().joints["hip_l"], [0.4, 3.0]);
    }

    #[test]
    fn mirror_is_off_in_side_views_and_when_unticked() {
        let mut e = quadruped();
        e.set_current("shoulder_r");
        assert!(!e.mirror_applies());
        assert_eq!(e.place_current([0.8, 2.0]), ["shoulder_r"]);
        assert!(!e.is_placed("shoulder_l"));

        let mut e = biped();
        e.mirror = false;
        e.set_current("knee_r");
        assert_eq!(e.place_current([-0.3, 1.6]), ["knee_r"]);
    }

    #[test]
    fn advancing_wraps_to_earlier_gaps_and_stops_when_full() {
        let mut e = biped();
        for j in e.joint_order() {
            if j != "chin" {
                e.set_current(&j);
                e.mirror = false;
                e.place_current([0.0, 1.0]);
            }
        }
        assert_eq!(e.current(), Some("chin"));
        e.place_current([0.0, 5.2]);
        assert_eq!(e.current(), None);
        // With nothing current a click places nothing.
        assert!(e.place_current([1.0, 1.0]).is_empty());
    }

    #[test]
    fn placing_a_placed_current_joint_moves_it() {
        let mut e = biped();
        e.place_current([0.0, 5.9]);
        e.set_current("crown");
        assert_eq!(e.selected(), Some("crown"));
        e.place_current([0.0, 6.0]);
        assert_eq!(e.rig().joints["crown"], [0.0, 6.0]);
        assert_eq!(e.current(), Some("chin"));
    }

    #[test]
    fn hit_testing_takes_the_nearest_joint_in_screen_space() {
        let mut e = biped();
        e.mirror = false;
        e.place_current([0.0, 5.0]);
        e.place_current([0.05, 5.0]);
        let view = ViewTransform { centre: [0.0, 5.0], zoom: 100.0 };
        let canvas = Canvas { min: [0.0, 0.0], size: [400.0, 400.0] };
        // crown at x=200, chin at x=205.
        assert_eq!(e.hit_joint(&view, &canvas, [199.0, 200.0]).as_deref(), Some("crown"));
        assert_eq!(e.hit_joint(&view, &canvas, [204.0, 203.0]).as_deref(), Some("chin"));
        assert_eq!(e.hit_joint(&view, &canvas, [230.0, 200.0]), None);
        // Zoomed far out the same world offset is within reach.
        let far = ViewTransform { centre: [0.0, 5.0], zoom: 10.0 };
        assert!(e.hit_joint(&far, &canvas, [203.0, 200.0]).is_some());
    }

    #[test]
    fn a_drag_is_one_undo_step_and_keeps_the_grab_offset() {
        let mut e = biped();
        e.place_current([0.0, 5.9]);
        let rev = e.revision();
        e.begin_drag("crown", [0.1, 5.8]);
        assert!(e.is_dragging());
        e.drag_to([0.2, 5.8]);
        e.drag_to([0.3, 5.7]);
        e.end_drag();
        assert!(!e.is_dragging());
        let p = e.rig().joints["crown"];
        assert!((p[0] - 0.2).abs() < 1e-6 && (p[1] - 5.8).abs() < 1e-6, "{p:?}");
        assert!(e.revision() > rev + 2, "release must trigger a re-bake");
        assert!(e.undo());
        assert_eq!(e.rig().joints["crown"], [0.0, 5.9]);
        assert!(e.undo());
        assert!(!e.is_placed("crown"));
        assert!(!e.can_undo());
    }

    #[test]
    fn a_press_without_movement_records_nothing() {
        let mut e = biped();
        e.place_current([0.0, 5.9]);
        let rev = e.revision();
        e.begin_drag("crown", [0.0, 5.9]);
        e.drag_to([0.0, 5.9]);
        e.end_drag();
        assert_eq!(e.revision(), rev);
        // Grabbing a joint does not steal the click-through cursor.
        assert_eq!(e.current(), Some("chin"));
        assert_eq!(e.selected(), Some("crown"));
        e.undo();
        assert!(!e.can_undo());
    }

    /// A joint the family does not have: skipped by the click-through,
    /// un-placed when marked, un-marked by placing it, carried through a
    /// rename and dropped with a delete; one undo step each.
    #[test]
    fn unused_joints_are_skipped_and_follow_edits() {
        let mut e = quadruped();
        assert_eq!(e.current(), Some("nose"));
        e.place_current([1.0, 2.0]);
        let next = e.current().unwrap().to_owned();
        e.place_current([0.5, 2.5]);
        assert!(e.is_placed(&next));
        // Marking a placed joint unused un-places it and moves the cursor on.
        e.set_unused(&next, true);
        assert!(e.is_unused(&next) && !e.is_placed(&next));
        assert_ne!(e.current(), Some(next.as_str()));
        // The click-through never lands on it.
        let order = e.joint_order();
        for _ in 0..order.len() {
            let Some(c) = e.current().map(str::to_owned) else { break };
            assert_ne!(c, next);
            e.place_current([0.0, 1.0]);
        }
        assert_eq!(e.current(), None, "every joint but the unused one is placed");
        // Undo brings it back as placed.
        e.undo();
        e.undo();
        let mut steps = 0;
        while e.is_unused(&next) {
            e.undo();
            steps += 1;
            assert!(steps < 100);
        }
        assert!(e.is_placed(&next));
        // Placing it clears the mark; rename and delete keep the set in step.
        e.set_unused("eye_r", true);
        e.set_current("eye_r");
        e.place_current([1.0, 2.6]);
        assert!(!e.is_unused("eye_r"));
        e.set_unused("tail_tip", true);
        e.rename_joint("tail_tip", "tail_end", false).unwrap();
        assert!(e.is_unused("tail_end") && !e.is_unused("tail_tip"));
        e.delete_joint("tail_end");
        assert!(e.rig().unused.is_empty());
    }

    #[test]
    fn unplacing_makes_the_joint_current_again() {
        let mut e = biped();
        e.place_current([0.0, 5.9]);
        e.place_current([0.0, 5.2]);
        assert!(e.unplace("crown"));
        assert!(!e.unplace("crown"));
        assert_eq!(e.current(), Some("crown"));
        assert_eq!(e.selected(), None);
        e.place_current([0.0, 5.95]);
        assert_eq!(e.current(), Some("neck_base"));
        e.set_current("chin");
        assert!(e.unplace_selected());
        assert!(!e.is_placed("chin"));
        assert!(!e.unplace_selected());
    }

    #[test]
    fn bone_edits_undo_and_radius_drags_coalesce() {
        let mut e = biped();
        e.set_bone_part(0, None);
        assert_eq!(e.rig().bones[0].part, None);
        e.set_bone_part(0, None); // no-op: no extra undo step
        for r in [0.1, 0.2, 0.3] {
            e.set_bone_radius(1, Some(r));
        }
        e.end_continuous_edit();
        e.set_bone_radius(1, Some(-4.0));
        assert_eq!(e.rig().bones[1].radius_ft, Some(0.0));
        assert!(e.undo());
        assert_eq!(e.rig().bones[1].radius_ft, Some(0.3));
        assert!(e.undo());
        assert_eq!(e.rig().bones[1].radius_ft, None);
        assert!(e.undo());
        assert_eq!(e.rig().bones[0].part.as_deref(), Some("head"));
        assert!(!e.is_dirty());
        assert!(e.redo() && e.is_dirty());
    }

    #[test]
    fn depth_and_tip_thickness_set_undo_and_follow_a_rename() {
        let mut e = biped();
        let name = e.rig().bones[1].name.clone();
        for r in [0.1, 0.2] {
            e.set_bone_depth_radius(1, Some(r));
        }
        e.end_continuous_edit();
        e.set_bone_tip_radius(1, Some(0.0));
        assert_eq!(e.rig().radius_depth_ft.get(&name), Some(&0.2));
        assert_eq!(e.rig().radius_tip_ft.get(&name), Some(&0.01), "a set value stays above zero");
        assert!(e.undo());
        assert!(e.rig().radius_tip_ft.is_empty());
        assert!(e.undo(), "one drag, one step");
        assert!(e.rig().radius_depth_ft.is_empty());
        assert!(e.redo());
        e.rename_bone(1, "spine_x").unwrap();
        assert_eq!(e.rig().radius_depth_ft.get("spine_x"), Some(&0.2), "the override follows the rename");
        assert!(!e.rig().radius_depth_ft.contains_key(&name));
        e.set_bone_depth_radius(1, None);
        assert!(e.rig().radius_depth_ft.is_empty());
        e.set_bone_tip_radius(1, Some(0.05));
        e.delete_bone(1);
        assert!(e.rig().radius_tip_ft.is_empty(), "a deleted bone takes its overrides");
    }

    #[test]
    fn dirty_tracks_the_saved_baseline_not_the_history() {
        let mut e = biped();
        assert!(!e.is_dirty());
        e.place_current([0.0, 5.9]);
        e.mark_saved();
        assert!(!e.is_dirty());
        e.undo();
        assert!(e.is_dirty(), "undoing past a save is an unsaved change");
        e.redo();
        assert!(!e.is_dirty());
    }

    #[test]
    fn joints_outside_the_template_still_get_listed() {
        let t = template("biped").unwrap();
        let mut rig = t.new_rig("troll", View::Front, 6.0, "crown");
        rig.joints.insert("horn".into(), [0.0, 6.5]);
        rig.bones.push(rig::RigBone {
            name: "club".into(),
            part: None,
            parent: None,
            joints: vec!["wrist_r".into(), "club_tip".into()],
            pivot: None,
            radius_ft: None,
            membrane: false,
        });
        let order = Editor::new(rig, t.clone()).joint_order();
        assert_eq!(&order[..t.joints.len()], &t.joints[..]);
        assert_eq!(&order[t.joints.len()..], ["club_tip", "horn"]);
    }
}
