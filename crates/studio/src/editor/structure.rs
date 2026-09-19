//! Changing what a rig is MADE of: joints (add, rename, delete, re-order),
//! bones (add, rename, delete, chain, parent, pivot), anchors and eyes.
//! Every operation is one undo step and keeps the rig self-consistent: a
//! rename reaches every bone, anchor and eye that names the joint; deleting
//! a bone hands its children to its parent; a parent can never be set to a
//! descendant.

use rig::{mirror_name, Rig, RigBone};

use super::state::Editor;

pub const MAX_NAME_LEN: usize = 40;

/// Undo tag for an eye-radius drag (bone radius drags use the bone index).
const EYE_RADIUS_TAG: u64 = u64::MAX;

/// The three client attachment points that name a joint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorSlot {
    Head,
    Mouth,
    Saddle,
}

impl AnchorSlot {
    pub const ALL: [AnchorSlot; 3] = [AnchorSlot::Head, AnchorSlot::Mouth, AnchorSlot::Saddle];

    pub fn label(self) -> &'static str {
        match self {
            AnchorSlot::Head => "head",
            AnchorSlot::Mouth => "mouth",
            AnchorSlot::Saddle => "saddle",
        }
    }

    pub fn get(self, rig: &Rig) -> &str {
        match self {
            AnchorSlot::Head => &rig.anchors.head,
            AnchorSlot::Mouth => &rig.anchors.mouth,
            AnchorSlot::Saddle => &rig.anchors.saddle,
        }
    }

    fn get_mut(self, rig: &mut Rig) -> &mut String {
        match self {
            AnchorSlot::Head => &mut rig.anchors.head,
            AnchorSlot::Mouth => &mut rig.anchors.mouth,
            AnchorSlot::Saddle => &mut rig.anchors.saddle,
        }
    }
}

/// A joint or bone name: lower snake_case, starting with a letter. Returns
/// the trimmed name, or why it will not do. (`_r` / `_l` endings are what
/// make a mirror pair, so a name may not be ONLY such an ending, and may not
/// end in a bare underscore.)
pub fn check_name(kind: &str, name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(format!("Type a {kind} name"));
    }
    if name.len() > MAX_NAME_LEN {
        return Err(format!("At most {MAX_NAME_LEN} characters"));
    }
    if !name.starts_with(|c: char| c.is_ascii_lowercase()) {
        return Err("Start with a lower-case letter".to_owned());
    }
    if let Some(bad) = name.chars().find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')) {
        return Err(format!("'{bad}' is not allowed: use lower-case letters, digits and _"));
    }
    if name.ends_with('_') || name.contains("__") {
        return Err("No trailing or doubled underscores".to_owned());
    }
    Ok(name.to_owned())
}

/// Rename a joint everywhere a rig names it.
fn rename_joint_in(rig: &mut Rig, old: &str, new: &str) {
    for j in rig.joint_order.iter_mut().filter(|j| *j == old) {
        *j = new.to_owned();
    }
    if let Some(p) = rig.joints.remove(old) {
        rig.joints.insert(new.to_owned(), p);
    }
    // The depth override belongs to the joint, so it follows the rename.
    if let Some(d) = rig.depth.remove(old) {
        rig.depth.insert(new.to_owned(), d);
    }
    for bone in &mut rig.bones {
        for j in bone.joints.iter_mut().filter(|j| *j == old) {
            *j = new.to_owned();
        }
        if bone.pivot.as_deref() == Some(old) {
            bone.pivot = Some(new.to_owned());
        }
    }
    for slot in AnchorSlot::ALL {
        if slot.get(rig) == old {
            *slot.get_mut(rig) = new.to_owned();
        }
    }
    for left in [true, false] {
        if rig.eyes.joint(left) == old {
            set_eye_joint_in(rig, left, Some(new));
        }
    }
}

/// `joint` = None is "(no eye)". The default name is stored as nothing.
fn set_eye_joint_in(rig: &mut Rig, left: bool, joint: Option<&str>) {
    let default = if left { "eye_l" } else { "eye_r" };
    let value = match joint {
        Some(j) if j == default => None,
        Some(j) => Some(j.to_owned()),
        None => Some(String::new()),
    };
    if left {
        rig.eyes.left_joint = value;
    } else {
        rig.eyes.right_joint = value;
    }
}

