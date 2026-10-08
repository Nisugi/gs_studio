//! mesh_project: a modelled mesh's sheet repainted from pictures of the
//! figure, by projection -- each picture laid over the mesh as seen from its
//! side, every point of the surface taking the picture that sees it most
//! squarely.
//!
//!   cargo run --release -p gs_puppet --example mesh_project -- <mesh dir> <view>=<picture.png>[@<from>..<to>] ...
//!
//! `<mesh dir>` holds `body.obj` (y up, facing +z) and `color.png`; the sheet
//! is rewritten (`color_model.png` keeps the mesh maker's own). `<view>` is
//! where the picture was taken from: `front`, `back`, `left` (the figure's
//! left side, the figure facing left in the picture), `right` (facing right).
//! The figure in each picture stands on a plain ground colour; it is found
//! by that, and its box fitted to the mesh's as that side sees it.
//!
//! `@<from>..<to>` (shares of the figure's height, 0 its feet .. 1 its top)
//! says the picture is of that band only -- a close-up of the head is
//! `front=head.png@0.84..1` -- and fits and paints only there, over the
//! whole-figure pictures (a close-up counts for more).
//!
//! A picture is lit by its painter, whatever it was asked: surfaces that
//! face down or away come out darker, and that shading would be painted on
//! (and then lit again by the renderer). Each picture's light is estimated
//! from the mesh -- how its brightness runs with the way the surface faces,
//! material by material -- and divided out (`MESH_DELIGHT=0`: not; `=<k>`
//! takes that share of it, 1 by default).
//!
//! What no picture sees squarely (the top of an out-held arm, a sole) is
//! painted from the nearest surface that one does. `MESH_SHEET=<px>` sets the
//! sheet's side (default: as `color.png`).

use std::path::Path;

use gs_puppet::math::{add, cross, dot, norm_or, scale, sub, V3};
use gs_puppet::mesh::{Mesh, Paint};

/// A surface must face a picture's camera at least this much (the cosine) to
/// be painted from it: nearer edge-on, a pixel covers a long strip of it.
const FACING_MIN: f32 = 0.25;
/// How far behind the front-most surface a point may be and still be seen
/// (shares of the figure's height).
const SEEN_EPS: f32 = 0.006;
/// The pictures' silhouettes are trusted this many pixels in from their edge.
const RIM_PX: i32 = 3;
/// A close-up counts this many times a whole-figure picture.
const CLOSE_UP: f32 = 6.0;

fn load_png(path: &Path) -> Paint {
    let mut reader = png::Decoder::new(std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).read_info().expect("a png");
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).expect("a png");
    let (w, h) = (info.width as usize, info.height as usize);
    let rgba = match (info.color_type, info.bit_depth) {
        (png::ColorType::Rgba, png::BitDepth::Eight) => buf[..w * h * 4].to_vec(),
        (png::ColorType::Rgb, png::BitDepth::Eight) => buf[..w * h * 3].chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        other => panic!("{}: {other:?} is not 8-bit RGB or RGBA", path.display()),
    };
    Paint { w, h, rgba }
}

fn save_png(path: &Path, w: usize, h: usize, rgb: &[[u8; 3]]) {
    let file = std::fs::File::create(path).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let bytes: Vec<u8> = rgb.iter().flatten().copied().collect();
    enc.write_header().and_then(|mut wr| wr.write_image_data(&bytes)).expect("write png");
}

/// One picture laid over the mesh.
struct View {
    name: String,
    /// Toward the camera, and the picture's right and down, in the mesh's space.
    to_cam: V3,
    right: V3,
    picture: Paint,
    /// Which of its pixels are the figure, `RIM_PX` in from the edge.
    figure: Vec<bool>,
    /// Picture pixel = `at + (right . p, -y) * by`.
    at: [f32; 2],
    by: [f32; 2],
    /// The band of the figure's height it shows (mesh y), and its weight.
    band: [f32; 2],
    weight: f32,
    /// Its painter's light: log brightness against the way a surface faces
    /// in the picture (`facing_terms`), less that of a surface facing the
    /// camera. All zero: none taken out.
    light: [f32; 9],
    /// Its colours times this agree with the pictures beside it (two
    /// paintings of one figure differ a little in tone: where they meet on
    /// the mesh the difference is an edge).
    tone: [f32; 3],
    /// The nearest surface along `to_cam` per cell of a grid over the mesh's
    /// box as this view sees it: (cells across, down, the grid's origin and step).
    depth: Vec<f32>,
    grid: (usize, usize, [f32; 2], f32),
}

