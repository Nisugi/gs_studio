//! Notes pinned to a scene, without the drawing: "the fire lights nothing
//! behind the bench", "too dark by the door at night", "the moon sits on
//! the roof" -- each on the spot of the stage it was clicked on, about the
//! lamp or prop there if there was one, with the hour and the weather it
//! was seen in, who wrote it, and its replies.
//! `global/scenes/notes/<scene>.toml`, beside the scene file, so a scene's
//! notes travel with it.
//!
//! For whoever works on the scene next, Claude included: a note names the
//! lamp, the prop, the spot and the moment, where a screenshot names only
//! pixels.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::notes::Reply;

/// One pinned note.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneNote {
    /// Its number within the scene, shown on its pin; never reused.
    pub id: u32,
    pub text: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub by: String,
    /// Where it is pinned: stage pixels (the 880 x 470 stage every scene is
    /// drawn on), so a note on the sky or a wall sits where it was put.
    pub at: [f32; 2],
    /// What it is about, when the click was on one: "Campfire 1", a prop's
    /// image name. Its index too, to select it on Show; the name says what
    /// it was if the list has changed since.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub about: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lamp: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prop: Option<usize>,
    /// The game hour it was seen at, when the scene was lit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hour: Option<f32>,
    /// The weather it was seen in ("" = clear).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub weather: String,
    /// Dealt with: kept in the list, no longer pinned on the stage.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub done: bool,
    /// When it was made, `YYYY-MM-DD`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub made: String,
    #[serde(default, rename = "reply", skip_serializing_if = "Vec::is_empty")]
    pub replies: Vec<Reply>,
}

/// Where a note goes before it has text, an id and a date.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScenePin {
    pub at: [f32; 2],
    pub about: String,
    pub lamp: Option<usize>,
    pub prop: Option<usize>,
    pub hour: Option<f32>,
    pub weather: String,
}

/// A lamp kind's name, as the Lamps list and a note's `about` show it.
pub fn lamp_label(kind: gs_field::lighting::LampKind) -> &'static str {
    use gs_field::lighting::LampKind;
    match kind {
        LampKind::Campfire => "Campfire",
        LampKind::Torch => "Torch",
        LampKind::Glow => "Glow",
        LampKind::Lantern => "Lantern",
        LampKind::Wisp => "Wisp",
    }
}

/// A scene's notes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SceneNotes {
    #[serde(default, rename = "note")]
    pub notes: Vec<SceneNote>,
}

const HEADER: &str = "# Notes pinned to this scene in gs-studio (Scenes / Preview, Notes, Pin notes):\n# each at `at` = [x, y] on the 880 x 470 stage, `about` the lamp or prop there,\n# seen at `hour` in `weather`; replies under it.\n\n";

impl SceneNotes {
    /// `global/scenes/notes/<scene>.toml` in the pool.
    pub fn path(scene: &str) -> Result<PathBuf, String> {
        let scene = scene.trim();
        if scene.is_empty() || scene.contains(['/', '\\']) {
            return Err(format!("no notes for an unnamed scene ('{scene}'): save it first"));
        }
        let dir = gs_field::data_dirs::scenes_dir().map_err(|e| e.to_string())?;
        Ok(dir.join("notes").join(format!("{scene}.toml")))
    }

    /// The scene's notes; none when it has no file yet.
    pub fn load(scene: &str) -> Result<SceneNotes, String> {
        let path = SceneNotes::path(scene)?;
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SceneNotes::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn save(&self, scene: &str) -> Result<PathBuf, String> {
        let path = SceneNotes::path(scene)?;
        let text = format!("{HEADER}{}", toml::to_string_pretty(self).map_err(|e| e.to_string())?);
        crate::notes::write_atomic(&path, &text)?;
        Ok(path)
    }

    /// Pin a new note; its id, one past the highest there has been.
    pub fn add(&mut self, pin: ScenePin, text: &str, by: &str, made: &str) -> u32 {
        let id = self.notes.iter().map(|n| n.id).max().unwrap_or(0) + 1;
        let ScenePin { at, about, lamp, prop, hour, weather } = pin;
        self.notes.push(SceneNote {
            id,
            text: text.trim().to_owned(),
            by: by.trim().to_owned(),
            at,
            about,
            lamp,
            prop,
            hour,
            weather,
            done: false,
            made: made.to_owned(),
            replies: Vec::new(),
        });
        id
    }

    pub fn get(&self, id: u32) -> Option<&SceneNote> {
        self.notes.iter().find(|n| n.id == id)
    }

    /// Add a reply under note `id`; false when there is no such note or
    /// nothing to say.
    pub fn reply(&mut self, id: u32, reply: Reply) -> bool {
        match self.notes.iter_mut().find(|n| n.id == id) {
            Some(note) if !reply.text.is_empty() => {
                note.replies.push(reply);
                true
            }
            _ => false,
        }
    }

    pub fn remove(&mut self, id: u32) -> bool {
        let before = self.notes.len();
        self.notes.retain(|n| n.id != id);
        self.notes.len() != before
    }

    /// The ones not yet dealt with.
    pub fn open(&self) -> impl Iterator<Item = &SceneNote> {
        self.notes.iter().filter(|n| !n.done)
    }
}

/// The stage's Notes section, without the drawing: the notes of the scene
/// on the stage (read again when another scene comes up), pinning, the note
/// being written, a reply being written, the one last shown.
#[derive(Debug, Default)]
pub struct NotesDesk {
    /// Whose notes are loaded, and them (or why not).
    loaded: Option<(String, Result<SceneNotes, String>)>,
    /// A click on the stage pins a note (instead of targeting or dragging).
    pub pinning: bool,
    /// The note being written: where, and its text so far.
    pub pending: Option<(ScenePin, String)>,
    pub draft: Option<(u32, String)>,
    pub focus: Option<u32>,
    /// Hide the done notes from the list.
    pub hide_done: bool,
}

impl NotesDesk {
    /// Load `scene`'s notes if they are not the ones held; another scene
    /// drops whatever was being written.
    pub fn sync(&mut self, scene: &str) {
        let scene = scene.trim();
        if self.loaded.as_ref().is_some_and(|(s, _)| s == scene) {
            return;
        }
        let notes = if scene.is_empty() { Err("Save the scene first: its notes are kept beside it.".to_owned()) } else { SceneNotes::load(scene) };
        self.loaded = Some((scene.to_owned(), notes));
        self.pending = None;
        self.draft = None;
        self.focus = None;
    }

