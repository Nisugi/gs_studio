//! splat_preview: one family's puppet as a Gaussian splat cloud, beside its
//! smooth skin, from several sides -- to judge the look of splats on the
//! stage before any GPU work. Every ring of the posed puppet is sampled
//! round its circumference; each sample becomes an anisotropic Gaussian
//! (long along the ring and along the bone, thin through the surface),
//! coloured from the family's painted sheet where it has one, and the
//! cloud is drawn on the CPU: sorted back to front and alpha-blended.
//!
//!   cargo run --release -p gs_puppet --example splat_preview -- <out.png> <family> [pose] [yaw ...]
//!
//! Tiles come in pairs per yaw: the skin (what the stage draws today) on
//! the left, the splats on the right. Yaws default to 0 45 90 135.
//! `SPLAT_SHEET=<sheet.png>` wears that sheet (default: the family's
//! `data/textures/<family>/sheet.png` when present, else the body colour);
//! `SPLAT_SPACING=<ft>` sets the sample spacing (default 0.04);
//! `SPLAT_TILE=<px>` the tile size (default 320); `SPLAT_DATA=<dir>`
//! another data root.

use std::path::Path;

use gs_puppet::math::{add, cross, dot, len, norm_or, scale, sub, V3};
use gs_puppet::raster::Canvas;
use gs_puppet::{ground_grid, Camera, Look, Pose, PoseSet, Puppet, Texture, UvLayout, BODY_DEFAULT, MASK_PX, SHEET_SIDE};
use rig::{Outlines, Rig, View};

const BACKGROUND: [u8; 3] = [0x2b, 0x30, 0x36];
const GRID: [u8; 4] = [0x4a, 0x52, 0x5c, 255];

struct Gaussian {
    c: V3,
    /// Half-extents as world vectors: along the ring, along the bone,
    /// through the surface.
    axes: [V3; 3],
    n: V3,
    rgb: [f32; 3],
    alpha: f32,
}

