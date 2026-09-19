//! The rig being edited and every rule about editing it.

use rig::{mirror_name, Pt, Rig, Template, View};

use super::undo::UndoStack;
use super::view::{Canvas, ViewTransform};

/// How close (screen px) a click must be to grab a joint.
pub const HIT_RADIUS_PX: f32 = 9.0;

#[derive(Debug, Clone)]
struct Drag {
    joint: String,
    /// Pointer-to-joint offset at grab time, in feet, so the joint does not
    /// jump to the cursor.
    grab_offset: Pt,
    /// Snapshot to record once the joint actually moves.
    before: Option<Rig>,
}

#[derive(Debug, Clone)]
pub struct Editor {
    rig: Rig,
    template: Template,
    /// Joint the next canvas click places.
    current: Option<String>,
    /// Joint Delete acts on.
    selected: Option<String>,
    pub mirror: bool,
    /// The rig as last saved / loaded / created: dirty = differs from this.
    baseline: Rig,
    history: UndoStack<Rig>,
    drag: Option<Drag>,
    /// Bumped on every rig change; the baker re-runs when it moves.
    revision: u64,
}

impl Editor {
    /// Edit an existing rig. `template` supplies click order and warnings.
    pub fn new(rig: Rig, template: Template) -> Editor {
        let mut e = Editor {
            baseline: rig.clone(),
            rig,
            template,
            current: None,
            selected: None,
            mirror: true,
            history: UndoStack::default(),
            drag: None,
            revision: 0,
        };
        e.current = e.first_unplaced();
        e
    }

    pub fn rig(&self) -> &Rig {
        &self.rig
    }

    pub fn template(&self) -> &Template {
        &self.template
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

    /// Click-through order: the template's joints, then anything else the
    /// rig mentions (a hand-edited file, or a rig from an older template).
    pub fn joint_order(&self) -> Vec<String> {
        let mut order = self.template.joints.clone();
        let extra = self.rig.bones.iter().flat_map(|b| b.joints.iter()).chain(self.rig.joints.keys());
        for j in extra {
            if !order.contains(j) {
                order.push(j.clone());
            }
        }
        order
    }

    pub fn is_placed(&self, joint: &str) -> bool {
        self.rig.joints.contains_key(joint)
    }

    fn first_unplaced(&self) -> Option<String> {
        self.joint_order().into_iter().find(|j| !self.is_placed(j))
    }

    /// The next unplaced joint after `from` in click order, wrapping round.
    fn next_unplaced_after(&self, from: &str) -> Option<String> {
        let order = self.joint_order();
        let start = order.iter().position(|j| j == from).map_or(0, |i| i + 1);
        (0..order.len()).map(|k| &order[(start + k) % order.len()]).find(|j| !self.is_placed(j)).cloned()
    }

    /// Clicking a joint in the list: it becomes current (and selected, if it
    /// is on the canvas).
    pub fn set_current(&mut self, joint: &str) {
        self.current = Some(joint.to_owned());
        self.selected = self.is_placed(joint).then(|| joint.to_owned());
    }

    fn edit(&mut self, tag: Option<u64>) -> &mut Rig {
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
        self.drag = None;
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

    /// A slider / drag-value was released.
    pub fn end_continuous_edit(&mut self) {
        self.history.seal();
        self.revision += 1;
    }

    pub fn set_axis(&mut self, axis: &str) {
        if self.rig.axis != axis {
            self.edit(None).axis = axis.to_owned();
        }
    }

    pub fn undo(&mut self) -> bool {
        self.drag = None;
        let done = self.history.undo(&mut self.rig);
        self.after_history_jump(done)
    }

    pub fn redo(&mut self) -> bool {
        self.drag = None;
        let done = self.history.redo(&mut self.rig);
        self.after_history_jump(done)
    }

    fn after_history_jump(&mut self, done: bool) -> bool {
        if done {
            self.revision += 1;
            if self.selected.as_deref().is_some_and(|j| !self.is_placed(j)) {
                self.selected = None;
            }
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
            joints: vec!["wrist_r".into(), "club_tip".into()],
            radius_ft: None,
        });
        let order = Editor::new(rig, t.clone()).joint_order();
        assert_eq!(&order[..t.joints.len()], &t.joints[..]);
        assert_eq!(&order[t.joints.len()..], ["club_tip", "horn"]);
    }
}
