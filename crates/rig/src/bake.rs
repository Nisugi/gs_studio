//! Rig + silhouette -> label map, and the two files a client reads:
//! `<family>.png` (the map) and `<family>.toml` (its metadata).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rig_bake::{label, metrics, part_index, rasterize, LabelMap, Mask, Metrics, Outline, Pt, PARTS};
use serde::{Deserialize, Serialize};

use crate::model::{tidy, Rig, RigError, View};

/// Auto eye radius: this share of the head bone's measured radius...
pub const EYE_RADIUS_SHARE: f32 = 0.18;
/// ...but never less than this many pixels on a 512 px map (scaled with the
/// map, so the floor is the same size in feet at every resolution).
pub const EYE_MIN_PX_AT_512: f32 = 2.0;

/// One `rig.bones` entry as posing will want it. Positions are map
/// fractions (y down), like the anchors.
#[derive(Debug, Clone, PartialEq)]
pub struct BakedBone {
    pub name: String,
    pub part: Option<String>,
    pub parent: Option<String>,
    /// Where the bone's piece rotates; None while the bone has no placed joint.
    pub pivot: Option<[f32; 2]>,
    /// The chain end away from the pivot.
    pub tip: Option<[f32; 2]>,
    /// Every placed joint of the chain, in order (a later step can split a
    /// multi-joint chain into segments that bend).
    pub joints: Vec<[f32; 2]>,
    /// Pixels whose bone channel names this bone.
    pub pixels: usize,
}

/// A baked rig. Positions are `[x, y]` fractions of the map, y measured
/// DOWN from the top edge.
///
/// `map.labels` has the eye discs painted in; `map.bones` holds 1-based
/// indices into the RIG's bone list (not into the filtered list `rig_bake`
/// saw), so a skipped bone never shifts anyone's index.
#[derive(Debug, Clone)]
pub struct Baked {
    pub map: LabelMap,
    pub metrics: Metrics,
    /// head / mouth / saddle (when their joint is placed) and feet (always).
    pub anchors: BTreeMap<String, [f32; 2]>,
    /// leftEye / rightEye, when placed.
    pub eyes: BTreeMap<String, [f32; 2]>,
    pub px_per_ft: f32,
    /// Where the ground line (y = 0 ft) sits, as a fraction down the map.
    pub ground_frac: f32,
    /// Pixel count per label value; `[0]` is outside.
    pub counts: [usize; PARTS.len() + 1],
    /// Per `rig.bones` entry: the radius `rig_bake` used, in feet. None for
    /// bones that were skipped (no part, or fewer than two placed joints).
    pub bone_radii_ft: Vec<Option<f32>>,
    /// Per `rig.bones` entry: how many pixels the bone owns (0 when skipped).
    pub bone_pixels: Vec<usize>,
    /// Per `rig.bones` entry, for the baked `[[bones]]` table.
    pub bones: Vec<BakedBone>,
    /// Eye disc radius used, in feet; None when there is no eye to paint or
    /// (on Auto) no head bone to measure.
    pub eye_radius_ft: Option<f32>,
    /// leftEye / rightEye -> head pixels the disc relabelled. An eye that is
    /// placed but painted nothing is here with 0.
    pub eye_pixels: BTreeMap<String, usize>,
}

impl Baked {
    /// Filled (inside-the-silhouette) pixel count.
    pub fn filled(&self) -> usize {
        self.counts[1..].iter().sum()
    }

    /// Share of the filled pixels carrying `part`, 0..=1.
    pub fn share(&self, part: &str) -> f32 {
        let filled = self.filled();
        match rig_bake::part_index(part) {
            Some(i) if filled > 0 => self.counts[i as usize] as f32 / filled as f32,
            _ => 0.0,
        }
    }

    /// Parts with at least one pixel, in `PARTS` order.
    pub fn parts_present(&self) -> Vec<&'static str> {
        PARTS.iter().enumerate().filter(|(i, _)| self.counts[i + 1] > 0).map(|(_, p)| *p).collect()
    }
}