impl Editor {
    // ------------------------------------------------------------ joints

    /// Why `name` cannot be a (new, or renamed) joint's name; None = fine.
    pub fn joint_name_error(&self, name: &str, renaming: Option<&str>) -> Option<String> {
        match check_name("joint", name) {
            Err(e) => Some(e),
            Ok(n) if renaming == Some(n.as_str()) => None,
            Ok(n) if self.rig.joint_order.contains(&n) => Some(format!("There is already a joint '{n}'")),
            Ok(_) => None,
        }
    }

    /// Adding `name` could add this mirror twin too (`horn_r` -> `horn_l`),
    /// if it does not exist yet.
    pub fn twin_to_add(&self, name: &str) -> Option<String> {
        mirror_name(name.trim()).filter(|t| check_name("joint", t).is_ok() && !self.rig.joint_order.contains(t))
    }

    /// New, unplaced joint(s), inserted after the current joint (at the end
    /// if there is none). The new joint becomes current, so the next canvas
    /// click places it.
    pub fn add_joint(&mut self, name: &str, with_twin: bool) -> Result<String, String> {
        if let Some(e) = self.joint_name_error(name, None) {
            return Err(e);
        }
        let name = name.trim().to_owned();
        let twin = self.twin_to_add(&name).filter(|_| with_twin);
        let at = self
            .current
            .as_deref()
            .and_then(|c| self.rig.joint_order.iter().position(|j| j == c))
            .map_or(self.rig.joint_order.len(), |i| i + 1);
        let rig = self.edit(None);
        rig.joint_order.insert(at, name.clone());
        if let Some(twin) = twin {
            rig.joint_order.insert(at + 1, twin);
        }
        self.current = Some(name.clone());
        self.selected = None;
        Ok(name)
    }

    /// Renaming `old` to `new` could rename its twin alongside:
    /// (`shoulder_l`, `scapula_l`) for `shoulder_r` -> `scapula_r`.
    pub fn twin_rename(&self, old: &str, new: &str) -> Option<(String, String)> {
        let (old_twin, new_twin) = (mirror_name(old)?, mirror_name(new.trim())?);
        let same_side = old.ends_with("_r") == new.trim().ends_with("_r");
        (same_side
            && new.trim() != old
            && self.rig.joint_order.contains(&old_twin)
            && self.joint_name_error(&new_twin, Some(&old_twin)).is_none())
        .then_some((old_twin, new_twin))
    }

    /// Rename a joint in the order list, the placed table, every bone chain
    /// and pivot, the anchors and the eyes.
    pub fn rename_joint(&mut self, old: &str, new: &str, with_twin: bool) -> Result<(), String> {
        if !self.rig.joint_order.iter().any(|j| j == old) {
            return Err(format!("No joint '{old}'"));
        }
        if let Some(e) = self.joint_name_error(new, Some(old)) {
            return Err(e);
        }
        let new = new.trim().to_owned();
        if new == old {
            return Ok(());
        }
        let twin = self.twin_rename(old, &new).filter(|_| with_twin);
        let rig = self.edit(None);
        rename_joint_in(rig, old, &new);
        if let Some((from, to)) = &twin {
            rename_joint_in(rig, from, to);
        }
        let mut renames = vec![(old.to_owned(), new)];
        renames.extend(twin);
        for slot in [&mut self.current, &mut self.selected] {
            if let Some((_, to)) = renames.iter().find(|(from, _)| Some(from) == slot.as_ref()) {
                *slot = Some(to.clone());
            }
        }
        Ok(())
    }

    /// Bones whose chain uses a joint.
    pub fn joint_users(&self, joint: &str) -> Vec<String> {
        self.rig.bones.iter().filter(|b| b.joints.iter().any(|j| j == joint)).map(|b| b.name.clone()).collect()
    }