impl View {
    fn cell(&self, p: V3) -> Option<usize> {
        let (gw, gh, o, step) = self.grid;
        let (x, y) = (((dot(p, self.right) - o[0]) / step) as isize, ((o[1] - p[1]) / step) as isize);
        (x >= 0 && y >= 0 && (x as usize) < gw && (y as usize) < gh).then(|| y as usize * gw + x as usize)
    }

    /// The picture's colour at `p` and how much it counts, when this view
    /// sees `p` (facing `n`) squarely enough.
    fn sample(&self, p: V3, n: V3, tall: f32) -> Option<([f32; 3], f32)> {
        if p[1] < self.band[0] || p[1] > self.band[1] {
            return None;
        }
        // Which side of a surface its facing names is not to be trusted (a
        // generated mesh has stretches facing inward): what the picture sees
        // is whatever is nearest its camera, whichever way that says it faces.
        let n = if dot(n, self.to_cam) < 0.0 { scale(n, -1.0) } else { n };
        let facing = dot(n, self.to_cam);
        if facing < FACING_MIN {
            return None;
        }
        let front = self.depth[self.cell(p)?];
        if dot(p, self.to_cam) < front - SEEN_EPS * tall {
            return None;
        }
        let (x, y) = (self.at[0] + dot(p, self.right) * self.by[0], self.at[1] - p[1] * self.by[1]);
        let (ix, iy) = (x.round() as isize, y.round() as isize);
        if ix < 0 || iy < 0 || ix as usize >= self.picture.w || iy as usize >= self.picture.h || !self.figure[iy as usize * self.picture.w + ix as usize] {
            return None;
        }
        // toward a band's ends a close-up fades into what is under it
        let span = (self.band[1] - self.band[0]).max(1e-6);
        let edge = if self.weight > 1.0 { (((p[1] - self.band[0]) / span).min(1.0) * 6.0).clamp(0.0, 1.0) } else { 1.0 };
        // its painter's light taken off: as bright as if it faced the camera
        let gain = (-self.shading(n)).exp().clamp(0.6, 1.9);
        let f = (facing - FACING_MIN) / (1.0 - FACING_MIN);
        let c = self.picture.sample([x / self.picture.w as f32, y / self.picture.h as f32]);
        // the picture that sees a surface most squarely has nearly all the say
        let f2 = f * f;
        Some((std::array::from_fn(|k| (c[k] * gain * self.tone[k]).min(255.0)), f2 * f2 * f2 * self.weight * edge))
    }

    /// A surface's facing as the picture sees it: to its right, up, and
    /// toward its camera.
    fn seen_facing(&self, n: V3) -> V3 {
        [dot(n, self.right), n[1], dot(n, self.to_cam)]
    }

    /// The log of how much brighter than a surface facing the camera the
    /// painter made one facing `n`.
    fn shading(&self, n: V3) -> f32 {
        let (t, front) = (facing_terms(self.seen_facing(n)), facing_terms([0.0, 0.0, 1.0]));
        (0..9).map(|k| self.light[k] * (t[k] - front[k])).sum()
    }
}

/// The terms a picture's light is fitted in: a constant, the facing, and its
/// squares (a light from one side, from above, and a sky's all-round one).
fn facing_terms(n: V3) -> [f32; 9] {
    let [x, y, z] = n;
    [1.0, x, y, z, x * y, y * z, x * z, x * x - y * y, 3.0 * z * z - 1.0]
}

