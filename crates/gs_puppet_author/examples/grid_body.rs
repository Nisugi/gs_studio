//! A grid figure for the injury doll: a form's modelled body painted with
//! a plain grid (the humanoid's grid skin's slate and pale 0.25 ft lines,
//! `uv_sheet --grid`), as a form of its own -- an extra in the doll's
//! roster (`injury_doll::roster`).
//!
//!     cargo run --release -p gs_puppet --example grid_body -- <mesh dir> <puppet.toml> <out dir> [<form>] [--size 2048] [--line 0.009]
//!     cargo run --release -p gs_puppet --example grid_body -- data/meshes/half_elf/40k data/meshes/half_elf/puppet.toml data/meshes/grid_figure
//!
//! The lines are laid in the body's own space, not the sheet's: a mesh made
//! by an image-to-3D model has its sheet cut into scattered islands, and a
//! grid drawn on them would break at every seam. Each of the three families
//! of planes (x, y, z every 0.25 ft, feet once the body stands its puppet's
//! height) draws where it crosses the surface, fading where the surface
//! runs along it (a plane meeting the surface edge-on would smear into a
//! band). Writes `body.obj` and `weights.txt` (copied), `color.png` (the
//! grid, each island grown a few texels past its edge so the sheet's
//! filtering never reaches the empty sheet), `puppet.toml` (the source's,
//! as `<form>`) and `doll.toml` (an extra, drawn as its own trained splats:
//! train them with image-gen/grid_pack.sh after this).

use gs_puppet::mesh::Mesh;
use gs_puppet::Puppet;
use std::path::{Path, PathBuf};

const GRID_FT: f32 = 0.25;
const GRID_BASE: [f32; 3] = [58.0, 66.0, 74.0];
const GRID_LINE: [f32; 3] = [158.0, 176.0, 184.0];
/// A line's half width on the surface, feet (`--line` overrides: a trained
/// splat cloud needs wider lines than a made one, 0.02 or so).
const LINE_FT: f32 = 0.009;
static LINE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
fn line_ft() -> f32 {
    *LINE.get().unwrap_or(&LINE_FT)
}
/// Texels an island is grown past its edge.
const GROW: usize = 6;

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The grid's colour at body point `p` (feet) facing `n` (unit).
fn grid(p: [f32; 3], n: [f32; 3]) -> [f32; 3] {
    let mut t = 0.0f32;
    for k in 0..3 {
        let along = n[k].abs();
        // the plane runs along the surface here: no line
        let fade = 1.0 - smoothstep(0.78, 0.94, along);
        if fade <= 0.0 {
            continue;
        }
        let m = (p[k] / GRID_FT).rem_euclid(1.0);
        // how far from the plane, measured on the surface
        let d = m.min(1.0 - m) * GRID_FT / (1.0 - along * along).sqrt().max(0.2);
        t = t.max((1.0 - smoothstep(line_ft() * 0.5, line_ft(), d)) * fade);
    }
    [0, 1, 2].map(|k| GRID_BASE[k] + (GRID_LINE[k] - GRID_BASE[k]) * t)
}

