//! `PUPPET_DEMO_SHOTS=<dir>`: software renders, fit and cost numbers.

use std::path::Path;
use std::time::Instant;

use gs_puppet::math::V3;
use gs_puppet::raster::Canvas;
use gs_puppet::{Camera, Look, Pose, Puppet};
use rig::View;

use crate::{iou_report, load_figures, overlay_prims, scene, Figure, BACKGROUND};

const SIZE: usize = 400;
const COLS: usize = 5;

fn write_png(path: &Path, w: usize, h: usize, rgba: &[u8]) {
    let file = std::fs::File::create(path).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut w| w.write_image_data(rgba)).expect("write png");
}

struct Shot<'a> {
    name: String,
    fig: &'a Figure,
    pose: &'a str,
    yaw: f32,
    look: Look<'a>,
}

fn render(shot: &Shot) -> (Vec<u8>, usize) {
    let pose = shot.fig.poses.get(shot.pose).unwrap_or_else(|| panic!("no pose {}", shot.pose));
    let cam = Camera::framing(shot.fig.puppet.height.max(5.0), [SIZE as f32, SIZE as f32]);
    let look = Look { yaw: shot.yaw, ..shot.look };
    let prims = scene(&shot.fig.puppet, pose, &look, &cam, true);
    let mut canvas = Canvas::new(SIZE, SIZE, 2, BACKGROUND);
    canvas.draw(&prims);
    (canvas.to_rgba8(), prims.len())
}

/// The rest puppet, orthographic, at 62% over the source silhouette.
fn overlay(fig: &Figure, view: View) -> Option<(Vec<u8>, usize, usize)> {
    let mask = fig.mask(view)?;
    let (w, h) = (mask.grid.w, mask.grid.h);
    let key = [1u8, 2, 3];
    let mut canvas = Canvas::new(w, h, 2, key);
    canvas.draw(&overlay_prims(fig, view)?);
    let puppet = canvas.to_rgba8();
    let mut out = Vec::with_capacity(w * h * 4);
    for i in 0..w * h {
        let under: [u8; 3] = if mask.inside[i] { [0x3f, 0x8f, 0xd8] } else { BACKGROUND };
        let px = &puppet[i * 4..i * 4 + 3];
        let covered = px != key;
        for k in 0..3 {
            let v = if covered { under[k] as f32 * 0.38 + px[k] as f32 * 0.62 } else { under[k] as f32 };
            out.push(v.round() as u8);
        }
        out.push(255);
    }
    Some((out, w, h))
}

fn blit(sheet: &mut [u8], sheet_w: usize, at: (usize, usize), src: &[u8], w: usize, h: usize) {
    for y in 0..h {
        let dst = ((at.1 + y) * sheet_w + at.0) * 4;
        sheet[dst..dst + w * 4].copy_from_slice(&src[y * w * 4..(y + 1) * w * 4]);
    }
}

/// Nearest-neighbour fit of an image into a SIZE x SIZE tile.
fn fit_tile(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut tile: Vec<u8> = (0..SIZE * SIZE).flat_map(|_| [BACKGROUND[0], BACKGROUND[1], BACKGROUND[2], 255]).collect();
    let s = (SIZE as f32 / w as f32).min(SIZE as f32 / h as f32);
    let (tw, th) = ((w as f32 * s) as usize, (h as f32 * s) as usize);
    let (ox, oy) = ((SIZE - tw) / 2, (SIZE - th) / 2);
    for y in 0..th {
        for x in 0..tw {
            let (sx, sy) = (((x as f32 + 0.5) / s) as usize, ((y as f32 + 0.5) / s) as usize);
            let (a, b) = ((sy.min(h - 1) * w + sx.min(w - 1)) * 4, ((oy + y) * SIZE + ox + x) * 4);
            tile[b..b + 4].copy_from_slice(&src[a..a + 4]);
        }
    }
    tile
}

