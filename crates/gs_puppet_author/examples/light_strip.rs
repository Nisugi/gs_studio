//! light_strip: PROTOTYPE of lighting a painted puppet from the scene
//! instead of the skin's own fixed cel light (Niffy, 2026-09-26: project
//! the texture flat, let a normal map and the lighting engine do the
//! shading).
//!
//!   cargo run --release -p gs_puppet --example light_strip -- <out.png> [family] [creature] [pose] [yaw]
//!
//! Columns are four scene lights: day (sun high, front left), a raking sun
//! from the right, dusk (low sun behind), night (moon and a torch). Rows:
//!
//! 0. today: the cel-shaded skin (a light fixed to the view), with the
//!    folder's normal maps if it has any; the night column adds the torch;
//! 1. flat paint, lit by the scene ([`SkinSurface::scene_lit`]), the
//!    geometry's own normals;
//! 2. flat paint + a bump map made from the paint alone (the high-passed
//!    brightness of each painting, its slopes the normals);
//! 3. flat paint + the folder's own normal maps (Depth Anything + detail),
//!    when it has them: their big forms on top of the hull's own;
//! 4. "de-lit" paint (the paintings' large-scale light and shade divided
//!    out) + the bump;
//! 5. the light alone on a grey body, with the bump.
//!
//! `LIGHT_STRIP_TILE` sets the tile size (default 300), `LIGHT_STRIP_BUMP`
//! the bump's strength (default 4). The scene-lit rows have no contour
//! lines unless `LIGHT_STRIP_OUTLINE=1`.

use gs_puppet_author::prelude::*;
use std::path::{Path, PathBuf};

use gs_puppet::{Camera, Effects, Lamp, Look, PoseSet, Puppet, SceneLight, Skin, Texture, UvLayout, SHEET_SIDE};
use gs_puppet_author::{prefill, prefill_normals, Painting, Paintings, MASK_PX};

const BACKGROUND: [u8; 3] = [0x2b, 0x30, 0x36];

fn read_png(path: &Path) -> Option<(usize, usize, Vec<u8>)> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = png::Decoder::new(std::io::BufReader::new(file)).read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..w * h * 4].to_vec(),
        png::ColorType::Rgb => buf[..w * h * 3].chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        _ => return None,
    };
    Some((w, h, rgba))
}

type Img = (usize, usize, Vec<u8>);

fn as_painting(p: &Option<Img>) -> Option<Painting<'_>> {
    p.as_ref().map(|(w, h, rgba)| Painting { rgba, w: *w, h: *h })
}

/// A box blur of radius `r`, three passes (near a Gaussian), of `v`
/// weighted by `a` and divided by the blurred weight: the background never
/// bleeds in.
fn blur_masked(v: &[f32], a: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    fn pass(src: &[f32], w: usize, h: usize, r: usize, horizontal: bool) -> Vec<f32> {
        let mut out = vec![0.0; src.len()];
        let (n, m) = if horizontal { (h, w) } else { (w, h) };
        let at = |line: usize, k: usize| if horizontal { line * w + k } else { k * w + line };
        for line in 0..n {
            let mut acc = 0.0;
            let mut count = 0.0;
            for k in 0..=r.min(m - 1) {
                acc += src[at(line, k)];
                count += 1.0;
            }
            for k in 0..m {
                out[at(line, k)] = acc / count;
                if k + r + 1 < m {
                    acc += src[at(line, k + r + 1)];
                    count += 1.0;
                }
                if k >= r {
                    acc -= src[at(line, k - r)];
                    count -= 1.0;
                }
            }
        }
        out
    }
    let mut num: Vec<f32> = v.iter().zip(a).map(|(v, a)| v * a).collect();
    let mut den = a.to_vec();
    for _ in 0..3 {
        num = pass(&pass(&num, w, h, r, true), w, h, r, false);
        den = pass(&pass(&den, w, h, r, true), w, h, r, false);
    }
    num.iter().zip(&den).map(|(n, d)| if *d > 1e-4 { n / d } else { 0.0 }).collect()
}

fn luminance(rgba: &[u8]) -> (Vec<f32>, Vec<f32>) {
    let l = rgba.chunks(4).map(|c| (0.2126 * c[0] as f32 + 0.7152 * c[1] as f32 + 0.0722 * c[2] as f32) / 255.0).collect();
    let a = rgba.chunks(4).map(|c| if c[3] > 127 { 1.0 } else { 0.0 }).collect();
    (l, a)
}

