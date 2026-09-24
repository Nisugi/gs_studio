//! The rig editor's 3D puppet preview, without the drawing: the orbit
//! camera, which pose is shown, and the pose sets it comes from.
//!
//! Poses are read from `<data>/poses/<body_type>.toml` -- the files Publish
//! ships, so an edit there shows here on *Reload poses* -- falling back to
//! the copies compiled into `gs_puppet` when the data root has none. The
//! family's own `[family.<name>]` overrides are folded in, exactly as the
//! game's renderer does.

use std::path::Path;

use gs_puppet::{Pose, PoseSet};

/// Where the shown pose set came from, for the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoseSource {
    /// `<data>/poses/<body_type>.toml`.
    DataRoot,
    /// Compiled into `gs_puppet` (the data root has no such file).
    Shipped,
}

#[derive(Debug, Clone)]
struct Loaded {
    body_type: String,
    family: String,
    result: Result<(PoseSet, PoseSource), String>,
}

#[derive(Debug, Clone)]
pub struct PuppetView {
    /// The preview panel is open.
    pub show: bool,
    /// Turns the figure, degrees. 0 faces the viewer.
    pub yaw: f32,
    /// Camera elevation, degrees.
    pub pitch: f32,
    pub zoom: f32,
    /// The pose shown, by name; "" = the rest pose (as clicked).
    pub pose: String,
    /// Draw the smooth skin (`Posed::skin`) instead of the shape list.
    pub skin: bool,
    /// Dress the skin in the family's painted texture (`editor::texture`).
    pub texture: bool,
    loaded: Option<Loaded>,
}

pub const PITCH_RANGE: std::ops::RangeInclusive<f32> = -10.0..=70.0;
pub const ZOOM_RANGE: std::ops::RangeInclusive<f32> = 0.2..=4.0;

impl Default for PuppetView {
    fn default() -> Self {
        PuppetView { show: false, yaw: 35.0, pitch: 12.0, zoom: 1.0, pose: String::new(), skin: true, texture: true, loaded: None }
    }
}

impl PuppetView {
    /// A drag on the preview: sideways turns the figure, up/down tilts.
    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw = (self.yaw + dx * 0.5 + 180.0).rem_euclid(360.0) - 180.0;
        self.pitch = (self.pitch + dy * 0.3).clamp(*PITCH_RANGE.start(), *PITCH_RANGE.end());
    }

    /// Back to the default framing; the pose and the loaded poses stay.
    pub fn reset_camera(&mut self) {
        let d = PuppetView::default();
        (self.yaw, self.pitch, self.zoom) = (d.yaw, d.pitch, d.zoom);
    }

    /// Wheel / pinch: a zoom factor.
    pub fn zoom_by(&mut self, factor: f32) {
        if factor.is_finite() && factor > 0.0 {
            self.zoom = (self.zoom * factor).clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
        }
    }

    /// Read the pose files again on next use (they were edited on disk).
    pub fn reload_poses(&mut self) {
        self.loaded = None;
    }

    /// The pose set for this body type as `family` sees it, loaded once.
    pub fn poses(&mut self, data_root: &Path, body_type: &str, family: &str) -> &Result<(PoseSet, PoseSource), String> {
        let stale = self.loaded.as_ref().is_none_or(|l| l.body_type != body_type || l.family != family);
        if stale {
            let result = load_poses(data_root, body_type).map(|(set, source)| (set.for_family(family).into_owned(), source));
            self.loaded = Some(Loaded { body_type: body_type.to_owned(), family: family.to_owned(), result });
        }
        &self.loaded.as_ref().expect("just loaded").result
    }

    /// The pose to draw: the named one if the set has it, else rest.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn current_pose(&mut self, data_root: &Path, body_type: &str, family: &str) -> Pose {
        let name = self.pose.clone();
        match self.poses(data_root, body_type, family) {
            Ok((set, _)) if !name.is_empty() => set.get(&name).cloned().unwrap_or_default(),
            _ => Pose::default(),
        }
    }
}

fn load_poses(data_root: &Path, body_type: &str) -> Result<(PoseSet, PoseSource), String> {
    if data_root.join("poses").join(format!("{body_type}.toml")).is_file() {
        return PoseSet::load(&data_root.join("poses"), body_type).map(|s| (s, PoseSource::DataRoot));
    }
    PoseSet::shipped(body_type)
        .map(|s| (s, PoseSource::Shipped))
        .ok_or_else(|| format!("no pose set for body type '{body_type}' (no {body_type}.toml in the data root's poses/)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orbit_wraps_the_yaw_and_clamps_the_pitch_and_zoom() {
        let mut v = PuppetView::default();
        v.yaw = 170.0;
        v.orbit(40.0, 0.0); // +20 degrees
        assert!((v.yaw - -170.0).abs() < 1e-3, "{}", v.yaw);
        v.orbit(0.0, 10_000.0);
        assert_eq!(v.pitch, *PITCH_RANGE.end());
        v.zoom_by(100.0);
        assert_eq!(v.zoom, *ZOOM_RANGE.end());
        v.zoom_by(f32::NAN);
        assert_eq!(v.zoom, *ZOOM_RANGE.end());
    }

    #[test]
    fn poses_come_from_the_data_root_else_the_shipped_copy() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = PuppetView::default();
        // Nothing in the data root: the shipped set, with the family folded in.
        let (set, source) = v.poses(dir.path(), "quadruped", "elephantid").as_ref().unwrap();
        assert_eq!(*source, PoseSource::Shipped);
        let shared = PoseSet::shipped("quadruped").unwrap();
        assert_ne!(set.get("sit"), shared.get("sit"), "elephantid's own sit");

        // A file in the data root wins once reloaded.
        std::fs::create_dir_all(dir.path().join("poses")).unwrap();
        std::fs::write(
            dir.path().join("poses/quadruped.toml"),
            "body_type = \"quadruped\"\n[pose.stand]\n[pose.wag]\ntail = { rot = [0.0, 30.0, 0.0] }\n",
        )
        .unwrap();
        assert_eq!(v.poses(dir.path(), "quadruped", "elephantid").as_ref().unwrap().1, PoseSource::Shipped, "cached");
        v.reload_poses();
        let (set, source) = v.poses(dir.path(), "quadruped", "elephantid").as_ref().unwrap();
        assert_eq!(*source, PoseSource::DataRoot);
        assert!(set.get("wag").is_some());

        // An unknown pose name, or none, draws the rest pose.
        v.pose = "nope".into();
        assert_eq!(v.current_pose(dir.path(), "quadruped", "canine"), Pose::default());
        v.pose = "wag".into();
        assert_ne!(v.current_pose(dir.path(), "quadruped", "canine"), Pose::default());
        assert!(v.poses(dir.path(), "tentacled", "x").is_err());
    }
}