    /// Remove a joint altogether: from the list, the canvas, every bone's
    /// chain (a bone left with fewer than two joints STAYS, and warns), and
    /// any anchor or eye that named it.
    pub fn delete_joint(&mut self, joint: &str) -> bool {
        let Some(at) = self.rig.joint_order.iter().position(|j| j == joint) else {
            return false;
        };
        (self.drag, self.depth_drag) = (None, None);
        let rig = self.edit(None);
        rig.joint_order.remove(at);
        rig.joints.remove(joint);
        // No joint, no depth override for it.
        rig.depth.remove(joint);
        for bone in &mut rig.bones {
            bone.joints.retain(|j| j != joint);
            if bone.pivot.as_deref() == Some(joint) {
                bone.pivot = None;
            }
        }
        for slot in AnchorSlot::ALL {
            if slot.get(rig) == joint {
                slot.get_mut(rig).clear();
            }
        }
        for left in [true, false] {
            let named = if left { &rig.eyes.left_joint } else { &rig.eyes.right_joint };
            if named.as_deref() == Some(joint) {
                set_eye_joint_in(rig, left, None);
            }
        }
        if self.selected.as_deref() == Some(joint) {
            self.selected = None;
        }
        if self.current.as_deref() == Some(joint) {
            // Stay where we were in the list rather than jumping to the top.
            let order = &self.rig.joint_order;
            self.current = order.get(at).or(order.last()).cloned();
        }
        true
    }

    /// Move a joint one place up or down the click-through list.
    pub fn move_joint(&mut self, joint: &str, up: bool) -> bool {
        let Some(at) = self.rig.joint_order.iter().position(|j| j == joint) else {
            return false;
        };
        let to = if up { at.checked_sub(1) } else { Some(at + 1).filter(|t| *t < self.rig.joint_order.len()) };
        match to {
            Some(to) => {
                self.edit(None).joint_order.swap(at, to);
                true
            }
            None => false,
        }
    }

    // ------------------------------------------------------------- bones

    pub fn selected_bone(&self) -> Option<usize> {
        self.selected_bone
    }

    /// Selecting another bone (or none) also stops picking a chain.
    pub fn select_bone(&mut self, bone: Option<usize>) {
        let bone = bone.filter(|b| *b < self.rig.bones.len());
        if bone != self.selected_bone {
            self.chain_pick = None;
        }
        self.selected_bone = bone;
    }

    pub fn bone_name_error(&self, name: &str, renaming: Option<usize>) -> Option<String> {
        match check_name("bone", name) {
            Err(e) => Some(e),
            Ok(n) => match self.rig.bone_index(&n) {
                Some(i) if Some(i) != renaming => Some(format!("There is already a bone '{n}'")),
                _ => None,
            },
        }
    }

    /// A new bone with no part and an empty chain, hung off the selected
    /// bone (a root if none is selected). It becomes selected and starts
    /// picking its chain.
    pub fn add_bone(&mut self, name: &str) -> Result<usize, String> {
        if let Some(e) = self.bone_name_error(name, None) {
            return Err(e);
        }
        let parent = self.selected_bone.and_then(|b| self.rig.bones.get(b)).map(|b| b.name.clone());
        let rig = self.edit(None);
        rig.bones.push(RigBone {
            name: name.trim().to_owned(),
            part: None,
            parent,
            joints: Vec::new(),
            pivot: None,
            radius_ft: None,
        });
        let index = self.rig.bones.len() - 1;
        self.selected_bone = Some(index);
        self.chain_pick = Some(index);
        Ok(index)
    }

    /// Rename a bone; its children follow.
    pub fn rename_bone(&mut self, bone: usize, new: &str) -> Result<(), String> {
        let Some(old) = self.rig.bones.get(bone).map(|b| b.name.clone()) else {
            return Err("No such bone".to_owned());
        };
        if let Some(e) = self.bone_name_error(new, Some(bone)) {
            return Err(e);
        }
        let new = new.trim().to_owned();
        if new == old {
            return Ok(());
        }
        let rig = self.edit(None);
        for b in rig.bones.iter_mut().filter(|b| b.parent.as_deref() == Some(old.as_str())) {
            b.parent = Some(new.clone());
        }
        rig.bones[bone].name = new;
        Ok(())
    }

