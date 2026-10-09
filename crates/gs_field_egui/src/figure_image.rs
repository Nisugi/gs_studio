//! A figure or a backdrop as the GPU pass's layer image (ADDITION; out of
//! `field_gpu_stage`, which uses them too): a puppet's CPU skin turned into
//! a lit image with its normals and depth ([`skin_image`]), the flat ground
//! a backdrop stands on ([`ground_geo`]), and a lit image's glow in its
//! alpha ([`with_glow`]). What the doll and the stage both draw through
//! (docs/crate-split-plan.md: the base render).

use std::sync::Arc;

use crate::field_gpu::{FieldCamera, LitImage};

/// Flat ground below the horizon, sky above, for a backdrop image laid over
/// `rect` (scene pixels).
pub fn ground_geo(camera: &FieldCamera, w: usize, h: usize, rect: [f32; 4]) -> Vec<[f32; 4]> {
    let up = camera.up();
    let mut geo = Vec::with_capacity(w * h);
    for j in 0..h {
        let sy = rect[1] + (j as f32 + 0.5) * rect[3] / h as f32;
        let z = camera.ground_depth(sy);
        let g = if z.is_finite() && z < 100.0 { [up[0], up[1], up[2], z] } else { [0.0, 0.0, -1.0, 100.0] };
        geo.extend(std::iter::repeat_n(g, w));
    }
    geo
}

/// `lit` with its alpha replaced by the glow (the pass reads the lit
/// image's alpha as "never lit, never darkened").
pub fn with_glow(lit: &[u8], glow: Option<&[u8]>) -> Vec<u8> {
    let mut out = lit.to_vec();
    for (i, p) in out.chunks_exact_mut(4).enumerate() {
        p[3] = glow.map_or(0, |g| g[i]);
    }
    out
}

/// How far round the figure an outline pixel looks for the body's depth,
/// pixels: past the widest outline a skin draws (the contour and a boss's
/// gold round it).
const OUTLINE_REACH: usize = 8;

/// A puppet's skin as the pass's layer image. Every pixel the skin shows is
/// in it, at the skin's own coverage: where the figure's surface is
/// (`surface.texel`), its own colour, normal and depth, lit by the lamps;
/// the contour drawn round it and a boss's gold outline -- skin with no
/// surface -- keep their colour and are left unlit (no albedo for a lamp
/// to light), at the depth of the nearest surface pixel, so the shadow
/// they cast is the body's edge.
pub fn skin_image(key: u64, skin: &gs_puppet::Skin, surface: &gs_puppet::SkinSurface, m_per_ft: f32) -> LitImage {
    // the skin's frame (x right, y up, +z toward the viewer) to the
    // camera's (x right, y down, z away)
    let back = surface.to_cam();
    let fwd = [-back[0], -back[1], -back[2]];
    let up = [0.0f32, 1.0, 0.0];
    let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let unit = |v: [f32; 3]| {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
        v.map(|c| c / l)
    };
    let right = unit(cross(fwd, up));
    let down = cross(fwd, right);
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let (w, h) = (skin.w, skin.h);
    let n = w * h;
    let mut albedo = vec![0u8; n * 4];
    let mut geo = vec![[0.0f32; 4]; n];
    let mut surfaced = vec![false; n];
    for i in 0..n {
        let shown = skin.rgba[i * 4 + 3];
        if let Some((pos, nrm, alb, _)) = surface.texel(i) {
            albedo[i * 4..i * 4 + 3].copy_from_slice(&alb);
            // wet blood rides in the normal's length (field_gpu::wet_normal)
            let wet = surface.wet().get(i).map_or(0.0, |w| *w as f32 / 255.0);
            let [nx, ny, nz] = crate::field_gpu::wet_normal([dot(nrm, right), dot(nrm, down), dot(nrm, fwd)], wet);
            geo[i] = [nx, ny, nz, dot(pos, fwd) * m_per_ft];
            surfaced[i] = true;
        }
        albedo[i * 4 + 3] = shown;
    }
    // an outline pixel: the depth of the nearest surface pixel, ring by ring
    for i in 0..n {
        if surfaced[i] || skin.rgba[i * 4 + 3] == 0 {
            continue;
        }
        let (x, y) = ((i % w) as isize, (i / w) as isize);
        'rings: for r in 1..=OUTLINE_REACH as isize {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dy.abs() != r {
                        continue;
                    }
                    let (qx, qy) = (x + dx, y + dy);
                    if qx < 0 || qy < 0 || qx >= w as isize || qy >= h as isize {
                        continue;
                    }
                    let j = qy as usize * w + qx as usize;
                    if surfaced[j] {
                        geo[i][3] = geo[j][3];
                        break 'rings;
                    }
                }
            }
        }
    }
    // what glows (nerve marks) keeps showing where no light reaches
    let glow = surface.glow();
    let glow = (glow.len() == n && glow.iter().any(|g| *g > 0)).then_some(glow);
    LitImage { key, w: w as u32, h: h as u32, albedo: Arc::new(albedo), lit: Arc::new(with_glow(&skin.rgba, glow)), geo: Arc::new(geo) }
}