/// A tangent-space normal map (RGB = xyz * 0.5 + 0.5; x right, y up, z to
/// the viewer) from a painting's brightness alone: its detail (the
/// brightness minus a blur of it) taken as height, the height's slopes as
/// the normals. Fine relief only -- the big forms are the hull's.
fn bump_from_paint((w, h, rgba): &Img, strength: f32) -> Img {
    let (w, h) = (*w, *h);
    let (l, a) = luminance(rgba);
    let r = (h / 170).max(2);
    let base = blur_masked(&l, &a, w, h, r);
    let height: Vec<f32> = l.iter().zip(&base).zip(&a).map(|((l, b), a)| (l - b) * a).collect();
    let mut out = vec![0u8; w * h * 4];
    // slopes per pixel scaled to the painting's size, so strength means the
    // same at any resolution
    let k = strength * h as f32 / 1024.0;
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if a[i] == 0.0 {
                continue;
            }
            let hx = |xx: usize| height[y * w + xx.min(w - 1)];
            let hy = |yy: usize| height[yy.min(h - 1) * w + x];
            let dx = (hx(x + 1) - hx(x.saturating_sub(1))) * 0.5;
            // image rows run down; the map's y is up
            let dy = (hy(y.saturating_sub(1)) - hy(y + 1)) * 0.5;
            let n = [-dx * k, -dy * k, 1.0];
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            for c in 0..3 {
                out[i * 4 + c] = ((n[c] / len * 0.5 + 0.5) * 255.0).round() as u8;
            }
            out[i * 4 + 3] = 255;
        }
    }
    (w, h, out)
}

