//! The baked puppet: everything a game client needs at run time, as TOML.
//! A client embeds (or ships) `<family>.puppet.toml`, calls
//! [`Puppet::from_baked_toml`], and never sees a silhouette or a rig.
//!
//! Every length a [`Puppet`] holds is already rounded to a ten-thousandth of
//! a foot (through f64), and that is what is written, so
//! `from_baked(to_baked(p)) == p` exactly and both draw identical lists.

use rig::View;
use serde::{Deserialize, Serialize};

use crate::puppet::{Eye, Kind, Puppet, PuppetError, Segment, Station};

pub const BAKED_VERSION: u32 = 1;

fn wide(v: f32) -> f64 {
    (v as f64 * 10_000.0).round() / 10_000.0
}
fn wide3(v: [f32; 3]) -> [f64; 3] {
    v.map(wide)
}
fn narrow3(v: [f64; 3]) -> [f32; 3] {
    v.map(|x| x as f32)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BakedSegment {
    pub name: String,
    pub bone: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    /// Index into `segments`; absent = a root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<u32>,
    pub group: u32,
    /// "trunk" | "limb" | "hand"
    pub kind: String,
    #[serde(default)]
    pub chain_index: u32,
    #[serde(default)]
    pub bare: u32,
    pub head: [f64; 3],
    pub tail: [f64; 3],
    pub axis_w: [f64; 3],
    pub axis_d: [f64; 3],
    pub hinge: [f64; 3],
    /// `[cx, cy, cz, half_w, half_d]` per station.
    pub stations: Vec<[f64; 5]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BakedEye {
    pub segment: u32,
    pub pos: [f64; 3],
    pub normal: [f64; 3],
    pub radius: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BakedPuppet {
    pub version: u32,
    pub family: String,
    pub body_type: String,
    pub view: View,
    pub height: f64,
    pub groups: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub eyes: Vec<BakedEye>,
    pub segments: Vec<BakedSegment>,
}

impl BakedPuppet {
    pub fn to_toml(&self) -> Result<String, PuppetError> {
        toml::to_string(self).map_err(|e| PuppetError::Baked(e.to_string()))
    }

    pub fn from_toml(text: &str) -> Result<BakedPuppet, PuppetError> {
        toml::from_str(text).map_err(|e| PuppetError::Baked(e.to_string()))
    }
}

impl Puppet {
    /// `<family>.puppet.toml`
    pub fn baked_file_name(family: &str) -> String {
        format!("{family}.puppet.toml")
    }

    pub fn to_baked(&self) -> BakedPuppet {
        BakedPuppet {
            version: BAKED_VERSION,
            family: self.family.clone(),
            body_type: self.body_type.clone(),
            view: self.view,
            height: wide(self.height),
            groups: self.groups.clone(),
            eyes: self
                .eyes
                .iter()
                .map(|e| BakedEye {
                    segment: e.segment as u32,
                    pos: wide3(e.pos),
                    normal: wide3(e.normal),
                    radius: wide(e.radius),
                })
                .collect(),
            segments: self
                .segments
                .iter()
                .map(|s| BakedSegment {
                    name: s.name.clone(),
                    bone: s.bone.clone(),
                    part: s.part.clone(),
                    parent: s.parent.map(|p| p as u32),
                    group: s.group as u32,
                    kind: match s.kind {
                        Kind::Trunk => "trunk",
                        Kind::Limb => "limb",
                        Kind::Hand => "hand",
                    }
                    .to_owned(),
                    chain_index: s.chain_index as u32,
                    bare: s.bare as u32,
                    head: wide3(s.head),
                    tail: wide3(s.tail),
                    axis_w: wide3(s.axis_w),
                    axis_d: wide3(s.axis_d),
                    hinge: wide3(s.hinge),
                    stations: s
                        .stations
                        .iter()
                        .map(|st| [wide(st.c[0]), wide(st.c[1]), wide(st.c[2]), wide(st.w), wide(st.d)])
                        .collect(),
                })
                .collect(),
        }
    }

    pub fn from_baked(baked: &BakedPuppet) -> Result<Puppet, PuppetError> {
        let bad = |what: String| Err(PuppetError::Baked(what));
        if baked.version > BAKED_VERSION {
            return bad(format!("version {} is newer than this code ({BAKED_VERSION})", baked.version));
        }
        let mut segments = Vec::with_capacity(baked.segments.len());
        for (i, s) in baked.segments.iter().enumerate() {
            if s.parent.is_some_and(|p| p as usize >= i) {
                return bad(format!("segment '{}': parent must come before it", s.name));
            }
            if s.group as usize >= baked.groups.len() {
                return bad(format!("segment '{}': no group {}", s.name, s.group));
            }
            let finite = [s.head, s.tail, s.axis_w, s.axis_d, s.hinge].iter().flatten().all(|v| v.is_finite())
                && s.stations.iter().flatten().all(|v| v.is_finite())
                && s.stations.iter().all(|st| st[3] > 0.0 && st[4] > 0.0);
            if !finite {
                return bad(format!("segment '{}': not finite, or an empty station", s.name));
            }
            segments.push(Segment {
                name: s.name.clone(),
                bone: s.bone.clone(),
                part: s.part.clone(),
                parent: s.parent.map(|p| p as usize),
                group: s.group as usize,
                kind: match s.kind.as_str() {
                    "trunk" => Kind::Trunk,
                    "limb" => Kind::Limb,
                    "hand" => Kind::Hand,
                    other => return bad(format!("segment '{}': unknown kind '{other}'", s.name)),
                },
                chain_index: s.chain_index as usize,
                bare: s.bare as usize,
                head: narrow3(s.head),
                tail: narrow3(s.tail),
                axis_w: narrow3(s.axis_w),
                axis_d: narrow3(s.axis_d),
                hinge: narrow3(s.hinge),
                stations: s
                    .stations
                    .iter()
                    .map(|st| Station {
                        c: [st[0] as f32, st[1] as f32, st[2] as f32],
                        w: st[3] as f32,
                        d: st[4] as f32,
                    })
                    .collect(),
            });
        }
        let mut eyes = Vec::with_capacity(baked.eyes.len());
        for e in &baked.eyes {
            if e.segment as usize >= segments.len() {
                return bad(format!("eye on segment {}, which does not exist", e.segment));
            }
            eyes.push(Eye {
                segment: e.segment as usize,
                pos: narrow3(e.pos),
                normal: narrow3(e.normal),
                radius: e.radius as f32,
            });
        }
        Ok(Puppet {
            family: baked.family.clone(),
            body_type: baked.body_type.clone(),
            view: baked.view,
            height: baked.height as f32,
            segments,
            groups: baked.groups.clone(),
            eyes,
        })
    }

    pub fn to_baked_toml(&self) -> Result<String, PuppetError> {
        self.to_baked().to_toml()
    }

    pub fn from_baked_toml(text: &str) -> Result<Puppet, PuppetError> {
        Puppet::from_baked(&BakedPuppet::from_toml(text)?)
    }
}
