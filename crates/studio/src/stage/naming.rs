//! New-scene naming. Ported from VellumFE `src/frontend/gui/studio.rs`
//! (`StudioRoomContext`, `NewSceneState`) at commit 60c84a79.

use creature_field::scenes::{self, SceneBinding};

/// Room facts handed over on the command line (`--room-uid`, `--room-title`,
/// `--room-location`) -- prefill for the new-scene inputs, never applied to
/// anything on its own. Without them everything is `None` and the tool
/// stays fully usable.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StudioRoomContext {
    pub uid: Option<i64>,
    pub title: Option<String>,
    pub location: Option<String>,
}

/// New-scene naming state: an explicit binding choice plus typed inputs,
/// composed into a filename stem through the scene path rules and
/// previewed BEFORE anything is saved.
pub struct NewSceneState {
    pub binding: SceneBinding,
    /// Room uid input (RoomUid binding), typed or prefilled.
    pub uid_text: String,
    /// Name input: uid garnish for RoomUid, the matched text otherwise.
    pub text: String,
    /// Launch-time room facts, kept so switching bindings can re-prefill.
    context: StudioRoomContext,
}

impl NewSceneState {
    /// Prefill the INPUTS from the room context (uid binding when a uid is
    /// known, else title, else location, else default). Only inputs: the
    /// loaded scene and its name are untouched, so launching with room
    /// context never silently overwrites a saved scene.
    pub fn new(context: StudioRoomContext) -> Self {
        let binding = if context.uid.is_some() {
            SceneBinding::RoomUid
        } else if context.title.is_some() {
            SceneBinding::RoomTitle
        } else if context.location.is_some() {
            SceneBinding::Location
        } else {
            SceneBinding::Default
        };
        let mut state = Self { binding, uid_text: String::new(), text: String::new(), context };
        state.prefill_inputs();
        state
    }

    /// Seed the inputs for the current binding from the room context.
    pub fn prefill_inputs(&mut self) {
        self.uid_text = self.context.uid.map(|u| u.to_string()).unwrap_or_default();
        self.text = match self.binding {
            SceneBinding::RoomUid | SceneBinding::RoomTitle => self.context.title.clone().unwrap_or_default(),
            SceneBinding::Location => self.context.location.clone().unwrap_or_default(),
            SceneBinding::Default => String::new(),
        };
    }

    /// Switch binding (a no-op when unchanged) and re-prefill.
    pub fn set_binding(&mut self, binding: SceneBinding) {
        if self.binding != binding {
            self.binding = binding;
            self.prefill_inputs();
        }
    }

    /// The filename stem these inputs compose to, or why they don't.
    pub fn preview(&self) -> Result<String, String> {
        scenes::compose_stem(self.binding, &self.uid_text, &self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::test_support::TempBase;

    #[test]
    fn new_scene_prefill_fills_inputs_and_never_touches_saved_scenes() {
        let _base = TempBase::new();

        // A saved scene for the room the client is standing in.
        let saved = scenes::StageScene { background: Some("scenes/desert.png".into()), ..Default::default() };
        saved.save("47009_-_Kitchen_Garden").unwrap();
        let on_disk = std::fs::read_to_string(scenes::StageScene::path("47009_-_Kitchen_Garden").unwrap()).unwrap();

        let state = NewSceneState::new(StudioRoomContext {
            uid: Some(47009),
            title: Some("Kitchen Garden".into()),
            location: Some("Castle Anwyn".into()),
        });
        // Prefill picked the uid binding and seeded the inputs.
        assert_eq!(state.binding, SceneBinding::RoomUid);
        assert_eq!(state.uid_text, "47009");
        assert_eq!(state.text, "Kitchen Garden");
        // The preview names the EXISTING scene, flagged as a replace...
        let stem = state.preview().unwrap();
        assert_eq!(stem, "47009_-_Kitchen_Garden");
        assert!(scenes::scene_exists(&stem));
        // ...and nothing was written: prefill never silently overwrites.
        let after = std::fs::read_to_string(scenes::StageScene::path("47009_-_Kitchen_Garden").unwrap()).unwrap();
        assert_eq!(on_disk, after);
    }

    #[test]
    fn new_scene_standalone_defaults_and_rebinding_reprefills() {
        // Standalone (no context): default binding, empty inputs, valid
        // "default" preview out of the box.
        let mut state = NewSceneState::new(StudioRoomContext::default());
        assert_eq!(state.binding, SceneBinding::Default);
        assert_eq!(state.preview().unwrap(), "default");
        // Typed inputs compose without any game connection.
        state.binding = SceneBinding::RoomTitle;
        state.text = "Barley Field".into();
        assert_eq!(state.preview().unwrap(), "Barley_Field");
        state.binding = SceneBinding::RoomUid;
        state.uid_text = "12".into();
        state.text.clear();
        assert_eq!(state.preview().unwrap(), "12");
        // Unsafe input is rejected at preview, before any save exists.
        state.uid_text = "../evil".into();
        assert!(state.preview().is_err());
        // Context-driven rebinding re-prefills the matching input.
        let mut state = NewSceneState::new(StudioRoomContext {
            uid: None,
            title: Some("Barley Field".into()),
            location: Some("Wehnimer's Landing".into()),
        });
        assert_eq!(state.binding, SceneBinding::RoomTitle);
        assert_eq!(state.text, "Barley Field");
        state.binding = SceneBinding::Location;
        state.prefill_inputs();
        assert_eq!(state.text, "Wehnimer's Landing");
    }

    #[test]
    fn set_binding_reprefills_only_on_a_real_change() {
        let mut state = NewSceneState::new(StudioRoomContext {
            uid: None,
            title: Some("Barley Field".into()),
            location: Some("Wehnimer's Landing".into()),
        });
        state.text = "typed by hand".into();
        state.set_binding(SceneBinding::RoomTitle);
        assert_eq!(state.text, "typed by hand", "same binding: typed input survives");
        state.set_binding(SceneBinding::Location);
        assert_eq!(state.text, "Wehnimer's Landing");
    }
}