/// Rasterize the outline and bake. `long_side_px` is the map's longer side.
pub fn bake(rig: &Rig, outline: &Outline, long_side_px: usize) -> Baked {
    bake_mask(rig, &rasterize(outline, long_side_px))
}

/// Bake against an already-rasterized silhouette (an editor re-bakes on
/// every change and the mask only depends on the outline and size).
pub fn bake_mask(rig: &Rig, mask: &Mask) -> Baked {
    let (indices, bones): (Vec<usize>, Vec<rig_bake::Bone>) = rig.bake_bones().into_iter().unzip();
    let mut map = label(mask, &bones);
    let grid = map.grid;

    // Bone channel: filtered-slice index -> index into `rig.bones`. A rig
    // bone past 255 cannot be named by a byte and owns nothing.
    let mut remap = vec![0u8; bones.len() + 1];
    for (k, &i) in indices.iter().enumerate() {
        remap[k + 1] = u8::try_from(i + 1).unwrap_or(0);
    }
    if indices.iter().enumerate().any(|(k, &i)| i != k) {
        for o in &mut map.bones {
            *o = remap[*o as usize];
        }
    }

    let (eye_radius_ft, eye_pixels) = paint_eyes(rig, &mut map, &indices);

    let mut counts = [0usize; PARTS.len() + 1];
    for &l in &map.labels {
        counts[l as usize] += 1;
    }

    let frac = |p: Pt| -> [f32; 2] {
        let (gx, gy) = grid.to_grid(p);
        [gx / grid.w.max(1) as f32, gy / grid.h.max(1) as f32]
    };
    let mut anchors = BTreeMap::new();
    for (key, joint) in
        [("head", &rig.anchors.head), ("mouth", &rig.anchors.mouth), ("saddle", &rig.anchors.saddle)]
    {
        if let Some(p) = rig.joints.get(joint) {
            anchors.insert(key.to_owned(), frac(*p));
        }
    }
    let feet = frac([0.0, 0.0]);
    anchors.insert("feet".to_owned(), feet);
    let mut eyes = BTreeMap::new();
    let (left, right) = rig.eye_points();
    if let Some(p) = left {
        eyes.insert("leftEye".to_owned(), frac(p));
    }
    if let Some(p) = right {
        eyes.insert("rightEye".to_owned(), frac(p));
    }

    let mut bone_radii_ft = vec![None; rig.bones.len()];
    let mut bone_pixels = vec![0usize; rig.bones.len()];
    for (k, &i) in indices.iter().enumerate() {
        bone_radii_ft[i] = Some(map.radii_ft[k]);
        bone_pixels[i] = map.bone_pixels[k];
    }
    let baked_bones = rig
        .bones
        .iter()
        .zip(&bone_pixels)
        .map(|(b, &pixels)| BakedBone {
            name: b.name.clone(),
            part: b.part.clone(),
            parent: b.parent.clone(),
            pivot: rig.bone_pivot(b).map(frac),
            tip: rig.bone_tip(b).map(frac),
            joints: rig.bone_points(b).into_iter().map(frac).collect(),
            pixels,
        })
        .collect();

    Baked {
        metrics: metrics(mask),
        anchors,
        eyes,
        px_per_ft: grid.px_per_ft,
        ground_frac: feet[1],
        counts,
        bone_radii_ft,
        bone_pixels,
        bones: baked_bones,
        eye_radius_ft,
        eye_pixels,
        map,
    }
}