    /// Delete a bone; its children re-parent to ITS parent (so they become
    /// roots when a root is deleted). Its joints stay.
    pub fn delete_bone(&mut self, bone: usize) -> bool {
        let Some(gone) = self.rig.bones.get(bone).cloned() else {
            return false;
        };
        let rig = self.edit(None);
        rig.bones.remove(bone);
        for b in rig.bones.iter_mut().filter(|b| b.parent.as_deref() == Some(gone.name.as_str())) {
            b.parent = gone.parent.clone();
        }
        let shift = |slot: &mut Option<usize>| {
            *slot = match *slot {
                Some(b) if b == bone => None,
                Some(b) if b > bone => Some(b - 1),
                other => other,
            }
        };
        shift(&mut self.selected_bone);
        shift(&mut self.chain_pick);
        true
    }

    /// Names a bone may take as its parent: never itself or a descendant.
    pub fn parent_choices(&self, bone: usize) -> Vec<String> {
        self.rig.parent_candidates(bone).into_iter().map(|k| self.rig.bones[k].name.clone()).collect()
    }

    /// `parent` = None makes the bone a root. Refuses (false) a parent that
    /// does not exist or would make a loop.
    pub fn set_bone_parent(&mut self, bone: usize, parent: Option<&str>) -> bool {
        let Some(current) = self.rig.bones.get(bone).map(|b| b.parent.clone()) else {
            return false;
        };
        if parent.is_some_and(|p| !self.parent_choices(bone).iter().any(|c| c == p)) {
            return false;
        }
        if current.as_deref() != parent {
            self.edit(None).bones[bone].parent = parent.map(str::to_owned);
        }
        true
    }

    /// `pivot` = None is "the chain's first joint". Must be on the chain.
    pub fn set_bone_pivot(&mut self, bone: usize, pivot: Option<&str>) -> bool {
        let Some(b) = self.rig.bones.get(bone) else {
            return false;
        };
        if pivot.is_some_and(|p| !b.joints.iter().any(|j| j == p)) {
            return false;
        }
        if b.pivot.as_deref() != pivot {
            self.edit(None).bones[bone].pivot = pivot.map(str::to_owned);
        }
        true
    }

    /// Joints that could still be added to a bone's chain, in list order. A
    /// chain visits a joint at most once.
    pub fn chain_choices(&self, bone: usize) -> Vec<String> {
        let Some(b) = self.rig.bones.get(bone) else {
            return Vec::new();
        };
        self.rig.joint_order.iter().filter(|j| !b.joints.contains(j)).cloned().collect()
    }

    /// Insert `joint` into a bone's chain at `at` (clamped; `usize::MAX`
    /// appends). False if the joint does not exist or is already on it.
    pub fn chain_insert(&mut self, bone: usize, at: usize, joint: &str) -> bool {
        if !self.chain_choices(bone).iter().any(|j| j == joint) {
            return false;
        }
        let chain = &mut self.edit(None).bones[bone].joints;
        chain.insert(at.min(chain.len()), joint.to_owned());
        true
    }

    pub fn chain_remove(&mut self, bone: usize, at: usize) -> bool {
        if self.rig.bones.get(bone).is_none_or(|b| at >= b.joints.len()) {
            return false;
        }
        let b = &mut self.edit(None).bones[bone];
        let gone = b.joints.remove(at);
        if b.pivot.as_deref() == Some(gone.as_str()) {
            b.pivot = None;
        }
        true
    }

    pub fn chain_move(&mut self, bone: usize, at: usize, up: bool) -> bool {
        let len = self.rig.bones.get(bone).map_or(0, |b| b.joints.len());
        let to = if up { at.checked_sub(1) } else { Some(at + 1) };
        match to {
            Some(to) if at < len && to < len => {
                self.edit(None).bones[bone].joints.swap(at, to);
                true
            }
            _ => false,
        }
    }

    /// The bone whose chain canvas clicks are building, if any.
    pub fn chain_pick(&self) -> Option<usize> {
        self.chain_pick
    }

    pub fn set_chain_pick(&mut self, bone: Option<usize>) {
        self.chain_pick = bone.filter(|b| *b < self.rig.bones.len());
        if self.chain_pick.is_some() {
            self.selected_bone = self.chain_pick;
        }
    }

    /// A joint was clicked on the canvas while a chain is being picked:
    /// append it. True if it was added (false: already on the chain).
    pub fn pick_joint(&mut self, joint: &str) -> bool {
        match self.chain_pick {
            Some(bone) => self.chain_insert(bone, usize::MAX, joint),
            None => false,
        }
    }