/// The painting with its large-scale light and shade divided out: every
/// pixel scaled by the painting's mean brightness over a wide blur of the
/// brightness round it (a crude albedo; local detail and colour stay).
fn delight((w, h, rgba): &Img) -> Img {
    let (w, h) = (*w, *h);
    let (l, a) = luminance(rgba);
    let wide = blur_masked(&l, &a, w, h, (h / 14).max(4));
    let (sum, n) = l.iter().zip(&a).fold((0.0, 0.0), |(s, n), (l, a)| (s + l * a, n + a));
    let mean = sum / n.max(1.0);
    let mut out = rgba.clone();
    for i in 0..w * h {
        if a[i] == 0.0 {
            continue;
        }
        let s = (mean / wide[i].max(0.02)).clamp(0.5, 2.0);
        for c in 0..3 {
            out[i * 4 + c] = (rgba[i * 4 + c] as f32 * s).round().clamp(0.0, 255.0) as u8;
        }
    }
    (w, h, out)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = PathBuf::from(args.first().expect("usage: light_strip <out.png> [family] [creature] [pose] [yaw]"));
    let family = args.get(1).map_or("humanoid", String::as_str);
    let creature = args.get(2).filter(|c| !c.is_empty() && *c != "-");
    let pose_name = args.get(3).map_or("stand", String::as_str);
    let yaw: f32 = args.get(4).and_then(|v| v.parse().ok()).unwrap_or(25.0);
    let tile: usize = std::env::var("LIGHT_STRIP_TILE").ok().and_then(|v| v.parse().ok()).unwrap_or(300);
    let bump: f32 = std::env::var("LIGHT_STRIP_BUMP").ok().and_then(|v| v.parse().ok()).unwrap_or(4.0);

    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let rig = rig::Rig::load(&data.join("rigs").join(rig::Rig::file_name(family))).expect("rig");
    let outlines = rig::Outlines::load(&data.join("outlines.json")).expect("outlines");
    let puppet = Puppet::build(&rig, &outlines).expect("puppet builds");
    let set = PoseSet::load(&data.join("poses"), &rig.body_type).expect("poses").for_family(family).into_owned();
    let posed = puppet.pose(set.get(pose_name).expect("pose"));

    let mut folder = data.join("textures").join(family);
    if let Some(c) = creature {
        folder = folder.join(c);
    }
    let grid = |view: rig::View| outlines.get(family, view).map(|e| rig_bake::rasterize(&e.outline(), MASK_PX).grid);
    let (front_grid, side_grid) = (grid(rig::View::Front), grid(rig::View::Side));
    let load = |suffix: &str| ["front", "side", "back"].map(|v| read_png(&folder.join(format!("{v}{suffix}.png"))));
    let paint = load("");
    assert!(paint.iter().any(Option::is_some), "no paintings in {}", folder.display());
    let maps = load("_normal");
    let bumps = paint.clone().map(|p| p.map(|p| bump_from_paint(&p, bump)));
    let delit = paint.clone().map(|p| p.map(|p| delight(&p)));
    let set_of = |imgs: &[Option<Img>; 3]| {
        // (Paintings borrows; build it where it is used)
        imgs.clone()
    };
    let paintings = |imgs: &[Option<Img>; 3]| -> Vec<u8> {
        let p = Paintings { front_grid, side_grid, front: as_painting(&imgs[0]), side: as_painting(&imgs[1]), back: as_painting(&imgs[2]) };
        prefill(&puppet, &UvLayout::fit(&puppet, SHEET_SIDE), &p)
    };
    let normals = |imgs: &[Option<Img>; 3]| -> Option<Vec<u8>> {
        imgs.iter().any(Option::is_some).then(|| {
            let p = Paintings { front_grid, side_grid, front: as_painting(&imgs[0]), side: as_painting(&imgs[1]), back: as_painting(&imgs[2]) };
            prefill_normals(&puppet, &UvLayout::fit(&puppet, SHEET_SIDE), &p)
        })
    };
    let layout = UvLayout::fit(&puppet, SHEET_SIDE);
    let paint_sheet = paintings(&set_of(&paint));
    let delit_sheet = paintings(&delit);
    let bump_sheet = normals(&bumps);
    let map_sheet = normals(&maps);
    let grey: Vec<u8> = paint_sheet.chunks(4).flat_map(|c| [178, 170, 160, c[3]]).collect();
    let tex = |rgba: &Vec<u8>, normal: &Option<Vec<u8>>| Texture { layout: layout.clone(), rgba: rgba.clone(), normal: normal.clone() };

    let lights = [
        ("day: sun high, front left", SceneLight { toward: [-0.55, 0.75, 0.55], rgb: [255, 244, 226], strength: 1.0, fill_rgb: [150, 170, 200], fill: 0.45 }),
        ("raking sun from the right", SceneLight { toward: [0.95, 0.2, 0.15], rgb: [255, 236, 200], strength: 1.1, fill_rgb: [120, 130, 150], fill: 0.3 }),
        ("dusk: low sun behind, left", SceneLight { toward: [-0.55, 0.15, -0.8], rgb: [255, 150, 80], strength: 1.0, fill_rgb: [90, 80, 120], fill: 0.35 }),
        ("night: moon, and a torch", SceneLight { toward: [0.4, 0.8, 0.3], rgb: [150, 170, 230], strength: 0.35, fill_rgb: [40, 50, 80], fill: 0.25 }),
    ];
    let torch = Lamp { pos: [-2.2, puppet.height * 0.7, 1.6], rgb: [255, 160, 74], reach: 9.0, strength: 1.0, beam: None };

    // rows: (label, texture, lit by the scene?)
    let mut rows: Vec<(&str, Texture, bool)> = vec![("today: cel light fixed to the view", tex(&paint_sheet, &map_sheet), false)];
    rows.push(("flat paint, scene light, the hull's normals", tex(&paint_sheet, &None), true));
    rows.push(("flat paint + bump from the paint", tex(&paint_sheet, &bump_sheet), true));
    if map_sheet.is_some() {
        rows.push(("flat paint + today's normal maps (big forms too)", tex(&paint_sheet, &map_sheet), true));
    }
    rows.push(("de-lit paint + bump", tex(&delit_sheet, &bump_sheet), true));
    rows.push(("light alone on grey, with the bump", tex(&grey, &bump_sheet), true));

    let (w, h) = (tile * lights.len(), tile * rows.len());
    let mut sheet: Vec<u8> = (0..w * h).flat_map(|_| [BACKGROUND[0], BACKGROUND[1], BACKGROUND[2], 255]).collect();
    // The scene-lit rows draw no contour lines (Niffy's flat hull has none,
    // and a dark line sits exactly where a low sun's rim light falls) unless
    // LIGHT_STRIP_OUTLINE=1.
    let scene_outline = std::env::var("LIGHT_STRIP_OUTLINE").is_ok_and(|v| v == "1");
    for (r, (_, texture, scene)) in rows.iter().enumerate() {
        let look = if *scene && !scene_outline { Look { yaw, outline_px: 0.0, ..Look::default() } } else { Look { yaw, ..Look::default() } };
        let mut cam = Camera::framing(puppet.height.max(0.5) * 1.15, [tile as f32, tile as f32]);
        cam.center[1] = tile as f32 * 0.55;
        let project = |p: gs_puppet::math::V3| cam.project(p);
        let (skin, surface) = posed.skin_surface(&project, &look, Some(texture), Effects::default());
        for (c, (_, light)) in lights.iter().enumerate() {
            let mut rgba = if *scene { surface.scene_lit(light, &skin) } else { skin.rgba.clone() };
            if c == lights.len() - 1 {
                let base = Skin { origin: skin.origin, w: skin.w, h: skin.h, rgba: rgba.clone() };
                let add = surface.lamp_light(&[torch], &base);
                for (v, a) in rgba.iter_mut().zip(&add) {
                    *v = v.saturating_add(*a);
                }
            }
            for y in 0..skin.h {
                for x in 0..skin.w {
                    let (tx, ty) = (skin.origin[0] as i64 + x as i64, skin.origin[1] as i64 + y as i64);
                    if tx < 0 || ty < 0 || tx >= tile as i64 || ty >= tile as i64 {
                        continue;
                    }
                    let s = &rgba[(y * skin.w + x) * 4..][..4];
                    let d = ((r * tile + ty as usize) * w + c * tile + tx as usize) * 4;
                    let a = s[3] as f32 / 255.0;
                    for k in 0..3 {
                        sheet[d + k] = (s[k] as f32 * a + sheet[d + k] as f32 * (1.0 - a)) as u8;
                    }
                }
            }
        }
    }
    let file = std::fs::File::create(&out).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut wr| wr.write_image_data(&sheet)).expect("write png");
    println!("wrote {} ({family}: {} paintings, {} normal maps)", out.display(), paint.iter().flatten().count(), maps.iter().flatten().count());
    println!("columns: {}", lights.iter().map(|l| l.0).collect::<Vec<_>>().join(" | "));
    for (i, (label, _, _)) in rows.iter().enumerate() {
        println!("row {i}: {label}");
    }
}
