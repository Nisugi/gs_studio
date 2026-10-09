//! Notes pinned to a form's puppet in the Rigs tab's 3D preview, without
//! the drawing: "reduce this bulge", "the jaw hangs and wobbles" -- each on
//! the bone it was clicked on, part way along it, with the pose, the
//! creature and the view it was seen in. `<data>/notes/<family>.toml`, kept
//! in git beside the rig.
//!
//! They are for whoever works on the rig next, Claude included: a note names
//! the bone, how far along it, the pose and the view, where a screenshot
//! names only pixels. `GS_STUDIO_NOTE=<family>#<n>` opens the Rigs tab's
//! preview on note `n` (its pose, creature and view, the pin marked), for a
//! screenshot of exactly what it points at.

use std::path::{Path, PathBuf};

use gs_puppet::math::V3;
use gs_puppet::Posed;
use serde::{Deserialize, Serialize};

pub use crate::notes::today;
use crate::notes::Reply;

/// One pinned note.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    /// Its number within the family, shown on its pin; never reused.
    pub id: u32,
    pub text: String,
    /// Who wrote it (`notes::author`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub by: String,
    /// The bone it is pinned to.
    pub bone: String,
    /// Which of the bone's segments, when the bone is a chain (a tail, a
    /// finger): its index in the chain.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub chain: u32,
    /// How far along the segment: its head 0 .. its tail 1.
    pub along: f32,
    /// The pose it was seen in ("" = rest, as drawn).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pose: String,
    /// The creature shown (the family's puppet in its outfit, in its own
    /// sheet), "" = the family bare.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub creature: String,
    /// The view it was pinned from: turn, tilt (degrees) and zoom.
    pub view: [f32; 3],
    /// Dealt with: kept in the list, no longer pinned on the figure.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub done: bool,
    /// When it was made, `YYYY-MM-DD`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub made: String,
    /// The conversation under it.
    #[serde(default, rename = "reply", skip_serializing_if = "Vec::is_empty")]
    pub replies: Vec<Reply>,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

/// A family's notes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Notes {
    #[serde(default, rename = "note")]
    pub notes: Vec<Note>,
}

/// Where a note goes before it has an id and a date.
#[derive(Debug, Clone, PartialEq)]
pub struct Pin {
    pub bone: String,
    pub chain: u32,
    pub along: f32,
    pub pose: String,
    pub creature: String,
    pub view: [f32; 3],
}

impl Pin {
    /// The pin under a screen point on `posed` (see
    /// [`crate::editor::pose_edit::pick_segment`]), seen in `pose` from
    /// `view`; None off the figure.
    pub fn at(posed: &Posed, segment: usize, along: f32, pose: &str, creature: &str, view: [f32; 3]) -> Option<Pin> {
        let s = posed.puppet.segments.get(segment)?;
        Some(Pin { bone: s.bone.clone(), chain: s.chain_index as u32, along: along.clamp(0.0, 1.0), pose: pose.to_owned(), creature: creature.to_owned(), view })
    }

    /// As a note with no id or text yet: where it will sit ([`anchor`]).
    pub fn note(&self) -> Note {
        Note {
            id: 0,
            text: String::new(),
            by: String::new(),
            bone: self.bone.clone(),
            chain: self.chain,
            along: self.along,
            pose: self.pose.clone(),
            creature: self.creature.clone(),
            view: self.view,
            done: false,
            made: String::new(),
            replies: Vec::new(),
        }
    }
}

const HEADER: &str = "# Notes pinned to this form's puppet in gs-studio's Rigs tab (3D preview, Pin notes):\n# each on a bone, `along` it from its head (0) to its tail (1), seen in `pose` from\n# `view` = [turn, tilt, zoom]. GS_STUDIO_NOTE=<family>#<id> opens the preview on one.\n\n";

impl Notes {
    /// `<data>/notes/<family>.toml`.
    pub fn path(data_root: &Path, family: &str) -> PathBuf {
        data_root.join("notes").join(format!("{family}.toml"))
    }