    // ------------------------------------------------- anchors and eyes

    /// `joint` = "" is "(none)".
    pub fn set_anchor(&mut self, slot: AnchorSlot, joint: &str) -> bool {
        if !joint.is_empty() && !self.rig.joint_order.iter().any(|j| j == joint) {
            return false;
        }
        if slot.get(&self.rig) != joint {
            *slot.get_mut(self.edit(None)) = joint.to_owned();
        }
        true
    }

    /// The joint that is the left / right eye; None = no such eye.
    pub fn eye_joint(&self, left: bool) -> Option<&str> {
        Some(self.rig.eyes.joint(left)).filter(|j| self.rig.joint_order.iter().any(|k| k == j))
    }

    pub fn set_eye_joint(&mut self, left: bool, joint: Option<&str>) -> bool {
        if joint.is_some_and(|j| !self.rig.joint_order.iter().any(|k| k == j)) {
            return false;
        }
        if self.eye_joint(left) != joint {
            set_eye_joint_in(self.edit(None), left, joint);
        }
        true
    }

    /// None = Auto. Consecutive changes are one undo step until
    /// [`Editor::end_continuous_edit`].
    pub fn set_eye_radius(&mut self, radius_ft: Option<f32>) {
        let radius_ft = radius_ft.map(|r| r.max(0.0));
        if self.rig.eye_radius_ft != radius_ft {
            self.edit(Some(EYE_RADIUS_TAG)).eye_radius_ft = radius_ft;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::{template, View};

    fn biped() -> Editor {
        let t = template("biped").unwrap();
        Editor::new(t.new_rig("humanoid", View::Front, 6.0, "crown"), t)
    }

    fn bone(e: &Editor, name: &str) -> usize {
        e.rig().bone_index(name).unwrap()
    }

    #[test]
    fn names_are_lower_snake_case() {
        assert_eq!(check_name("joint", "  horn_tip2 ").as_deref(), Ok("horn_tip2"));
        for bad in ["", "  ", "Horn", "2horn", "_r", "horn-tip", "horn tip", "horn_", "horn__tip", "h\u{f6}rn"] {
            assert!(check_name("joint", bad).is_err(), "{bad:?}");
        }
        assert!(check_name("bone", &"x".repeat(MAX_NAME_LEN + 1)).is_err());
    }

    #[test]
    fn adding_a_joint_inserts_it_after_the_current_one_and_makes_it_current() {
        let mut e = biped();
        e.set_current("chin");
        assert_eq!(e.twin_to_add("horn"), None);
        assert_eq!(e.add_joint(" horn ", true).as_deref(), Ok("horn"));
        let order = e.joint_order();
        assert_eq!(&order[..3], ["crown", "chin", "horn"]);
        assert_eq!(e.current(), Some("horn"));
        assert!(e.add_joint("horn", false).unwrap_err().contains("already"));
        assert!(e.add_joint("Horn!", false).is_err());
        // The next click places it; undo removes placement, then the joint.
        e.place_current([0.0, 6.2]);
        assert!(e.is_placed("horn"));
        e.undo();
        e.undo();
        assert!(!e.joint_order().contains(&"horn".to_owned()));
        assert!(!e.is_dirty());
    }

    #[test]
    fn a_sided_joint_can_bring_its_twin_in_one_step() {
        let mut e = biped();
        assert_eq!(e.twin_to_add("horn_r").as_deref(), Some("horn_l"));
        e.add_joint("horn_r", true).unwrap();
        let order = e.joint_order();
        let at = order.iter().position(|j| j == "horn_r").unwrap();
        assert_eq!(order[at + 1], "horn_l");
        // Mirror placement works for the new pair like any template pair.
        assert_eq!(e.place_current([-0.2, 6.1]), ["horn_r", "horn_l"]);
        e.undo();
        e.undo();
        assert!(!e.can_undo(), "pair added in ONE step");

        e.add_joint("horn_r", false).unwrap();
        assert!(!e.joint_order().contains(&"horn_l".to_owned()));
        assert_eq!(e.twin_to_add("shoulder_r"), None, "twin exists already");
    }

    #[test]
    fn renaming_a_joint_reaches_bones_anchors_eyes_and_placement() {
        let mut e = biped();
        e.place_current([0.0, 5.9]); // crown
        e.place_current([0.0, 5.2]); // chin
        e.set_bone_pivot(bone(&e, "neck"), Some("chin"));
        e.rename_joint("chin", "jaw", false).unwrap();
        let rig = e.rig();
        assert_eq!(rig.joints["jaw"], [0.0, 5.2]);
        assert!(!rig.joints.contains_key("chin") && !rig.joint_order.contains(&"chin".to_owned()));
        assert_eq!(rig.bones[bone(&e, "skull")].joints, ["crown", "jaw"]);
        assert_eq!(rig.bones[bone(&e, "skull")].pivot.as_deref(), Some("jaw"));
        assert_eq!(rig.bones[bone(&e, "neck")].joints, ["jaw", "neck_base"]);
        assert_eq!(rig.anchors.mouth, "jaw");
        assert_eq!(e.selected(), Some("jaw"));
        assert_eq!(rig.joint_order[1], "jaw", "keeps its place in the list");

        assert!(e.rename_joint("jaw", "crown", false).is_err());
        assert!(e.rename_joint("nope", "x", false).is_err());
        assert!(e.rename_joint("jaw", "jaw", false).is_ok());

        // Eyes: the default name is implicit, so renaming must pin the new one.
        e.rename_joint("eye_l", "oculus_l", false).unwrap();
        assert_eq!(e.rig().eyes.left_joint.as_deref(), Some("oculus_l"));
        assert_eq!(e.eye_joint(true), Some("oculus_l"));
        assert_eq!(e.eye_joint(false), Some("eye_r"));
        e.rename_joint("oculus_l", "eye_l", false).unwrap();
        assert_eq!(e.rig().eyes.left_joint, None, "back to the default: nothing stored");
    }

    #[test]
    fn a_sided_rename_can_take_the_twin_along() {
        let mut e = biped();
        assert_eq!(e.twin_rename("shoulder_r", "scapula_r"), Some(("shoulder_l".into(), "scapula_l".into())));
        assert_eq!(e.twin_rename("shoulder_r", "scapula_l"), None, "sides swapped: leave the twin alone");
        assert_eq!(e.twin_rename("shoulder_r", "scapula"), None);
        assert_eq!(e.twin_rename("shoulder_r", "elbow_r"), None);
        e.set_current("shoulder_l");
        e.rename_joint("shoulder_r", "scapula_r", true).unwrap();
        let clavicle = &e.rig().bones[bone(&e, "clavicle")];
        assert_eq!(clavicle.joints, ["scapula_r", "neck_base", "scapula_l"]);
        assert_eq!(e.current(), Some("scapula_l"));
        e.undo();
        assert_eq!(e.rig().bones[bone(&e, "clavicle")].joints, ["shoulder_r", "neck_base", "shoulder_l"]);
        assert!(!e.can_undo());
    }

    #[test]
    fn deleting_a_joint_strips_it_from_everything_but_keeps_the_bones() {
        let mut e = biped();
        e.place_current([0.0, 5.9]);
        e.place_current([0.0, 5.2]);
        assert_eq!(e.joint_users("chin"), ["skull", "neck"]);
        e.set_current("chin");
        assert!(e.delete_joint("chin"));
        assert!(!e.delete_joint("chin"));
        let rig = e.rig();
        assert!(!rig.joints.contains_key("chin"));
        assert_eq!(rig.bones[bone(&e, "skull")].joints, ["crown"], "the bone stays, one joint short");
        assert_eq!(rig.bones[bone(&e, "skull")].pivot, None);
        assert_eq!(rig.anchors.mouth, "");
        assert_eq!(e.current(), Some("neck_base"), "current moves to the joint that took its place");
        assert_eq!(e.selected(), None);

        e.set_eye_joint(true, Some("crown"));
        e.delete_joint("crown");
        assert_eq!(e.eye_joint(true), None);
        e.delete_joint("eye_r");
        assert_eq!(e.eye_joint(false), None);

        for _ in 0..4 {
            assert!(e.undo());
        }
        assert_eq!(e.rig().anchors.mouth, "chin");
        assert_eq!(e.rig().bones[bone(&e, "skull")].joints, ["crown", "chin"]);
    }

    #[test]
    fn joints_move_up_and_down_the_click_order() {
        let mut e = biped();
        assert!(!e.move_joint("crown", true));
        assert!(e.move_joint("crown", false));
        assert_eq!(&e.joint_order()[..2], ["chin", "crown"]);
        assert_eq!(e.current(), Some("crown"), "still the same joint");
        let last = e.joint_order().last().unwrap().clone();
        assert!(!e.move_joint(&last, false));
        assert!(e.move_joint(&last, true));
        assert!(!e.move_joint("nope", true));
        e.undo();
        e.undo();
        assert!(!e.is_dirty());
    }

    #[test]
    fn a_new_bone_hangs_off_the_selected_one_and_picks_its_chain() {
        let mut e = biped();
        e.select_bone(Some(bone(&e, "hand_r")));
        let club = e.add_bone("club").unwrap();
        assert_eq!(e.rig().bones[club].parent.as_deref(), Some("hand_r"));
        assert_eq!((e.selected_bone(), e.chain_pick()), (Some(club), Some(club)));
        assert!(e.add_bone("club").is_err() && e.add_bone("Club").is_err());

        e.add_joint("club_tip", false).unwrap();
        assert!(e.pick_joint("wrist_r"));
        assert!(e.pick_joint("club_tip"));
        assert!(!e.pick_joint("wrist_r"), "a chain visits a joint once");
        assert!(!e.pick_joint("nope"));
        assert_eq!(e.rig().bones[club].joints, ["wrist_r", "club_tip"]);
        assert!(!e.chain_choices(club).contains(&"club_tip".to_owned()));

        // Selecting another bone ends the pick; clicks then append nothing.
        e.select_bone(Some(0));
        assert_eq!(e.chain_pick(), None);
        assert!(!e.pick_joint("elbow_r"));

        e.select_bone(None);
        let root = e.add_bone("halo").unwrap();
        assert_eq!(e.rig().bones[root].parent, None);
    }

    #[test]
    fn chains_can_be_edited_in_place() {
        let mut e = biped();
        let arm = bone(&e, "arm_r");
        assert!(e.chain_insert(arm, 1, "sternum"));
        assert_eq!(e.rig().bones[arm].joints, ["shoulder_r", "sternum", "elbow_r", "wrist_r"]);
        assert!(!e.chain_insert(arm, 0, "sternum"));
        assert!(e.chain_move(arm, 1, false));
        assert!(e.chain_move(arm, 3, true));
        assert_eq!(e.rig().bones[arm].joints, ["shoulder_r", "elbow_r", "wrist_r", "sternum"]);
        assert!(!e.chain_move(arm, 0, true) && !e.chain_move(arm, 3, false) && !e.chain_move(arm, 9, true));
        assert!(e.set_bone_pivot(arm, Some("sternum")));
        assert!(!e.set_bone_pivot(arm, Some("crown")), "pivot must be on the chain");
        assert!(e.chain_remove(arm, 3));
        assert_eq!(e.rig().bones[arm].pivot, None, "removing the pivot joint clears the pivot");
        assert!(!e.chain_remove(arm, 3));
        assert_eq!(e.rig().bones[arm].joints, ["shoulder_r", "elbow_r", "wrist_r"]);
    }

    #[test]
    fn renaming_a_bone_takes_its_children_along() {
        let mut e = biped();
        let arm = bone(&e, "arm_l");
        e.rename_bone(arm, "wing_l").unwrap();
        assert_eq!(e.rig().bones[bone(&e, "hand_l")].parent.as_deref(), Some("wing_l"));
        assert!(e.rename_bone(arm, "arm_r").is_err());
        assert!(e.rename_bone(arm, "wing_l").is_ok());
        assert!(e.rename_bone(99, "x").is_err());
        assert_eq!(e.rig().hierarchy_issues(), []);
        e.undo();
        assert!(!e.is_dirty());
    }

    #[test]
    fn deleting_a_bone_hands_its_children_to_its_parent() {
        let mut e = biped();
        let clavicle = bone(&e, "clavicle");
        e.select_bone(Some(bone(&e, "leg_l")));
        e.set_chain_pick(Some(bone(&e, "leg_l")));
        assert!(e.delete_bone(clavicle));
        assert_eq!(e.rig().bones[bone(&e, "arm_r")].parent.as_deref(), Some("torso_upper"));
        assert_eq!(e.rig().bones[bone(&e, "arm_l")].parent.as_deref(), Some("torso_upper"));
        assert_eq!(e.selected_bone(), Some(bone(&e, "leg_l")), "selection follows the bone, not the index");
        assert_eq!(e.chain_pick(), e.selected_bone());
        assert_eq!(e.rig().hierarchy_issues(), []);

        // Deleting the root orphans its children into roots.
        let root = bone(&e, "torso_lower");
        e.delete_bone(root);
        let roots: Vec<&str> =
            e.rig().bones.iter().filter(|b| b.parent.is_none()).map(|b| b.name.as_str()).collect();
        assert_eq!(roots, ["torso_upper", "leg_r", "leg_l"]);

        let selected = e.selected_bone().unwrap();
        e.delete_bone(selected);
        assert_eq!((e.selected_bone(), e.chain_pick()), (None, None));
        assert!(!e.delete_bone(99));
    }

    #[test]
    fn a_parent_can_never_be_a_descendant() {
        let mut e = biped();
        let clavicle = bone(&e, "clavicle");
        let choices = e.parent_choices(clavicle);
        for banned in ["clavicle", "arm_r", "hand_r", "arm_l", "hand_l"] {
            assert!(!choices.contains(&banned.to_owned()), "{banned}");
        }
        assert!(choices.contains(&"skull".to_owned()) && choices.contains(&"torso_lower".to_owned()));
        assert!(!e.set_bone_parent(clavicle, Some("hand_r")));
        assert!(!e.set_bone_parent(clavicle, Some("clavicle")));
        assert!(!e.set_bone_parent(clavicle, Some("nope")));
        assert!(!e.can_undo());
        assert!(e.set_bone_parent(clavicle, Some("neck")));
        assert!(e.set_bone_parent(clavicle, Some("neck")), "no change is fine");
        assert!(e.set_bone_parent(bone(&e, "skull"), None));
        assert_eq!(e.rig().bones[bone(&e, "skull")].parent, None);
        e.undo();
        e.undo();
        assert!(!e.can_undo());
    }

    #[test]
    fn anchors_and_eyes_name_joints_and_undo() {
        let mut e = biped();
        assert!(e.set_anchor(AnchorSlot::Saddle, "sternum"));
        assert_eq!(e.rig().anchors.saddle, "sternum");
        assert!(!e.set_anchor(AnchorSlot::Head, "nope"));
        assert!(e.set_anchor(AnchorSlot::Mouth, ""));
        assert_eq!(AnchorSlot::Mouth.get(e.rig()), "");

        assert!(e.set_eye_joint(false, Some("crown")));
        assert_eq!(e.eye_joint(false), Some("crown"));
        assert!(e.set_eye_joint(false, None));
        assert_eq!(e.eye_joint(false), None);
        assert!(e.set_eye_joint(false, Some("eye_r")));
        assert_eq!(e.rig().eyes.right_joint, None);
        assert!(!e.set_eye_joint(true, Some("nope")));

        let steps_before = 5;
        for r in [0.05, 0.06, 0.07] {
            e.set_eye_radius(Some(r));
        }
        e.end_continuous_edit();
        e.set_eye_radius(Some(-1.0));
        assert_eq!(e.rig().eye_radius_ft, Some(0.0));
        e.undo();
        assert_eq!(e.rig().eye_radius_ft, Some(0.07));
        e.undo();
        assert_eq!(e.rig().eye_radius_ft, None, "one drag, one step");
        for _ in 0..steps_before {
            assert!(e.undo());
        }
        assert!(!e.can_undo() && !e.is_dirty());
    }

    #[test]
    fn undo_drops_a_stale_bone_selection() {
        let mut e = biped();
        let halo = e.add_bone("halo").unwrap();
        assert_eq!(e.chain_pick(), Some(halo));
        e.undo();
        assert_eq!((e.selected_bone(), e.chain_pick()), (None, None));
    }
}
