//! A puppet as a client holds it: a tree of segments, each a run of
//! elliptical cross-sections ("stations") in rest space, its eyes, and what
//! it needs to be posed. Read from its baked file (`baked`); made by the
//! builder (`build`: a rig + two silhouettes), which is authoring and leaves
//! this crate (docs/crate-split-plan.md).

use std::fmt;

use rig::View;

use crate::math::{M3, V3};

/// Stations are sampled about this far apart along a segment (the builder
/// measures them so; a wound's place and a skin's layout count in them).
pub const STATION_STEP_FT: f32 = 0.08;
/// Rings an end cap adds to a segment's measured stations.
pub const CAP_RINGS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Head, neck, chest, abdomen, back: depth is read from the other view.
    Trunk,
    /// Arm / leg parts, and bones with no part: round where the other view
    /// cannot tell.
    Limb,
    /// A hand held off the ground: a mitten, wide and flat.
    Hand,
    /// Something held, not body ([`crate::prop`]): drawn in its own colour,
    /// never textured, never baked, and left out of grounding.
    Prop { color: [u8; 3] },
    /// Clothing ([`crate::outfit`]): painted and outlined like the body, but
    /// never baked and left out of grounding; `hang` says how it follows a
    /// pose.
    Garment { hang: crate::outfit::Hang },
}

impl Kind {
    pub fn is_prop(self) -> bool {
        matches!(self, Kind::Prop { .. })
    }

    pub fn is_garment(self) -> bool {
        matches!(self, Kind::Garment { .. })
    }

    /// Flesh: what the rig measured, what stands on the floor.
    pub fn is_body(self) -> bool {
        matches!(self, Kind::Trunk | Kind::Limb | Kind::Hand)
    }
}

pub fn is_limb_part(part: Option<&str>) -> bool {
    part.is_none_or(|p| p.ends_with("Arm") || p.ends_with("Leg") || p.ends_with("Hand"))
}

/// One elliptical cross-section, in REST space. The ellipse lies in the
/// plane of its segment's `axis_w` and `axis_d`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Station {
    pub c: V3,
    /// Half extent along `axis_w` / `axis_d`, feet.
    pub w: f32,
    pub d: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// The bone's name, or `bone.N` when the bone has several segments.
    pub name: String,
    pub bone: String,
    pub part: Option<String>,
    /// Index of the parent segment (always smaller than this one's).
    pub parent: Option<usize>,
    /// Index into [`Puppet::groups`]: segments outlined as one shape.
    pub group: usize,
    pub kind: Kind,
    /// 0 = the first segment of its chain (it takes the bone's rotation);
    /// k > 0 takes the pose's `bend[k - 1]` about `hinge`.
    pub chain_index: usize,
    /// Rest pivot and rest far joint.
    pub head: V3,
    pub tail: V3,
    /// Unit axes of every station's ellipse: `axis_w` lies in the view the
    /// segment was measured in, across the bone; `axis_d` is that view's
    /// off-plane world axis.
    pub axis_w: V3,
    pub axis_d: V3,
    /// Natural hinge axis (unit, rest space) for `bend`.
    pub hinge: V3,
    /// Head cap rings (if any), the measured stations head -> tail, then the
    /// tail cap rings.
    pub stations: Vec<Station>,
    /// Leading stations that sit INSIDE the trunk at rest (a thigh in the
    /// pelvis, an upper arm in the shoulder): they are filled but not
    /// outlined, so a limb grows out of the body instead of being pinned on.
    pub bare: usize,
    /// Covered by a garment ([`crate::outfit`]): not drawn, outlined or
    /// given depth, but still posed and still standing on the floor.
    pub hidden: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Eye {
    pub segment: usize,
    /// Rest position on the head surface, and the outward normal there.
    pub pos: V3,
    pub normal: V3,
    pub radius: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Puppet {
    pub family: String,
    pub body_type: String,
    pub view: View,
    /// Built from a silhouette drawn from above (`Rig::from_above`): the
    /// figure lies flat, its front toward +z, as any other puppet does,
    /// but its own silhouette is its plan, not its elevation.
    pub from_above: bool,
    /// Drawn from above: how far its plan's frame (the puppet's z) runs
    /// ahead of its side silhouette's (side x = z - side_shift), as the two
    /// were registered for the build ([`plan_shift`]). 0 for every other
    /// puppet.
    pub side_shift: f32,
    /// Drawn with the arms raised (`Rig::arms_raised`): they hang for
    /// posing ([`Puppet::arm_hang`]).
    pub arms_raised: bool,
    /// Top of the rest figure, feet.
    pub height: f32,
    pub segments: Vec<Segment>,
    pub groups: Vec<String>,
    pub eyes: Vec<Eye>,
    /// Arms drawn raised (an A-pose sheet), turned down to hang before a
    /// pose applies: (the arm's first segment, the turn about its head).
    /// Worked out from the segments ([`Puppet::arm_hangs`], `hang.rs`);
    /// empty when every arm is drawn hanging.
    pub arm_hang: Vec<(usize, M3)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PuppetError {
    /// `outlines.json` has no silhouette for the rig's own view.
    NoOutline(String),
    /// No bone has two placed joints.
    NoSegments,
    Baked(String),
}

impl fmt::Display for PuppetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PuppetError::NoOutline(key) => write!(f, "no silhouette '{key}'"),
            PuppetError::NoSegments => write!(f, "no bone has two placed joints"),
            PuppetError::Baked(e) => write!(f, "baked puppet: {e}"),
        }
    }
}

impl std::error::Error for PuppetError {}

impl Puppet {
    pub fn segment(&self, name: &str) -> Option<usize> {
        self.segments.iter().position(|s| s.name == name)
    }

    /// Every segment below `root` in the tree, `root` included.
    pub fn subtree(&self, root: usize) -> Vec<usize> {
        let mut out = vec![root];
        for i in root + 1..self.segments.len() {
            if self.segments[i].parent.is_some_and(|p| out.contains(&p)) {
                out.push(i);
            }
        }
        out
    }

    /// Distinct bone names, in tree order (what a pose addresses).
    pub fn bones(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for s in &self.segments {
            if !out.contains(&s.bone.as_str()) {
                out.push(&s.bone);
            }
        }
        out
    }

    pub fn station_count(&self) -> usize {
        self.segments.iter().map(|s| s.stations.len()).sum()
    }
}