fn read_png(path: &Path) -> Option<(usize, usize, Vec<u8>)> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = png::Decoder::new(std::io::BufReader::new(file)).read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..w * h * 4].to_vec(),
        png::ColorType::Rgb => buf[..w * h * 3].chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        _ => return None,
    };
    Some((w, h, rgba))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(out), Some(family)) = (args.first(), args.get(1)) else {
        eprintln!("usage: splat_preview <out.png> <family> [pose] [yaw ...]");
        std::process::exit(2);
    };
    let pose_name = args.get(2).map(String::as_str).unwrap_or("stand");
    let mut yaws: Vec<f32> = args.iter().skip(3).filter_map(|a| a.parse().ok()).collect();
    if yaws.is_empty() {
        yaws = vec![0.0, 45.0, 90.0, 135.0];
    }
    let tile: usize = std::env::var("SPLAT_TILE").ok().and_then(|v| v.parse().ok()).unwrap_or(320);
    let spacing: f32 = std::env::var("SPLAT_SPACING").ok().and_then(|v| v.parse().ok()).unwrap_or(0.04);

    let data = std::env::var_os("SPLAT_DATA").map(std::path::PathBuf::from).unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data"));
    let outlines = Outlines::load(&data.join("outlines.json")).expect("outlines.json");
    let rig = Rig::load(&data.join("rigs").join(Rig::file_name(family))).expect("rig loads");
    let mask = |v: View| outlines.get(family, v).map(|e| rig_bake::rasterize(&e.outline(), MASK_PX));
    let puppet = Puppet::build_masks(&rig, mask(View::Front).as_ref(), mask(View::Side).as_ref()).expect("puppet builds");
    let set = PoseSet::load(&data.join("poses"), &rig.body_type).expect("pose set").for_family(family).into_owned();
    let pose = if pose_name == "rest" { Pose::default() } else { set.clamp(set.get(pose_name).expect("no such pose")) };
    let posed = puppet.pose(&pose);

    // The painted sheet, when it fits this puppet's pattern.
    let sheet_path = std::env::var_os("SPLAT_SHEET").map(std::path::PathBuf::from).unwrap_or_else(|| data.join("textures").join(family).join("sheet.png"));
    let layout = UvLayout::fit(&puppet, SHEET_SIDE);
    let texture = read_png(&sheet_path).and_then(|(w, h, rgba)| {
        if (w, h) == (layout.w, layout.h) {
            Some(Texture { layout: layout.clone(), rgba, normal: None })
        } else {
            eprintln!("{}: {w}x{h} does not fit the pattern ({}x{}), body colour instead", sheet_path.display(), layout.w, layout.h);
            None
        }
    });
    let body = BODY_DEFAULT.map(|c| c as f32);

    // Rings -> Gaussians.
    let mut cloud: Vec<Gaussian> = Vec::new();
    for (i, s) in puppet.segments.iter().enumerate() {
        if s.hidden || !s.kind.is_body() {
            continue;
        }
        let xf = &posed.xf[i];
        let n = s.stations.len();
        for (k, st) in s.stations.iter().enumerate() {
            // Buried rings (a thigh in the pelvis, a socket stub) are inside
            // the body: a surface hides them, a cloud would show them.
            if k < s.bare {
                continue;
            }
            let c = xf.point(st.c);
            let a = scale(xf.dir(s.axis_w), st.w);
            let b = scale(xf.dir(s.axis_d), st.d);
            // Along the bone: half the gap to the neighbouring rings.
            let along = {
                let prev = if k > 0 { xf.point(s.stations[k - 1].c) } else { c };
                let next = if k + 1 < n { xf.point(s.stations[k + 1].c) } else { c };
                scale(sub(next, prev), 0.5)
            };
            let along = if len(along) < 1e-4 { scale(norm_or(xf.dir(sub(s.tail, s.head)), [0.0, 1.0, 0.0]), spacing * 0.5) } else { along };
            let circ = std::f32::consts::PI * (3.0 * (st.w + st.d) - ((3.0 * st.w + st.d) * (st.w + 3.0 * st.d)).sqrt());
            let m = ((circ / spacing).ceil() as usize).clamp(6, 400);
            for j in 0..m {
                let theta = j as f32 / m as f32 * std::f32::consts::TAU;
                let (sin, cos) = theta.sin_cos();
                let radial = add(scale(a, cos), scale(b, sin));
                let p = add(c, radial);
                let tangent = sub(scale(b, cos), scale(a, sin));
                let nrm = norm_or(cross(tangent, along), norm_or(radial, [0.0, 0.0, 1.0]));
                let nrm = if dot(nrm, radial) < 0.0 { scale(nrm, -1.0) } else { nrm };
                let rgb = texture
                    .as_ref()
                    .and_then(|t| t.layout.locate(i, k as f32, theta).and_then(|at| t.sample(at)))
                    .unwrap_or(body);
                // Half-extents at about 0.6 of the sample spacing each way:
                // neighbours overlap enough to close, not enough to blur.
                let half_ring = scale(norm_or(tangent, [1.0, 0.0, 0.0]), circ / m as f32 * 0.6);
                cloud.push(Gaussian { c: p, axes: [half_ring, scale(along, 0.6), scale(nrm, spacing * 0.15)], n: nrm, rgb, alpha: 0.9 });
            }
        }
    }
    println!("{family} {pose_name}: {} gaussians ({} px tiles)", cloud.len(), tile);

    // Frame as the turnaround does.
    let [lo, hi] = posed.bounds();
    let reach = (lo[0].abs().max(hi[0].abs())).hypot(lo[2].abs().max(hi[2].abs()));
    let size = (hi[1] - lo[1]).max(reach * 1.4).max(0.5) * 1.1;
    let (w, h) = (tile * yaws.len() * 2, tile);
    let mut canvas = Canvas::new(w, h, 1, BACKGROUND);
    let mut cams = Vec::new();
    for k in 0..yaws.len() * 2 {
        let mut cam = Camera::framing(size, [tile as f32, tile as f32]);
        cam.target = [0.0, (hi[1] + lo[1]) * 0.5, 0.0];
        cam.center = [tile as f32 * (k as f32 + 0.5), tile as f32 * 0.5];
        canvas.draw(&ground_grid(&|p| cam.project(p), 5, GRID, 1.0));
        cams.push(cam);
    }
    let mut rgba = canvas.to_rgba8();
    let light = norm_or([0.3, 0.8, 0.6], [0.0, 1.0, 0.0]);
    for (t, yaw) in yaws.iter().enumerate() {
        // Left: the skin.
        let cam = &cams[t * 2];
        let skin = posed.skin_textured(&|p| cam.project(p), &Look { yaw: *yaw, outline_px: 1.5, ..Look::default() }, texture.as_ref());
        for y in 0..skin.h {
            for x in 0..skin.w {
                let (tx, ty) = (skin.origin[0] as i64 + x as i64, skin.origin[1] as i64 + y as i64);
                if tx < 0 || ty < 0 || tx >= w as i64 || ty >= h as i64 {
                    continue;
                }
                let s = &skin.rgba[(y * skin.w + x) * 4..][..4];
                let a = s[3] as u32;
                let d = &mut rgba[(ty as usize * w + tx as usize) * 4..][..4];
                for c in 0..3 {
                    d[c] = ((s[c] as u32 * a + d[c] as u32 * (255 - a)) / 255) as u8;
                }
            }
        }
        // Right: the splats, the figure turned by `yaw` as the skin turns it.
        let cam = &cams[t * 2 + 1];
        let rot = gs_puppet::math::rot_y(*yaw);
        let turned = |v: V3| gs_puppet::math::mul_mv(&rot, v);
        let project = |p: V3| cam.project(turned(p));
        let mut order: Vec<(f32, usize)> = cloud.iter().enumerate().map(|(i, g)| (project(g.c).1, i)).collect();
        order.sort_by(|a, b| b.0.total_cmp(&a.0));
        // Straight float RGB over the background, back to front.
        let mut fb: Vec<[f32; 3]> = rgba.chunks_exact(4).map(|p| [p[0] as f32, p[1] as f32, p[2] as f32]).collect();
        let x0 = (t * 2 + 1) * tile;
        for (_, i) in order {
            let g = &cloud[i];
            let (centre, _) = project(g.c);
            // The 2D covariance from the projected axes.
            let mut cov = [[0.0f32; 2]; 2];
            for ax in g.axes {
                let (q, _) = project(add(g.c, ax));
                let v = [q[0] - centre[0], q[1] - centre[1]];
                for r in 0..2 {
                    for c in 0..2 {
                        cov[r][c] += v[r] * v[c];
                    }
                }
            }
            // Soften: a splat covers at least a pixel.
            cov[0][0] += 0.3;
            cov[1][1] += 0.3;
            let det = cov[0][0] * cov[1][1] - cov[0][1] * cov[1][0];
            if det <= 1e-6 {
                continue;
            }
            let inv = [[cov[1][1] / det, -cov[0][1] / det], [-cov[1][0] / det, cov[0][0] / det]];
            let rx = (3.0 * cov[0][0].sqrt()).ceil() as i64;
            let ry = (3.0 * cov[1][1].sqrt()).ceil() as i64;
            let shade = 0.55 + 0.45 * dot(turned(g.n), light).max(0.0);
            let (cx, cy) = (centre[0], centre[1]);
            let xa = ((cx as i64) - rx).max(x0 as i64);
            let xb = ((cx as i64) + rx).min((x0 + tile) as i64 - 1);
            let ya = ((cy as i64) - ry).max(0);
            let yb = ((cy as i64) + ry).min(h as i64 - 1);
            for py in ya..=yb {
                for px in xa..=xb {
                    let d = [px as f32 + 0.5 - cx, py as f32 + 0.5 - cy];
                    let m = d[0] * (inv[0][0] * d[0] + inv[0][1] * d[1]) + d[1] * (inv[1][0] * d[0] + inv[1][1] * d[1]);
                    if m > 9.0 {
                        continue;
                    }
                    let a = (g.alpha * (-0.5 * m).exp()).min(0.99);
                    let dst = &mut fb[py as usize * w + px as usize];
                    for c in 0..3 {
                        dst[c] = g.rgb[c] * shade * a + dst[c] * (1.0 - a);
                    }
                }
            }
        }
        for (p, px) in fb.iter().zip(rgba.chunks_exact_mut(4)) {
            px[0] = p[0].round().clamp(0.0, 255.0) as u8;
            px[1] = p[1].round().clamp(0.0, 255.0) as u8;
            px[2] = p[2].round().clamp(0.0, 255.0) as u8;
        }
    }
    let file = std::fs::File::create(out).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut wr| wr.write_image_data(&rgba)).expect("write png");
    println!("-> {out}");
}