    /// The family's notes; none when it has no file yet.
    pub fn load(data_root: &Path, family: &str) -> Result<Notes, String> {
        let path = Notes::path(data_root, family);
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Notes::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Write the file (a temporary beside it, then renamed over it, so a
    /// failed write never leaves half a file).
    pub fn save(&self, data_root: &Path, family: &str) -> Result<PathBuf, String> {
        let path = Notes::path(data_root, family);
        let text = format!("{HEADER}{}", toml::to_string_pretty(self).map_err(|e| e.to_string())?);
        crate::notes::write_atomic(&path, &text)?;
        Ok(path)
    }

    /// Pin a new note; its id, one past the highest there has been.
    pub fn add(&mut self, pin: Pin, text: &str, by: &str, made: &str) -> u32 {
        let id = self.notes.iter().map(|n| n.id).max().unwrap_or(0) + 1;
        self.notes.push(Note {
            id,
            text: text.trim().to_owned(),
            by: by.trim().to_owned(),
            bone: pin.bone,
            chain: pin.chain,
            along: pin.along,
            pose: pin.pose,
            creature: pin.creature,
            view: pin.view,
            done: false,
            made: made.to_owned(),
            replies: Vec::new(),
        });
        id
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

    pub fn get(&self, id: u32) -> Option<&Note> {
        self.notes.iter().find(|n| n.id == id)
    }

    pub fn remove(&mut self, id: u32) -> bool {
        let before = self.notes.len();
        self.notes.retain(|n| n.id != id);
        self.notes.len() != before
    }

    /// The ones not yet dealt with.
    pub fn open(&self) -> impl Iterator<Item = &Note> {
        self.notes.iter().filter(|n| !n.done)
    }
}

/// Where a note sits on the posed figure (the point `along` its bone's
/// segment, before the preview turns it), or None when the rig has no such
/// bone any more.
pub fn anchor(posed: &Posed, note: &Note) -> Option<V3> {
    let (i, s) = posed.puppet.segments.iter().enumerate().find(|(_, s)| s.bone == note.bone && s.chain_index as u32 == note.chain)?;
    let t = note.along.clamp(0.0, 1.0);
    let rest = [0, 1, 2].map(|k| s.head[k] + (s.tail[k] - s.head[k]) * t);
    Some(posed.point(i, rest))
}

/// Where a note's pin lands on the preview: its [`anchor`], turned by the
/// view's `yaw` as the preview turns the figure, through `project` (world
/// point -> screen point and depth, as the bone picker takes it).
pub fn on_screen(posed: &Posed, note: &Note, yaw: f32, project: &dyn Fn(V3) -> ([f32; 2], f32)) -> Option<[f32; 2]> {
    let p = anchor(posed, note)?;
    Some(project(gs_puppet::math::mul_mv(&gs_puppet::math::rot_y(yaw), p)).0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gs_puppet_author::prelude::*;

    fn humanoid() -> gs_puppet::Puppet {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let outlines = rig::Outlines::load(&repo.join("outlines.json")).unwrap();
        let rig = rig::Rig::load(&repo.join("rigs").join(rig::Rig::file_name("humanoid"))).unwrap();
        gs_puppet::Puppet::build(&rig, &outlines).unwrap()
    }

    /// A note pins to the bone under the click, part way along it; it
    /// saves, loads back the same, and sits on that spot of the figure in
    /// any pose -- the shoulder note moves with the raised arm.
    #[test]
    fn a_note_pins_to_its_bone_and_follows_it() {
        let data = std::env::temp_dir().join(format!("gs_studio_notes_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let p = humanoid();
        let rest = p.pose(&gs_puppet::Pose::default());
        let arm = p.segments.iter().position(|s| s.bone == "arm_l" && s.chain_index == 0).expect("an upper arm");
        let pin = Pin::at(&rest, arm, 0.1, "", "", [35.0, 12.0, 1.0]).expect("on the figure");
        assert_eq!((pin.bone.as_str(), pin.chain), ("arm_l", 0));
        let mut notes = Notes::load(&data, "humanoid").expect("no file: none");
        assert!(notes.notes.is_empty());
        let id = notes.add(pin, "  a bulge on the shoulder  ", "Nisugi", "2026-09-28");
        assert!(notes.reply(id, Reply::new("Claude", "trimmed both arms alike", "2026-09-28")));
        assert_eq!(id, 1);
        notes.save(&data, "humanoid").unwrap();
        let back = Notes::load(&data, "humanoid").unwrap();
        assert_eq!(back, notes);
        assert_eq!(back.get(1).unwrap().text, "a bulge on the shoulder");
        assert_eq!((back.get(1).unwrap().by.as_str(), back.get(1).unwrap().replies.len()), ("Nisugi", 1));
        // the pin follows the bone: raised, the arm's shoulder end moves
        let at_rest = anchor(&rest, back.get(1).unwrap()).unwrap();
        let set = gs_puppet::PoseSet::from_toml(gs_puppet::SHIPPED_POSES.iter().find(|(n, _)| *n == "biped").unwrap().1).unwrap();
        let raised = p.pose(set.get("arms_up").expect("a pose with arms up"));
        let posed_at = anchor(&raised, back.get(1).unwrap()).unwrap();
        assert!((0..3).any(|k| (posed_at[k] - at_rest[k]).abs() > 1e-3), "the pin stayed behind: {at_rest:?} {posed_at:?}");
        // ids are never reused, done notes leave the figure but stay listed
        let mut notes = back;
        let two = notes.add(Pin::at(&rest, arm, 0.9, "stand", "", [0.0, 0.0, 1.0]).unwrap(), "elbow", "", "2026-09-28");
        assert!(notes.remove(1));
        let three = notes.add(Pin::at(&rest, arm, 0.5, "", "", [0.0, 0.0, 1.0]).unwrap(), "mid", "", "2026-09-28");
        assert_eq!((two, three), (2, 3));
        notes.notes.iter_mut().find(|n| n.id == 2).unwrap().done = true;
        assert_eq!(notes.open().map(|n| n.id).collect::<Vec<_>>(), vec![3]);
        let _ = std::fs::remove_dir_all(&data);
    }
}