/// Eyes are regions, not just points: a small disc per placed eye, labelled
/// leftEye / rightEye, painted over the finished label map -- but ONLY onto
/// pixels that are `head`, and only in the part channel (the bone channel
/// keeps the head bone, so the eye moves with the skull). Returns the radius
/// used and the pixels each eye relabelled.
fn paint_eyes(rig: &Rig, map: &mut LabelMap, indices: &[usize]) -> (Option<f32>, BTreeMap<String, usize>) {
    let mut painted = BTreeMap::new();
    let (left, right) = rig.eye_points();
    if left.is_none() && right.is_none() {
        return (None, painted);
    }
    let head = part_index("head").unwrap_or(0);
    let grid = map.grid;
    let radius_ft = rig.eye_radius_ft.map(|r| r.max(0.0)).or_else(|| {
        let k = indices.iter().position(|&i| rig.bones[i].part.as_deref() == Some("head"))?;
        let floor_px = EYE_MIN_PX_AT_512 * grid.w.max(grid.h) as f32 / 512.0;
        Some((map.radii_ft[k] * EYE_RADIUS_SHARE).max(floor_px / grid.px_per_ft))
    });
    let Some(radius_ft) = radius_ft else {
        return (None, painted);
    };
    let r_px = radius_ft * grid.px_per_ft;
    for (key, point) in [("leftEye", left), ("rightEye", right)] {
        let (Some(p), Some(eye)) = (point, part_index(key)) else {
            continue;
        };
        let (cx, cy) = grid.to_grid(p);
        let mut n = 0;
        let (x0, x1) = ((cx - r_px).floor().max(0.0) as usize, (cx + r_px).ceil().max(0.0) as usize);
        let (y0, y1) = ((cy - r_px).floor().max(0.0) as usize, (cy + r_px).ceil().max(0.0) as usize);
        for y in y0..=y1.min(grid.h.saturating_sub(1)) {
            for x in x0..=x1.min(grid.w.saturating_sub(1)) {
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                let i = y * grid.w + x;
                if dx * dx + dy * dy <= r_px * r_px && map.labels.get(i) == Some(&head) {
                    map.labels[i] = eye;
                    n += 1;
                }
            }
        }
        painted.insert(key.to_owned(), n);
    }
    (Some(radius_ft), painted)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapSize {
    pub width: u32,
    pub height: u32,
}

/// `<family>.toml`, beside `<family>.png`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BakedMeta {
    pub family: String,
    pub view: View,
    pub ref_ft: f64,
    pub axis: String,
    pub px_per_ft: f64,
    /// Content width / height.
    pub aspect: f64,
    /// Ground-contact width as a fraction of content width.
    pub span: f64,
    /// Ground line as a fraction down the map.
    pub ground_frac: f64,
    /// Radius of the painted eye discs, in feet; absent when none were.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eye_radius_ft: Option<f64>,
    /// Parts with at least one pixel, in the client's DOLL_PARTS order.
    pub parts: Vec<String>,
    pub map: MapSize,
    /// `[x, y]` fractions of the map, y down from the top.
    pub anchors: BTreeMap<String, [f64; 2]>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub eyes: BTreeMap<String, [f64; 2]>,
    /// One per bone of the rig, IN RIG ORDER: the PNG's G channel is the
    /// 1-based index into this list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bones: Vec<BoneMeta>,
}

/// One `[[bones]]` entry of `<family>.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoneMeta {
    /// The value this bone's pixels carry in the PNG's G channel.
    pub index: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    /// Parent bone's name; absent = a root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Where the piece rotates, `[x, y]` map fractions, y down. Absent for a
    /// bone with no placed joint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pivot: Option<[f64; 2]>,
    /// The chain end away from the pivot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tip: Option<[f64; 2]>,
    /// Every placed joint of the chain, in order.
    #[serde(default)]
    pub joints: Vec<[f64; 2]>,
    /// Pixels in the map owned by this bone (0 = it was skipped or lost).
    pub pixels: u64,
}