/// Both families at the sizes they appear in game.
fn small(figures: &[Figure], dir: &Path) {
    const HEIGHTS: [f32; 4] = [40.0, 64.0, 96.0, 140.0];
    let rows: Vec<(&Figure, &str, f32, Look)> = figures
        .iter()
        .flat_map(|f| {
            let second = if f.rig.body_type == "biped" { "kneel" } else { "sit" };
            let wounds: &[(&str, u8)] = &[("chest", 2), ("leftArm", 1)];
            [
                (f, "stand", if f.rig.body_type == "biped" { 30.0 } else { 55.0 }, Look::default()),
                (f, second, 40.0, Look { body: [0x8a, 0x9a, 0x72], ..Look::default() }),
                (f, "dead", 60.0, Look { dead: true, ..Look::default() }),
                (f, "stand", -50.0, Look { wounds, boss_outline: true, ..Look::default() }),
            ]
        })
        .collect();
    let (cell_w, cell_h) = (300usize, 170usize);
    let (w, h) = (cell_w * HEIGHTS.len(), cell_h * rows.len());
    let mut canvas = Canvas::new(w, h, 3, BACKGROUND);
    for (r, (fig, pose, yaw, look)) in rows.iter().enumerate() {
        for (c, px) in HEIGHTS.iter().enumerate() {
            // `px` is the STANDING figure's height on screen.
            let ppf = px / fig.puppet.height;
            let cam = Camera {
                yaw_deg: 0.0,
                pitch_deg: 12.0,
                distance: 16.0,
                focal: ppf * 16.0,
                target: [0.0, 0.0, 0.0],
                center: [(c * cell_w + cell_w / 2) as f32, (r * cell_h + cell_h - 16) as f32],
            };
            let look = Look { yaw: *yaw, ..*look };
            let pose = fig.poses.get(pose).expect("pose");
            canvas.draw(&scene(&fig.puppet, pose, &look, &cam, false));
        }
    }
    write_png(&dir.join("small.png"), w, h, &canvas.to_rgba8());
}

