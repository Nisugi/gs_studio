//! The "type a name" prompt behind Add / Rename for joints and bones: what
//! it is called, whether the text will do, what the mirror-twin checkbox
//! offers, and what OK does. The egui layer only draws it.

use super::state::Editor;

#[derive(Debug, Clone, PartialEq)]
pub enum PromptKind {
    AddJoint,
    RenameJoint(String),
    AddBone,
    RenameBone(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub struct NamePrompt {
    pub kind: PromptKind,
    pub text: String,
    /// The "and its mirror twin" box (only shown when [`NamePrompt::twin_offer`]
    /// has something to offer).
    pub with_twin: bool,
}

impl NamePrompt {
    /// Rename prompts start from the current name.
    pub fn new(kind: PromptKind, editor: &Editor) -> NamePrompt {
        let text = match &kind {
            PromptKind::RenameJoint(j) => j.clone(),
            PromptKind::RenameBone(b) => editor.rig().bones.get(*b).map(|b| b.name.clone()).unwrap_or_default(),
            PromptKind::AddJoint | PromptKind::AddBone => String::new(),
        };
        NamePrompt { kind, text, with_twin: true }
    }

    pub fn title(&self, editor: &Editor) -> String {
        match &self.kind {
            PromptKind::AddJoint => "Add joint".to_owned(),
            PromptKind::RenameJoint(j) => format!("Rename joint '{j}'"),
            PromptKind::AddBone => match editor.selected_bone().and_then(|b| editor.rig().bones.get(b)) {
                Some(parent) => format!("Add bone under '{}'", parent.name),
                None => "Add root bone".to_owned(),
            },
            PromptKind::RenameBone(b) => {
                format!("Rename bone '{}'", editor.rig().bones.get(*b).map_or("?", |b| b.name.as_str()))
            }
        }
    }

    /// Why OK is disabled; None = the name will do.
    pub fn error(&self, editor: &Editor) -> Option<String> {
        match &self.kind {
            PromptKind::AddJoint => editor.joint_name_error(&self.text, None),
            PromptKind::RenameJoint(j) => editor.joint_name_error(&self.text, Some(j)),
            PromptKind::AddBone => editor.bone_name_error(&self.text, None),
            PromptKind::RenameBone(b) => editor.bone_name_error(&self.text, Some(*b)),
        }
    }

    /// Checkbox text for taking the `_r` / `_l` twin along, when there is
    /// one to take.
    pub fn twin_offer(&self, editor: &Editor) -> Option<String> {
        match &self.kind {
            PromptKind::AddJoint => editor.twin_to_add(&self.text).map(|t| format!("Also add its mirror twin '{t}'")),
            PromptKind::RenameJoint(j) => {
                editor.twin_rename(j, &self.text).map(|(from, to)| format!("Also rename '{from}' to '{to}'"))
            }
            PromptKind::AddBone | PromptKind::RenameBone(_) => None,
        }
    }

    /// OK was pressed. One undo step, whatever it does.
    pub fn apply(&self, editor: &mut Editor) -> Result<(), String> {
        match &self.kind {
            PromptKind::AddJoint => editor.add_joint(&self.text, self.with_twin).map(|_| ()),
            PromptKind::RenameJoint(j) => editor.rename_joint(j, &self.text, self.with_twin),
            PromptKind::AddBone => editor.add_bone(&self.text).map(|_| ()),
            PromptKind::RenameBone(b) => editor.rename_bone(*b, &self.text),
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

    #[test]
    fn an_add_prompt_validates_live_and_offers_the_twin() {
        let mut e = biped();
        let mut p = NamePrompt::new(PromptKind::AddJoint, &e);
        assert_eq!(p.title(&e), "Add joint");
        assert!(p.error(&e).is_some(), "empty");
        p.text = "crown".into();
        assert_eq!(p.error(&e).as_deref(), Some("There is already a joint 'crown'"));
        p.text = "horn_r".into();
        assert_eq!(p.error(&e), None);
        assert_eq!(p.twin_offer(&e).as_deref(), Some("Also add its mirror twin 'horn_l'"));
        p.apply(&mut e).unwrap();
        assert!(e.joint_order().contains(&"horn_l".to_owned()));
        assert!(p.apply(&mut e).is_err(), "applying twice: the name is taken now");
    }

    #[test]
    fn a_rename_prompt_starts_from_the_name_and_accepts_it_unchanged() {
        let mut e = biped();
        let mut p = NamePrompt::new(PromptKind::RenameJoint("hip_r".into()), &e);
        assert_eq!((p.text.as_str(), p.error(&e)), ("hip_r", None));
        assert_eq!(p.twin_offer(&e), None);
        p.text = "haunch_r".into();
        assert_eq!(p.twin_offer(&e).as_deref(), Some("Also rename 'hip_l' to 'haunch_l'"));
        p.with_twin = false;
        p.apply(&mut e).unwrap();
        assert!(e.joint_order().contains(&"hip_l".to_owned()) && e.joint_order().contains(&"haunch_r".to_owned()));

        let arm = e.rig().bone_index("arm_r").unwrap();
        let mut p = NamePrompt::new(PromptKind::RenameBone(arm), &e);
        assert_eq!(p.title(&e), "Rename bone 'arm_r'");
        assert_eq!(p.error(&e), None);
        p.text = "arm_l".into();
        assert!(p.error(&e).is_some());
        p.text = "wing_r".into();
        p.apply(&mut e).unwrap();
        assert_eq!(e.rig().bones[arm].name, "wing_r");
    }

    #[test]
    fn a_bone_prompt_says_where_the_bone_will_hang() {
        let mut e = biped();
        assert_eq!(NamePrompt::new(PromptKind::AddBone, &e).title(&e), "Add root bone");
        e.select_bone(e.rig().bone_index("hand_l"));
        let mut p = NamePrompt::new(PromptKind::AddBone, &e);
        assert_eq!(p.title(&e), "Add bone under 'hand_l'");
        p.text = "claw_l".into();
        p.apply(&mut e).unwrap();
        assert_eq!(e.rig().bones.last().unwrap().parent.as_deref(), Some("hand_l"));
    }
}
