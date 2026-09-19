//! Rig + silhouette -> label map, and the two files a client reads:
//! `<family>.png` (the map) and `<family>.toml` (its metadata).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rig_bake::{label, metrics, rasterize, LabelMap, Mask, Metrics, Outline, Pt, PARTS};
use serde::{Deserialize, Serialize};

use crate::model::{tidy, Rig, RigError, View};

/// A baked rig. Positions are `[x, y]` fractions of the map, y measured
/// DOWN from the top edge.
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
    /// Per `rig.bones` entry: true when the bone was baked and at least one
    /// pixel along it carries its part.
    pub bone_has_pixels: Vec<bool>,
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
    let map = label(mask, &bones);
    let grid = map.grid;

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
    let mut bone_has_pixels = vec![false; rig.bones.len()];
    for (k, &i) in indices.iter().enumerate() {
        bone_radii_ft[i] = Some(map.radii_ft[k]);
        bone_has_pixels[i] = bone_touches_its_part(&map, &bones[k]);
    }

    Baked {
        metrics: metrics(mask),
        anchors,
        eyes,
        px_per_ft: grid.px_per_ft,
        ground_frac: feet[1],
        counts,
        bone_radii_ft,
        bone_has_pixels,
        map,
    }
}

/// Walk the bone at half-pixel steps; true if any sample lands on a pixel
/// labelled with the bone's part. False means the bone lies wholly outside
/// the silhouette, or another bone of a different part sits exactly on it.
fn bone_touches_its_part(map: &LabelMap, bone: &rig_bake::Bone) -> bool {
    let grid = map.grid;
    bone.joints.windows(2).any(|seg| {
        let (a, b) = (seg[0], seg[1]);
        let steps = (((b[0] - a[0]).hypot(b[1] - a[1]) * grid.px_per_ft) * 2.0).ceil().max(1.0) as usize;
        (0..=steps).any(|k| {
            let t = k as f32 / steps as f32;
            let (gx, gy) = grid.to_grid([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
            gx >= 0.0
                && gy >= 0.0
                && (gx as usize) < grid.w
                && (gy as usize) < grid.h
                && map.labels[gy as usize * grid.w + gx as usize] == bone.part
        })
    })
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
    /// Parts with at least one pixel, in the client's DOLL_PARTS order.
    pub parts: Vec<String>,
    pub map: MapSize,
    /// `[x, y]` fractions of the map, y down from the top.
    pub anchors: BTreeMap<String, [f64; 2]>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub eyes: BTreeMap<String, [f64; 2]>,
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
            parts: baked.parts_present().into_iter().map(str::to_owned).collect(),
            map: MapSize { width: baked.map.grid.w as u32, height: baked.map.grid.h as u32 },
            anchors: pts(&baked.anchors),
            eyes: pts(&baked.eyes),
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
/// G = B = 0, A = 255 inside the silhouette and 0 outside.
pub fn encode_png(map: &LabelMap) -> Result<Vec<u8>, RigError> {
    let mut rgba = Vec::with_capacity(map.labels.len() * 4);
    for &l in &map.labels {
        rgba.extend_from_slice(&[l, 0, 0, if l == 0 { 0 } else { 255 }]);
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

/// Inverse of [`encode_png`]: (width, height, labels). Transparent pixels
/// read as 0 whatever their red channel says.
pub fn decode_png(bytes: &[u8]) -> Result<(usize, usize, Vec<u8>), RigError> {
    let err = |e: png::DecodingError| RigError::Parse(e.to_string());
    let mut reader = png::Decoder::new(bytes).read_info().map_err(err)?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(err)?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return Err(RigError::Parse(format!("expected 8-bit RGBA, got {:?}", info.color_type)));
    }
    let labels = buf[..info.buffer_size()]
        .chunks_exact(4)
        .map(|px| if px[3] == 0 { 0 } else { px[0] })
        .collect();
    Ok((info.width as usize, info.height as usize, labels))
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