pub fn run(dir: &Path) {
    std::fs::create_dir_all(dir).expect("shots dir");
    let figures = load_figures();
    let by = |label: &str| figures.iter().find(|f| f.label == label).unwrap_or_else(|| panic!("no {label}"));
    let (human, dog) = (by("humanoid"), by("canine"));
    let plain = Look::default();
    let human_wounds: &[(&str, u8)] = &[("leftArm", 1), ("chest", 2), ("rightLeg", 3)];
    let dog_wounds: &[(&str, u8)] = &[("rightArm", 3), ("back", 1)];
    let shot = |name: &str, fig, pose, yaw, look| Shot { name: name.to_owned(), fig, pose, yaw, look };
    let list = vec![
        shot("stand_front", human, "stand", 0.0, plain),
        shot("stand_34", human, "stand", 35.0, plain),
        shot("stand_side", human, "stand", 90.0, plain),
        shot("stand_back34", human, "stand", 145.0, plain),
        shot("kneel_34", human, "kneel", 35.0, plain),
        shot("kneel_side", human, "kneel", 90.0, plain),
        shot("sit_34", human, "sit", 35.0, plain),
        shot("prone", human, "prone", 35.0, plain),
        shot("supine", human, "supine", 35.0, plain),
        shot("fallen_side", human, "fallen_side", 35.0, plain),
        shot("stunned", human, "stunned", 35.0, plain),
        shot("arms_up", human, "arms_up", 0.0, plain),
        shot("wounded", human, "stand", 35.0, Look { wounds: human_wounds, ..plain }),
        shot("dead", human, "dead", 35.0, Look { dead: true, ..plain }),
        shot("boss", human, "stand", 35.0, Look { boss_outline: true, body: [0x7d, 0x93, 0x6a], ..plain }),
        shot("canine_stand_34", dog, "stand", 50.0, plain),
        shot("canine_stand_side", dog, "stand", 90.0, plain),
        shot("canine_stand_front", dog, "stand", 0.0, plain),
        shot("canine_sit", dog, "sit", 50.0, plain),
        shot("canine_lie", dog, "lie", 50.0, plain),
        shot("canine_fallen_side", dog, "fallen_side", 50.0, plain),
        shot("canine_wounded", dog, "stand", 50.0, Look { wounds: dog_wounds, body: [0x8c, 0x8a, 0x84], ..plain }),
        shot("canine_dead", dog, "dead", 50.0, Look { dead: true, ..plain }),
    ];

    let mut tiles: Vec<Vec<u8>> = Vec::new();
    for s in &list {
        let (rgba, prims) = render(s);
        write_png(&dir.join(format!("{}.png", s.name)), SIZE, SIZE, &rgba);
        eprintln!("puppet_demo: {}.png  {prims} prims", s.name);
        tiles.push(rgba);
    }
    for fig in &figures {
        let ious = iou_report(fig);
        for (view, iou) in &ious {
            let v = view.as_str();
            eprintln!("puppet_demo: {} rest-pose IoU vs {v} silhouette = {iou:.3}", fig.label);
            if let Some((rgba, w, h)) = overlay(fig, *view) {
                let name = if fig.label == "humanoid" { format!("overlay_{v}") } else { format!("{}_overlay_{v}", fig.label) };
                write_png(&dir.join(format!("{name}_iou{:.0}.png", iou * 1000.0)), w, h, &rgba);
                tiles.push(fit_tile(&rgba, w, h));
            }
        }
    }
    let rows = tiles.len().div_ceil(COLS);
    let mut sheet: Vec<u8> =
        (0..COLS * SIZE * rows * SIZE).flat_map(|_| [BACKGROUND[0], BACKGROUND[1], BACKGROUND[2], 255]).collect();
    for (i, t) in tiles.iter().enumerate() {
        blit(&mut sheet, COLS * SIZE, (i % COLS * SIZE, i / COLS * SIZE), t, SIZE, SIZE);
    }
    write_png(&dir.join("sheet.png"), COLS * SIZE, rows * SIZE, &sheet);
    small(&figures, dir);

    // ---- cost
    for fig in &figures {
        let n = 8;
        let started = Instant::now();
        for _ in 0..n {
            std::hint::black_box(
                Puppet::build_masks(&fig.rig, fig.mask(View::Front), fig.mask(View::Side)).expect("build"),
            );
        }
        let measure = started.elapsed() / n;
        let outlines = rig::Outlines::load(&crate::data_dir().join("outlines.json")).expect("outlines");
        let started = Instant::now();
        std::hint::black_box(Puppet::build(&fig.rig, &outlines).expect("build"));
        let full = started.elapsed();
        let baked = fig.puppet.to_baked_toml().expect("bake");
        let path = dir.join(Puppet::baked_file_name(&fig.label));
        std::fs::write(&path, &baked).expect("write baked");
        let again = Puppet::from_baked_toml(&baked).expect("unbake");
        assert_eq!(again, fig.puppet, "baked round trip");
        eprintln!(
            "puppet_demo: {}: {} segments, {} stations; Puppet::build {full:?} (of which measuring {measure:?}); baked {} bytes -> {}",
            fig.label,
            fig.puppet.segments.len(),
            fig.puppet.station_count(),
            baked.len(),
            path.display()
        );
        let started = Instant::now();
        for _ in 0..200 {
            std::hint::black_box(Puppet::from_baked_toml(&baked).expect("unbake"));
        }
        eprintln!("puppet_demo: {}: from_baked_toml {:?}", fig.label, started.elapsed() / 200);

        let pose: &Pose = fig.poses.poses.values().nth(1).expect("a pose");
        for px in [64.0f32, 140.0, 400.0] {
            let ppf = px / fig.puppet.height;
            let cam = Camera { yaw_deg: 0.0, pitch_deg: 12.0, distance: 16.0, focal: ppf * 16.0, target: [0.0; 3], center: [0.0; 2] };
            let look = Look { yaw: 35.0, ..Look::default() };
            let project = |p: V3| cam.project(p);
            let n = 400;
            let started = Instant::now();
            let mut prims = 0;
            for _ in 0..n {
                let posed = fig.puppet.pose(pose);
                prims = posed.draw_list(&project, &look).len();
            }
            eprintln!(
                "puppet_demo: {} at {px} px tall: {prims} prims, pose + draw list {:?} per frame",
                fig.label,
                started.elapsed() / n
            );
        }
    }
    for fig in &figures {
        eprintln!("puppet_demo: {} depth guesses", fig.label);
        for (name, g) in &fig.guesses {
            eprintln!("  {name:<14} {:+.3}  {:?}", g.value, g.source);
        }
        for s in &fig.puppet.segments {
            let body: Vec<String> = s.stations.iter().map(|st| format!("{:.2}/{:.2}", st.w, st.d)).collect();
            eprintln!("  seg {:<14} parent {:?} bare {} {:?}  {}", s.name, s.parent, s.bare, s.kind, body.join(" "));
        }
    }
}