    /// The loaded notes, or why there are none.
    pub fn notes(&self) -> Result<&SceneNotes, &str> {
        match &self.loaded {
            Some((_, Ok(n))) => Ok(n),
            Some((_, Err(e))) => Err(e),
            None => Err("no scene"),
        }
    }

    /// Change the notes and write them at once; the path written, or why not.
    pub fn edit(&mut self, change: impl FnOnce(&mut SceneNotes)) -> Result<PathBuf, String> {
        let Some((scene, Ok(notes))) = self.loaded.as_mut() else {
            return Err("no notes to change".to_owned());
        };
        change(notes);
        notes.save(scene)
    }

    /// Pin the note being written, signed `by`; its id.
    pub fn pin_pending(&mut self, by: &str, made: &str) -> Result<u32, String> {
        let Some((pin, text)) = self.pending.take() else { return Err("nothing being written".to_owned()) };
        let mut id = 0;
        self.edit(|n| id = n.add(pin, &text, by, made))?;
        self.focus = Some(id);
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::test_support::TempBase;

    #[test]
    fn the_desk_follows_the_scene_and_writes_at_once() {
        let _base = TempBase::new();
        let mut desk = NotesDesk::default();
        desk.sync("");
        assert!(desk.notes().is_err(), "an unsaved scene keeps no notes");
        desk.sync("bonfire");
        assert!(desk.notes().unwrap().notes.is_empty());
        desk.pending = Some((pin(), "too dark by the door".into()));
        let id = desk.pin_pending("Nisugi", "2026-09-28").unwrap();
        assert_eq!((id, desk.focus), (1, Some(1)));
        assert_eq!(SceneNotes::load("bonfire").unwrap().notes.len(), 1, "written at once");
        desk.pending = Some((pin(), "half written".into()));
        desk.sync("henge");
        assert!(desk.pending.is_none() && desk.focus.is_none(), "another scene drops the draft");
        desk.sync("bonfire");
        assert_eq!(desk.notes().unwrap().get(1).unwrap().by, "Nisugi");
    }

    fn pin() -> ScenePin {
        ScenePin { at: [440.0, 300.0], about: "Campfire 1".into(), lamp: Some(0), prop: None, hour: Some(23.0), weather: String::new() }
    }

    /// A note saves beside the scene and loads back the same, replies and
    /// all; ids are never reused; done notes leave the stage but stay
    /// listed; an unnamed scene has nowhere to keep them.
    #[test]
    fn scene_notes_round_trip_beside_the_scene() {
        let _base = TempBase::new();
        assert!(SceneNotes::path("").is_err());
        let mut notes = SceneNotes::load("7503305 - Bonfire").expect("no file: none");
        assert!(notes.notes.is_empty());
        let one = notes.add(pin(), "  the bench takes no firelight  ", "Nisugi", "2026-09-28");
        assert!(notes.reply(one, Reply::new("Claude", "props go through the GPU pass now", "2026-09-28")));
        assert!(!notes.reply(one, Reply::new("Claude", "   ", "2026-09-28")), "nothing to say");
        assert!(!notes.reply(9, Reply::new("Claude", "x", "2026-09-28")), "no such note");
        let path = notes.save("7503305 - Bonfire").unwrap();
        assert!(path.ends_with("scenes/notes/7503305 - Bonfire.toml"), "{}", path.display());
        let back = SceneNotes::load("7503305 - Bonfire").unwrap();
        assert_eq!(back, notes);
        assert_eq!(back.get(1).unwrap().text, "the bench takes no firelight");
        assert_eq!(back.get(1).unwrap().replies.len(), 1);
        // the notes folder is not a scene
        assert!(!gs_field::scenes::list_scenes().iter().any(|s| s.contains("notes")));

        let mut notes = back;
        let two = notes.add(ScenePin::default(), "b", "", "2026-09-28");
        assert!(notes.remove(one));
        let three = notes.add(ScenePin::default(), "c", "", "2026-09-28");
        assert_eq!((two, three), (2, 3));
        notes.notes.iter_mut().find(|n| n.id == 2).unwrap().done = true;
        assert_eq!(notes.open().map(|n| n.id).collect::<Vec<_>>(), vec![3]);
    }
}