/// The nine weights `w` least-squares fitting `rows` (terms, value): solved
/// by elimination, a little weight on keeping them small.
fn fit9(rows: &[([f32; 9], f32)]) -> [f32; 9] {
    let mut m = [[0.0f64; 10]; 9];
    for (t, v) in rows {
        for i in 0..9 {
            for j in 0..9 {
                m[i][j] += (t[i] * t[j]) as f64;
            }
            m[i][9] += (t[i] * v) as f64;
        }
    }
    for (i, row) in m.iter_mut().enumerate() {
        row[i] += 1e-3 * rows.len().max(1) as f64;
    }
    for i in 0..9 {
        let pivot = (i..9).max_by(|a, b| m[*a][i].abs().total_cmp(&m[*b][i].abs())).unwrap_or(i);
        m.swap(i, pivot);
        if m[i][i].abs() < 1e-12 {
            continue;
        }
        for r in 0..9 {
            if r != i {
                let k = m[r][i] / m[i][i];
                for c in i..10 {
                    m[r][c] -= k * m[i][c];
                }
            }
        }
    }
    std::array::from_fn(|i| if m[i][i].abs() < 1e-12 { 0.0 } else { (m[i][9] / m[i][i]) as f32 })
}

/// The figure in a picture: every pixel the ground does not reach -- the
/// ground being what is like the picture's corners in colour AND joined to
/// its edge (a shaded cheek can be as grey as the ground, and is not a hole
/// in the figure) -- then `RIM_PX` taken off its edge.
fn figure_mask(p: &Paint) -> Vec<bool> {
    let px = |x: usize, y: usize| -> [f32; 3] { std::array::from_fn(|c| p.rgba[(y * p.w + x) * 4 + c] as f32) };
    let corners = [px(2, 2), px(p.w - 3, 2), px(2, p.h - 3), px(p.w - 3, p.h - 3)];
    let ground: [f32; 3] = std::array::from_fn(|c| corners.iter().map(|k| k[c]).sum::<f32>() / 4.0);
    let like: Vec<bool> = (0..p.w * p.h).map(|i| (0..3).map(|c| (p.rgba[i * 4 + c] as f32 - ground[c]).abs()).fold(0.0, f32::max) <= 26.0).collect();
    let mut outside = vec![false; p.w * p.h];
    let mut queue: Vec<usize> = Vec::new();
    for x in 0..p.w {
        queue.extend([x, (p.h - 1) * p.w + x]);
    }
    for y in 0..p.h {
        queue.extend([y * p.w, y * p.w + p.w - 1]);
    }
    while let Some(i) = queue.pop() {
        if outside[i] || !like[i] {
            continue;
        }
        outside[i] = true;
        let (x, y) = (i % p.w, i / p.w);
        if x > 0 {
            queue.push(i - 1);
        }
        if x + 1 < p.w {
            queue.push(i + 1);
        }
        if y > 0 {
            queue.push(i - p.w);
        }
        if y + 1 < p.h {
            queue.push(i + p.w);
        }
    }
    let mut mask: Vec<bool> = outside.iter().map(|o| !o).collect();
    for _ in 0..RIM_PX {
        let was = mask.clone();
        for y in 1..p.h - 1 {
            for x in 1..p.w - 1 {
                let i = y * p.w + x;
                mask[i] = was[i] && was[i - 1] && was[i + 1] && was[i - p.w] && was[i + p.w];
            }
        }
    }
    mask
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: mesh_project <mesh dir> <view>=<picture.png>[@<from>..<to>] ...");
        std::process::exit(2);
    }
    let dir = Path::new(&args[0]);
    let mut mesh = Mesh::load_obj(&dir.join("body.obj")).expect("body.obj");
    // The way each point of the surface faces, from the triangles that meet
    // there IN SPACE. The file's own facings stop at the sheet's seams (a
    // reduced mesh is cut into thousands of pieces, a vertex per piece), so
    // they are faceted and noisy -- and which picture paints a surface is
    // decided by its facing: a face came out in angular patches, each from
    // a different picture.
    {
        let quant = |p: V3| p.map(|v| (v * 20000.0).round() as i64);
        let mut sum: std::collections::HashMap<[i64; 3], V3> = std::collections::HashMap::new();
        for t in &mesh.tris {
            let [a, b, c] = t.map(|i| mesh.pos[i as usize]);
            let n = cross(sub(b, a), sub(c, a));
            for p in [a, b, c] {
                let at = sum.entry(quant(p)).or_insert([0.0; 3]);
                *at = add(*at, n);
            }
        }
        // twice more, each point taking in the points it shares a triangle with
        let mut nrm: Vec<V3> = mesh.pos.iter().map(|p| norm_or(sum[&quant(*p)], [0.0, 0.0, 1.0])).collect();
        for _ in 0..2 {
            let mut next: std::collections::HashMap<[i64; 3], V3> = std::collections::HashMap::new();
            for t in &mesh.tris {
                let mean = t.iter().fold([0.0; 3], |a, i| add(a, nrm[*i as usize]));
                for i in t {
                    let at = next.entry(quant(mesh.pos[*i as usize])).or_insert([0.0; 3]);
                    *at = add(*at, mean);
                }
            }
            nrm = mesh.pos.iter().map(|p| norm_or(next[&quant(*p)], [0.0, 0.0, 1.0])).collect();
        }
        mesh.nrm = nrm;
    }
    let model = dir.join("color_model.png");
    if !model.exists() {
        std::fs::copy(dir.join("color.png"), &model).expect("keep the maker's sheet");
    }
    let old = load_png(&model);
    let side: usize = std::env::var("MESH_SHEET").ok().and_then(|v| v.parse().ok()).unwrap_or(old.w);
    let (lo, hi) = mesh.pos.iter().fold(([f32::MAX; 3], [f32::MIN; 3]), |(lo, hi), p| (std::array::from_fn(|k| lo[k].min(p[k])), std::array::from_fn(|k| hi[k].max(p[k]))));
    let tall = hi[1] - lo[1];

    // ---- the views
    let mut views: Vec<View> = Vec::new();
    for spec in &args[1..] {
        let (name, rest) = spec.split_once('=').unwrap_or_else(|| panic!("not <view>=<picture>: {spec}"));
        let (path, band) = match rest.rsplit_once('@') {
            Some((path, band)) if band.contains("..") => {
                let (a, b) = band.split_once("..").expect("checked");
                (path, Some([a.parse::<f32>().expect("a share"), b.parse::<f32>().expect("a share")]))
            }
            _ => (rest, None),
        };
        let (to_cam, right): (V3, V3) = match name {
            "front" => ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
            "back" => ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0]),
            "left" => ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
            "right" => ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            other => panic!("no view {other}: front, back, left, right"),
        };
        let band_y = band.map_or([lo[1] - 1.0, hi[1] + 1.0], |b| [lo[1] + b[0] * tall, lo[1] + b[1] * tall]);
        let picture = load_png(Path::new(path));
        let figure = figure_mask(&picture);
        // the figure's box in the picture, and the mesh's (that band of it) as this side sees it
        let (mut a, mut b) = ([usize::MAX; 2], [0usize; 2]);
        for y in 0..picture.h {
            for x in 0..picture.w {
                if figure[y * picture.w + x] {
                    a = [a[0].min(x), a[1].min(y)];
                    b = [b[0].max(x), b[1].max(y)];
                }
            }
        }
        let (mut u0, mut u1, mut y0, mut y1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for p in mesh.pos.iter().filter(|p| p[1] >= band_y[0] && p[1] <= band_y[1]) {
            let u = dot(*p, right);
            (u0, u1, y0, y1) = (u0.min(u), u1.max(u), y0.min(p[1]), y1.max(p[1]));
        }
        // (the mask lost RIM_PX each side)
        let (fa, fb) = ([a[0] as f32 - RIM_PX as f32, a[1] as f32 - RIM_PX as f32], [b[0] as f32 + RIM_PX as f32, b[1] as f32 + RIM_PX as f32]);
        let by = [(fb[0] - fa[0]) / (u1 - u0), (fb[1] - fa[1]) / (y1 - y0)];
        let at = [fa[0] - u0 * by[0], fa[1] + y1 * by[1]];
        // its depth grid: every triangle's corners and middle are enough at this step
        let step = tall / 900.0;
        let (mut g0, mut g1) = (f32::MAX, f32::MIN);
        for p in &mesh.pos {
            let u = dot(*p, right);
            (g0, g1) = (g0.min(u), g1.max(u));
        }
        let grid = (((g1 - g0) / step) as usize + 2, (tall / step) as usize + 2, [g0, hi[1]], step);
        let mut view = View { name: name.to_owned(), to_cam, right, picture, figure, at, by, band: band_y, weight: if band.is_some() { CLOSE_UP } else { 1.0 }, light: [0.0; 9], tone: [1.0; 3], depth: vec![f32::MIN; grid.0 * grid.1], grid };
        for t in &mesh.tris {
            let [pa, pb, pc] = t.map(|i| mesh.pos[i as usize]);
            // points over the triangle, a grid step apart
            let n = ((len3(sub(pb, pa)).max(len3(sub(pc, pa))) / step).ceil() as usize + 1).min(64);
            for i in 0..=n {
                for j in 0..=n - i {
                    let (u, v) = (i as f32 / n as f32, j as f32 / n as f32);
                    let p = add(add(scale(pa, 1.0 - u - v), scale(pb, u)), scale(pc, v));
                    if let Some(c) = view.cell(p) {
                        view.depth[c] = view.depth[c].max(dot(p, to_cam));
                    }
                }
            }
        }
        if std::env::var_os("MESH_PROJECT_WHY").is_some() {
            // of the triangles that face this picture: how many it paints, and why not the rest
            let (mut facing, mut hidden, mut off, mut ground, mut ok) = (0, 0, 0, 0, 0);
            for t in &mesh.tris {
                let [ia, ib, ic] = t.map(|i| i as usize);
                let p = scale(add(add(mesh.pos[ia], mesh.pos[ib]), mesh.pos[ic]), 1.0 / 3.0);
                let n = norm_or(add(add(mesh.nrm[ia], mesh.nrm[ib]), mesh.nrm[ic]), [0.0, 0.0, 1.0]);
                if dot(n, view.to_cam).abs() < 0.5 || p[1] < view.band[0] || p[1] > view.band[1] {
                    continue;
                }
                facing += 1;
                let Some(c) = view.cell(p) else { off += 1; continue };
                if dot(p, view.to_cam) < view.depth[c] - SEEN_EPS * tall {
                    hidden += 1;
                    continue;
                }
                let (x, y) = ((view.at[0] + dot(p, view.right) * view.by[0]).round() as isize, (view.at[1] - p[1] * view.by[1]).round() as isize);
                if x < 0 || y < 0 || x as usize >= view.picture.w || y as usize >= view.picture.h {
                    off += 1;
                } else if !view.figure[y as usize * view.picture.w + x as usize] {
                    ground += 1;
                } else {
                    ok += 1;
                }
            }
            println!("  {name}: of {facing} triangles facing it, {ok} painted, {hidden} behind something, {ground} on the picture's ground, {off} off the picture");
        }
        println!("{name}: {path} ({} x {}), the figure {} x {} px, {:.0} px a unit", view.picture.w, view.picture.h, b[0] - a[0], b[1] - a[1], by[1]);
        views.push(view);
    }

    // ---- each picture's light, from the triangles it sees: brightness against
    // facing, within each material (colours alike in hue: a dark boot beside
    // a pale face says nothing of the light)
    let delight: f32 = std::env::var("MESH_DELIGHT").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    if delight > 0.0 {
        for k in 0..views.len() {
            // (how bright, its hue as two shares of its brightness, its facing)
            let mut seen: Vec<(f32, [f32; 2], V3)> = Vec::new();
            for t in &mesh.tris {
                let [ia, ib, ic] = t.map(|i| i as usize);
                let p = scale(add(add(mesh.pos[ia], mesh.pos[ib]), mesh.pos[ic]), 1.0 / 3.0);
                let n = norm_or(add(add(mesh.nrm[ia], mesh.nrm[ib]), mesh.nrm[ic]), [0.0, 0.0, 1.0]);
                if let Some((c, _)) = views[k].sample(p, n, tall) {
                    let n = if dot(n, views[k].to_cam) < 0.0 { scale(n, -1.0) } else { n };
                    let sum = (c[0] + c[1] + c[2]).max(3.0);
                    seen.push(((sum / 3.0).ln(), [c[0] / sum, c[1] / sum], views[k].seen_facing(n)));
                }
            }
            if seen.len() < 200 {
                continue;
            }
            // materials: hue cells, a twelfth of the triangle of hues across
            let cell_of = |h: [f32; 2]| ((h[0] * 12.0) as usize).min(11) * 12 + ((h[1] * 12.0) as usize).min(11);
            let mut mean = vec![(0.0f32, 0u32); 144];
            for (l, h, _) in &seen {
                let m = &mut mean[cell_of(*h)];
                *m = (m.0 + l, m.1 + 1);
            }
            let rows: Vec<([f32; 9], f32)> = seen
                .iter()
                .filter(|(_, h, _)| mean[cell_of(*h)].1 >= 40)
                .map(|(l, h, n)| {
                    let m = mean[cell_of(*h)];
                    (facing_terms(*n), l - m.0 / m.1 as f32)
                })
                .collect();
            let fit = fit9(&rows);
            views[k].light = fit.map(|w| w * delight);
            let at = |n: V3| views[k].shading(n).exp();
            println!(
                "  {}'s light, against a surface facing its camera: facing up x{:.2}, down x{:.2}, to its left x{:.2}, right x{:.2}",
                views[k].name,
                at(add(scale(views[k].to_cam, 0.5), [0.0, 0.87, 0.0])),
                at(add(scale(views[k].to_cam, 0.5), [0.0, -0.87, 0.0])),
                at(add(scale(views[k].to_cam, 0.5), scale(views[k].right, -0.87))),
                at(add(scale(views[k].to_cam, 0.5), scale(views[k].right, 0.87))),
            );
        }
    }

    // ---- each picture's tone brought to the first's: over the triangles two
    // pictures both see, the later one's colours scaled to the mean of those
    // already matched (front, then the sides against it, then the back
    // against the sides)
    for k in 1..views.len() {
        let (mut mine, mut theirs, mut n) = ([0.0f32; 3], [0.0f32; 3], 0usize);
        for t in &mesh.tris {
            let [ia, ib, ic] = t.map(|i| i as usize);
            let p = scale(add(add(mesh.pos[ia], mesh.pos[ib]), mesh.pos[ic]), 1.0 / 3.0);
            let nrm = norm_or(add(add(mesh.nrm[ia], mesh.nrm[ib]), mesh.nrm[ic]), [0.0, 0.0, 1.0]);
            let Some((c, _)) = views[k].sample(p, nrm, tall) else { continue };
            let (mut sum, mut total) = ([0.0f32; 3], 0.0f32);
            for other in &views[..k] {
                if let Some((o, w)) = other.sample(p, nrm, tall) {
                    sum = add(sum, scale(o, w));
                    total += w;
                }
            }
            if total > 0.0 {
                mine = add(mine, c);
                theirs = add(theirs, scale(sum, 1.0 / total));
                n += 1;
            }
        }
        if n >= 200 {
            views[k].tone = std::array::from_fn(|c| (theirs[c] / mine[c].max(1.0)).clamp(0.75, 1.35));
            println!("  {}'s tone against those before it: x{:.2} {:.2} {:.2} (over {n} triangles both see)", views[k].name, views[k].tone[0], views[k].tone[1], views[k].tone[2]);
        }
    }

    // ---- the sheet: every texel a triangle covers, painted from the views
    let mut sheet: Vec<[u8; 3]> = (0..side * side).map(|i| {
        let (x, y) = (i % side, i / side);
        old.sample([(x as f32 + 0.5) / side as f32, (y as f32 + 0.5) / side as f32]).map(|c| c.round() as u8)
    }).collect();
    let mut state = vec![0u8; side * side]; // 0 outside every triangle, 1 painted, 2 seen by no picture
    let mut unseen: Vec<(u32, V3)> = Vec::new();
    // the painted colours, in a grid over the mesh's box, for what no picture sees
    let cell = tall / 260.0;
    let dims: [usize; 3] = std::array::from_fn(|k| ((hi[k] - lo[k]) / cell) as usize + 3);
    let at_cell = |p: V3| -> [f32; 3] { std::array::from_fn(|k| (p[k] - lo[k]) / cell + 1.0) };
    let index = |c: [usize; 3]| (c[2] * dims[1] + c[1]) * dims[0] + c[0];
    let mut known: Vec<([f32; 3], f32)> = vec![([0.0; 3], 0.0); dims[0] * dims[1] * dims[2]];
    let mut used = vec![0usize; views.len()];
    let why = std::env::var_os("MESH_PROJECT_WHY").is_some();
    for t in &mesh.tris {
        let [ia, ib, ic] = t.map(|i| i as usize);
        let uv = [mesh.uv[ia], mesh.uv[ib], mesh.uv[ic]].map(|u| [u[0] * side as f32, u[1] * side as f32]);
        let den = (uv[1][1] - uv[2][1]) * (uv[0][0] - uv[2][0]) + (uv[2][0] - uv[1][0]) * (uv[0][1] - uv[2][1]);
        if den.abs() < 1e-9 {
            continue;
        }
        let face = norm_or(cross(sub(mesh.pos[ib], mesh.pos[ia]), sub(mesh.pos[ic], mesh.pos[ia])), [0.0, 0.0, 1.0]);
        let x0 = uv.iter().map(|u| u[0]).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
        let x1 = (uv.iter().map(|u| u[0]).fold(f32::MIN, f32::max).ceil() as usize).min(side - 1);
        let y0 = uv.iter().map(|u| u[1]).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
        let y1 = (uv.iter().map(|u| u[1]).fold(f32::MIN, f32::max).ceil() as usize).min(side - 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (qx, qy) = (x as f32 + 0.5, y as f32 + 0.5);
                let wa = ((uv[1][1] - uv[2][1]) * (qx - uv[2][0]) + (uv[2][0] - uv[1][0]) * (qy - uv[2][1])) / den;
                let wb = ((uv[2][1] - uv[0][1]) * (qx - uv[2][0]) + (uv[0][0] - uv[2][0]) * (qy - uv[2][1])) / den;
                let wc = 1.0 - wa - wb;
                // a little past its edges: the sheet's gutter
                if wa < -0.08 || wb < -0.08 || wc < -0.08 {
                    continue;
                }
                let i = y * side + x;
                let inside = wa >= 0.0 && wb >= 0.0 && wc >= 0.0;
                if state[i] != 0 && !inside {
                    continue;
                }
                let p = add(add(scale(mesh.pos[ia], wa), scale(mesh.pos[ib], wb)), scale(mesh.pos[ic], wc));
                let n = norm_or(add(add(scale(mesh.nrm[ia], wa), scale(mesh.nrm[ib], wb)), scale(mesh.nrm[ic], wc)), face);
                let (mut sum, mut total) = ([0.0f32; 3], 0.0f32);
                for (k, view) in views.iter().enumerate() {
                    if let Some((c, w)) = view.sample(p, n, tall) {
                        // MESH_PROJECT_WHY: each picture's share shown as a colour
                        // (front red, left green, right blue, back yellow)
                        let c = if why { [[255.0, 0.0, 0.0], [0.0, 255.0, 0.0], [0.0, 0.0, 255.0], [255.0, 255.0, 0.0]][k % 4] } else { c };
                        sum = add(sum, scale(c, w));
                        total += w;
                        used[k] += 1;
                    }
                }
                if total > 0.02 {
                    let c = scale(sum, 1.0 / total);
                    sheet[i] = c.map(|v| v.round().clamp(0.0, 255.0) as u8);
                    state[i] = 1;
                    let g = at_cell(p).map(|v| v as usize);
                    let at = &mut known[index(g)];
                    *at = (add(at.0, c), at.1 + 1.0);
                } else {
                    state[i] = 2;
                    unseen.push((i as u32, p));
                }
            }
        }
    }
    // ---- what no picture sees: the painted colours spread through the grid
    // (each empty cell the mean of its painted neighbours, ring by ring,
    // then evened), read between its cells -- so a surface no picture
    // reached shades smoothly from what is painted round it
    let mut field: Vec<[f32; 3]> = known.iter().map(|(c, n)| if *n > 0.0 { scale(*c, 1.0 / n) } else { [0.0; 3] }).collect();
    let mut have: Vec<bool> = known.iter().map(|(_, n)| *n > 0.0).collect();
    let around = |c: [usize; 3]| {
        let mut out = Vec::with_capacity(26);
        for dz in -1i32..=1 {
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let q = [c[0] as i32 + dx, c[1] as i32 + dy, c[2] as i32 + dz];
                    if (dx, dy, dz) != (0, 0, 0) && (0..3).all(|k| q[k] >= 0 && (q[k] as usize) < dims[k]) {
                        out.push(index(q.map(|v| v as usize)));
                    }
                }
            }
        }
        out
    };
    for _ in 0..dims.iter().max().copied().unwrap_or(1) {
        let (was, had) = (field.clone(), have.clone());
        let mut grew = false;
        for z in 0..dims[2] {
            for y in 0..dims[1] {
                for x in 0..dims[0] {
                    let i = index([x, y, z]);
                    if had[i] {
                        continue;
                    }
                    let near: Vec<usize> = around([x, y, z]).into_iter().filter(|j| had[*j]).collect();
                    if !near.is_empty() {
                        field[i] = scale(near.iter().fold([0.0; 3], |a, j| add(a, was[*j])), 1.0 / near.len() as f32);
                        have[i] = true;
                        grew = true;
                    }
                }
            }
        }
        if !grew {
            break;
        }
    }
    // evened: the cells that were spread into, not the painted ones
    for _ in 0..3 {
        let was = field.clone();
        for z in 0..dims[2] {
            for y in 0..dims[1] {
                for x in 0..dims[0] {
                    let i = index([x, y, z]);
                    if known[i].1 > 0.0 {
                        continue;
                    }
                    let near = around([x, y, z]);
                    field[i] = scale(near.iter().fold(was[i], |a, j| add(a, was[*j])), 1.0 / (near.len() + 1) as f32);
                }
            }
        }
    }
    // (a texel one triangle's rim left unseen may since have been painted by the triangle it lies in)
    unseen.retain(|(i, _)| state[*i as usize] == 2);
    let filled = unseen.len();
    for (i, p) in &unseen {
        let g = at_cell(*p).map(|v| (v - 0.5).max(0.0));
        let base = g.map(|v| v as usize);
        let frac: [f32; 3] = std::array::from_fn(|k| g[k] - base[k] as f32);
        let mut c = [0.0f32; 3];
        for corner in 0..8usize {
            let q: [usize; 3] = std::array::from_fn(|k| (base[k] + (corner >> k & 1)).min(dims[k] - 1));
            let w: f32 = (0..3).map(|k| if corner >> k & 1 == 1 { frac[k] } else { 1.0 - frac[k] }).product();
            c = add(c, scale(field[index(q)], w));
        }
        sheet[*i as usize] = c.map(|v| v.round().clamp(0.0, 255.0) as u8);
    }
    save_png(&dir.join("color.png"), side, side, &sheet);
    let painted = state.iter().filter(|s| **s == 1).count();
    println!("{}: {side} px sheet, {painted} texels painted from the pictures, {} seen by none ({filled} filled from beside them)", dir.display(), unseen.len());
    for (view, n) in views.iter().zip(&used) {
        println!("  {:<6} {n} texels", view.name);
    }
}

fn len3(v: V3) -> f32 {
    dot(v, v).sqrt()
}