fn norm(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 1e-9 {
        v.map(|c| c / l)
    } else {
        [0.0, 0.0, 1.0]
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let size = match args.iter().position(|a| a == "--size") {
        Some(i) => {
            let v = args.remove(i + 1);
            args.remove(i);
            v.parse().expect("--size <texels>")
        }
        None => 2048usize,
    };
    if let Some(i) = args.iter().position(|a| a == "--line") {
        let v = args.remove(i + 1);
        args.remove(i);
        LINE.set(v.parse().expect("--line <feet>")).ok();
    }
    if args.len() < 3 {
        eprintln!("grid_body <mesh dir> <puppet.toml> <out dir> [<form>] [--size 2048] [--line 0.009]");
        std::process::exit(2);
    }
    let (mesh_dir, puppet_file, out) = (PathBuf::from(&args[0]), PathBuf::from(&args[1]), PathBuf::from(&args[2]));
    let form = args.get(3).cloned().unwrap_or_else(|| out.file_name().expect("an out dir name").to_string_lossy().into_owned());
    let puppet_text = std::fs::read_to_string(&puppet_file).unwrap_or_else(|e| panic!("{}: {e}", puppet_file.display()));
    let puppet = Puppet::from_baked_toml(&puppet_text).unwrap_or_else(|e| panic!("{}: {e}", puppet_file.display()));
    let mut mesh = Mesh::load_obj(&mesh_dir.join("body.obj")).unwrap_or_else(|e| panic!("{}: {e}", mesh_dir.display()));
    mesh.fit_to_height(puppet.height);
    println!("{} triangles, {} ft tall, a {size} sheet", mesh.tris.len(), puppet.height);

    // the sheet: each triangle's texels, painted from where they are on the body
    let mut rgb = vec![[0.0f32; 3]; size * size];
    let mut filled = vec![false; size * size];
    let s = size as f32;
    for tri in &mesh.tris {
        let [a, b, c] = tri.map(|v| v as usize);
        let uv = [a, b, c].map(|v| [mesh.uv[v][0] * s, mesh.uv[v][1] * s]);
        let (x0, x1) = (uv.iter().map(|q| q[0]).fold(f32::MAX, f32::min).floor().max(0.0) as usize, (uv.iter().map(|q| q[0]).fold(f32::MIN, f32::max).ceil() as usize).min(size - 1));
        let (y0, y1) = (uv.iter().map(|q| q[1]).fold(f32::MAX, f32::min).floor().max(0.0) as usize, (uv.iter().map(|q| q[1]).fold(f32::MIN, f32::max).ceil() as usize).min(size - 1));
        let area = (uv[1][0] - uv[0][0]) * (uv[2][1] - uv[0][1]) - (uv[2][0] - uv[0][0]) * (uv[1][1] - uv[0][1]);
        if area.abs() < 1e-12 {
            continue;
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                let q = [x as f32 + 0.5, y as f32 + 0.5];
                let w1 = ((q[0] - uv[0][0]) * (uv[2][1] - uv[0][1]) - (uv[2][0] - uv[0][0]) * (q[1] - uv[0][1])) / area;
                let w2 = ((uv[1][0] - uv[0][0]) * (q[1] - uv[0][1]) - (q[0] - uv[0][0]) * (uv[1][1] - uv[0][1])) / area;
                let w0 = 1.0 - w1 - w2;
                // a texel's centre in the triangle, or just over its edge
                if w0 < -0.02 || w1 < -0.02 || w2 < -0.02 {
                    continue;
                }
                let at = |f: &dyn Fn(usize) -> [f32; 3]| -> [f32; 3] { [0, 1, 2].map(|k| f(a)[k] * w0 + f(b)[k] * w1 + f(c)[k] * w2) };
                let p = at(&|v| mesh.pos[v]);
                let n = norm(at(&|v| mesh.nrm[v]));
                let i = y * size + x;
                rgb[i] = grid(p, n);
                filled[i] = true;
            }
        }
    }
    // grow each island past its edge, so filtering never reads the empty sheet
    for _ in 0..GROW {
        let was = filled.clone();
        for y in 0..size {
            for x in 0..size {
                let i = y * size + x;
                if was[i] {
                    continue;
                }
                let (mut sum, mut n) = ([0.0f32; 3], 0.0f32);
                for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= size as i32 || ny >= size as i32 {
                        continue;
                    }
                    let j = ny as usize * size + nx as usize;
                    if was[j] {
                        sum = [0, 1, 2].map(|k| sum[k] + rgb[j][k]);
                        n += 1.0;
                    }
                }
                if n > 0.0 {
                    rgb[i] = sum.map(|v| v / n);
                    filled[i] = true;
                }
            }
        }
    }
    let covered = filled.iter().filter(|f| **f).count();
    let bytes: Vec<u8> = rgb.iter().zip(&filled).flat_map(|(c, f)| {
        let c = if *f { *c } else { GRID_BASE };
        [c[0].round() as u8, c[1].round() as u8, c[2].round() as u8, 255]
    }).collect();
    std::fs::create_dir_all(&out).expect("the out dir");
    write_png(&out.join("color.png"), size, size, &bytes);
    for f in ["body.obj", "weights.txt"] {
        let src = mesh_dir.join(f);
        if src.is_file() {
            std::fs::copy(&src, out.join(f)).unwrap_or_else(|e| panic!("{}: {e}", src.display()));
        }
    }
    // its own puppet: the source's skeleton and rings, as this form
    let renamed: String = puppet_text
        .lines()
        .map(|l| if l.trim_start().starts_with("family =") { format!("family = \"{form}\"") } else { l.to_owned() })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(out.join("puppet.toml"), renamed).expect("puppet.toml");
    let doll = out.join("doll.toml");
    if !doll.exists() {
        std::fs::write(&doll, "# The injury doll's grid figure: an extra, drawn as its own TRAINED splats\n# (splat.ply: Brush on unlit orbit pictures of this sheet, image-gen/grid_pack.sh).\nlabel = \"Grid figure\"\nkind = \"extra\"\ndraw = \"splats\"\n").expect("doll.toml");
    }
    println!("wrote {} ({:.0}% of the sheet painted) as form '{form}'", out.display(), covered as f32 * 100.0 / (size * size) as f32);
}

fn write_png(path: &Path, w: usize, h: usize, rgba: &[u8]) {
    let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())));
    let mut enc = png::Encoder::new(file, w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut wr| wr.write_image_data(rgba)).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}