impl BakedMeta {
    pub fn new(rig: &Rig, baked: &Baked) -> BakedMeta {
        let pts = |m: &BTreeMap<String, [f32; 2]>| {
            m.iter().map(|(k, p)| (k.clone(), [tidy(p[0]), tidy(p[1])])).collect()
        };
        BakedMeta {
            family: rig.family.clone(),
            view: rig.view,
            ref_ft: tidy(rig.ref_ft),
            axis: rig.axis.clone(),
            px_per_ft: tidy(baked.px_per_ft),
            aspect: tidy(baked.metrics.aspect),
            span: tidy(baked.metrics.span),
            ground_frac: tidy(baked.ground_frac),
            eye_radius_ft: baked.eye_radius_ft.map(tidy),
            parts: baked.parts_present().into_iter().map(str::to_owned).collect(),
            map: MapSize { width: baked.map.grid.w as u32, height: baked.map.grid.h as u32 },
            anchors: pts(&baked.anchors),
            eyes: pts(&baked.eyes),
            bones: baked
                .bones
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    let pt = |p: [f32; 2]| [tidy(p[0]), tidy(p[1])];
                    BoneMeta {
                        index: i as u32 + 1,
                        name: b.name.clone(),
                        part: b.part.clone(),
                        parent: b.parent.clone(),
                        pivot: b.pivot.map(pt),
                        tip: b.tip.map(pt),
                        joints: b.joints.iter().copied().map(pt).collect(),
                        pixels: b.pixels as u64,
                    }
                })
                .collect(),
        }
    }

    pub fn to_toml(&self) -> Result<String, RigError> {
        toml::to_string(self).map_err(|e| RigError::Encode(e.to_string()))
    }

    pub fn from_toml(text: &str) -> Result<BakedMeta, RigError> {
        toml::from_str(text).map_err(|e| RigError::Parse(e.to_string()))
    }
}

/// The label map as an 8-bit RGBA PNG: R = label (1-based `PARTS` index),
/// G = owning bone (1-based index into the rig's -- and the baked TOML's --
/// bone list; 0 = none), B = 0, A = 255 inside the silhouette and 0 outside.
pub fn encode_png(map: &LabelMap) -> Result<Vec<u8>, RigError> {
    let mut rgba = Vec::with_capacity(map.labels.len() * 4);
    for (i, &l) in map.labels.iter().enumerate() {
        let bone = map.bones.get(i).copied().unwrap_or(0);
        rgba.extend_from_slice(&[l, if l == 0 { 0 } else { bone }, 0, if l == 0 { 0 } else { 255 }]);
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, map.grid.w as u32, map.grid.h as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| RigError::Encode(e.to_string()))?;
    writer.write_image_data(&rgba).map_err(|e| RigError::Encode(e.to_string()))?;
    writer.finish().map_err(|e| RigError::Encode(e.to_string()))?;
    Ok(out)
}

/// A decoded `<family>.png`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedMap {
    pub width: usize,
    pub height: usize,
    /// R: 0 outside, else the 1-based part.
    pub labels: Vec<u8>,
    /// G: 0 outside (and everywhere in a map baked before bones were
    /// recorded), else the 1-based bone.
    pub bones: Vec<u8>,
}

/// Inverse of [`encode_png`]. Transparent pixels read as 0 in both channels
/// whatever the file says.
pub fn decode_png(bytes: &[u8]) -> Result<DecodedMap, RigError> {
    let err = |e: png::DecodingError| RigError::Parse(e.to_string());
    let mut reader = png::Decoder::new(bytes).read_info().map_err(err)?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(err)?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return Err(RigError::Parse(format!("expected 8-bit RGBA, got {:?}", info.color_type)));
    }
    let pixels = buf[..info.buffer_size()].chunks_exact(4);
    let labels = pixels.clone().map(|px| if px[3] == 0 { 0 } else { px[0] }).collect();
    let bones = pixels.map(|px| if px[3] == 0 { 0 } else { px[1] }).collect();
    Ok(DecodedMap { width: info.width as usize, height: info.height as usize, labels, bones })
}

/// Write `<family>.png` and `<family>.toml` into `dir`; returns both paths.
pub fn write_baked(rig: &Rig, baked: &Baked, dir: &Path) -> Result<(PathBuf, PathBuf), RigError> {
    std::fs::create_dir_all(dir).map_err(|e| RigError::Io(dir.to_owned(), e))?;
    let png_path = dir.join(format!("{}.png", rig.family));
    let meta_path = dir.join(format!("{}.toml", rig.family));
    std::fs::write(&png_path, encode_png(&baked.map)?).map_err(|e| RigError::Io(png_path.clone(), e))?;
    let meta = BakedMeta::new(rig, baked).to_toml()?;
    std::fs::write(&meta_path, meta).map_err(|e| RigError::Io(meta_path.clone(), e))?;
    Ok((png_path, meta_path))
}
