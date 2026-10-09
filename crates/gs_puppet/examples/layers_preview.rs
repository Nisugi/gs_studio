//! layers_preview: one family's puppet as LAYERED splats (`gs_puppet::layers`)
//! -- skin dots on the surface, flesh and bone under it where a wound has
//! opened it, a coating standing off it -- from several sides, drawn on the
//! CPU. One row per coating step, one tile per yaw.
//!
//!   cargo run --release -p gs_puppet --example layers_preview -- <out.png> <family> [pose] [yaw ...]
//!
//! Yaws default to 0 45 135 180. `LAYERS_WOUNDS=chest:2,leftArm:3` and
//! `LAYERS_SCARS=` (the same) hurt it; `LAYERS_COAT=ice:0,0.3,0.6,1` coats
//! it (a row per progress; `stone` is the other material) and
//! `LAYERS_COAT_FT=<ft>` sets the coat's thickness, `LAYERS_COAT_OVER=1`
//! lays it over the wounds (it came after them; else they cut through it);
//! `LAYERS_CAUSE=fire` (every wound's) or `chest:fire,abdomen:puncture`
//! says what made them, by the game's damage types (slash, puncture, crush,
//! fire, cold, lightning, impact, jab, punch, kick, grapple, unbalance,
//! acid, steam, plasma, disintegrate, disruption, non_corporeal, vacuum);
//! `LAYERS_CRIT=<1..9>` is the critical rank of every wound;
//! `LAYERS_FX=bleeding,rooted,...` puts those statuses on it (webbed,
//! frozen, poisoned, diseased, bleeding, rooted, bound, hidden, invisible,
//! possessed);
//! `LAYERS_FRESH=chest:0.05,0.3,1.5` shows that part just hit, a row per
//! age in seconds (in place of the coating's rows);
//! `LAYERS_CLOUD=1` wears the family's fitted cloud
//! (`<data>/clouds/<family>.ply`; or a path to one) as the skin, in place
//! of the made dots; `LAYERS_CLOUD_FLIP=yz` turns a cloud over that was
//! made in other axes (each letter negates that axis: `yz` is a half turn
//! about x) and `LAYERS_CLOUD_YAW=<deg>` turns it about the vertical, so it
//! stands upright facing +z as the puppet does;
//! `LAYERS_SPACING=<ft>` the dot spacing (default 0.015);
//! `LAYERS_SHEET=<sheet.png>` wears that sheet (default the family's
//! `data/textures/<family>/sheet.png`, `none` for the body colour);
//! `LAYERS_TILE=<px>` the tile size (default 420); `LAYERS_ZOOM=<x>`
//! magnifies and `LAYERS_LOOK_Y=<ft>` aims at that height;
//! `LAYERS_DATA=<dir>` another data root.

use std::path::Path;

use gs_puppet::layers::{Cause, Coating, Fresh, Layer, LayerCloud, LayerLook, Material, COAT_FT};
use gs_puppet::raster::Canvas;
use gs_puppet::splat;
use gs_puppet::{ground_grid, Camera, Look, Pose, PoseSet, Puppet, Texture, UvLayout, MASK_PX, SHEET_SIDE};
use rig::{Outlines, Rig, View};

const BACKGROUND: [u8; 3] = [0x2b, 0x30, 0x36];
const GRID: [u8; 4] = [0x4a, 0x52, 0x5c, 255];

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

/// `chest:2,leftArm:3`
fn ranks(var: &str) -> Vec<(String, u8)> {
    std::env::var(var)
        .unwrap_or_default()
        .split(',')
        .filter_map(|w| w.trim().split_once(':'))
        .filter_map(|(part, rank)| Some((part.to_owned(), rank.parse().ok()?)))
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(out), Some(family)) = (args.first(), args.get(1)) else {
        eprintln!("usage: layers_preview <out.png> <family> [pose] [yaw ...]");
        std::process::exit(2);
    };
    let pose_name = args.get(2).map(String::as_str).unwrap_or("stand");
    let mut yaws: Vec<f32> = args.iter().skip(3).filter_map(|a| a.parse().ok()).collect();
    if yaws.is_empty() {
        yaws = vec![0.0, 45.0, 135.0, 180.0];
    }
    let env = |name: &str| std::env::var(name).ok();
    let tile: usize = env("LAYERS_TILE").and_then(|v| v.parse().ok()).unwrap_or(420);
    let spacing: f32 = env("LAYERS_SPACING").and_then(|v| v.parse().ok()).unwrap_or(0.015);

    let data = std::env::var_os("LAYERS_DATA").map(std::path::PathBuf::from).unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data"));
    let outlines = Outlines::load(&data.join("outlines.json")).expect("outlines.json");
    let rig = Rig::load(&data.join("rigs").join(Rig::file_name(family))).expect("rig loads");
    let mask = |v: View| outlines.get(family, v).map(|e| rig_bake::rasterize(&e.outline(), MASK_PX));
    let puppet = Puppet::build_masks(&rig, mask(View::Front).as_ref(), mask(View::Side).as_ref()).expect("puppet builds");
    let set = PoseSet::load(&data.join("poses"), &rig.body_type).expect("pose set").for_family(family).into_owned();
    let pose = if pose_name == "rest" { Pose::default() } else { set.clamp(set.get(pose_name).expect("no such pose")) };
    let posed = puppet.pose(&pose);

    let sheet_path = std::env::var_os("LAYERS_SHEET").map(std::path::PathBuf::from).unwrap_or_else(|| data.join("textures").join(family).join("sheet.png"));
    let layout = UvLayout::fit(&puppet, SHEET_SIDE);
    let texture = (sheet_path.as_os_str() != "none").then(|| read_png(&sheet_path)).flatten().and_then(|(w, h, rgba)| {
        if (w, h) == (layout.w, layout.h) {
            Some(Texture { layout: layout.clone(), rgba, normal: None })
        } else {
            eprintln!("{}: {w}x{h} does not fit the pattern ({}x{}), body colour instead", sheet_path.display(), layout.w, layout.h);
            None
        }
    });

    let (wounds, scars) = (ranks("LAYERS_WOUNDS"), ranks("LAYERS_SCARS"));
    let wounds: Vec<(&str, u8)> = wounds.iter().map(|(p, r)| (p.as_str(), *r)).collect();
    let scars: Vec<(&str, u8)> = scars.iter().map(|(p, r)| (p.as_str(), *r)).collect();
    // A row per coating step; one bare row without any.
    let thickness: f32 = env("LAYERS_COAT_FT").and_then(|v| v.parse().ok()).unwrap_or(COAT_FT);
    let coats: Vec<Option<Coating>> = match env("LAYERS_COAT") {
        Some(text) => {
            let (name, steps) = text.split_once(':').unwrap_or((text.as_str(), "1"));
            let material = Material::from_name(name).expect("LAYERS_COAT: ice or stone");
            steps.split(',').filter_map(|s| s.trim().parse().ok()).map(|progress| Some(Coating { material, progress, thickness_ft: thickness, over_wounds: env("LAYERS_COAT_OVER").is_some() })).collect()
        }
        None => vec![None],
    };

    let every = env("LAYERS_CAUSE").filter(|v| !v.contains(':')).and_then(|v| Cause::from_name(&v));
    let listed: Vec<(String, Cause)> = env("LAYERS_CAUSE")
        .unwrap_or_default()
        .split(',')
        .filter_map(|w| w.trim().split_once(':'))
        .filter_map(|(part, cause)| Some((part.to_owned(), Cause::from_name(cause)?)))
        .collect();
    let causes: Vec<(&str, Cause)> = match every {
        Some(cause) => wounds.iter().map(|(part, _)| (*part, cause)).collect(),
        None => listed.iter().map(|(p, c)| (p.as_str(), *c)).collect(),
    };
    let crit: u8 = env("LAYERS_CRIT").and_then(|v| v.parse().ok()).unwrap_or(0);
    let fx = env("LAYERS_FX").unwrap_or_default();
    let effects = gs_puppet::Effects {
        webbed: fx.contains("web"),
        frozen: fx.contains("froz"),
        poisoned: fx.contains("poison"),
        diseased: fx.contains("disease"),
        bleeding: fx.contains("bleed"),
        rooted: fx.contains("root"),
        bound: fx.contains("bound"),
        hidden: fx.contains("hidden"),
        invisible: fx.contains("invisible"),
        possessed: fx.contains("possess"),
        cutthroat: fx.contains("cutthroat"),
        blinded: fx.contains("blind"),
        stunned: fx.contains("stun"),
        dazed: fx.contains("daze"),
        frenzied: fx.contains("frenz"),
        overexerted: fx.contains("overex"),
        calmed: fx.contains("calm"),
        vulnerable: fx.contains("vulner"),
        silenced: fx.contains("silence"),
        immobile: fx.contains("immobile"),
    };
    let crits: Vec<(&str, u8)> = wounds.iter().map(|(part, _)| (*part, crit)).collect();
    // a part just hit: a row per age, the coating's first step on each
    let fresh = env("LAYERS_FRESH").and_then(|v| v.split_once(':').map(|(part, ages)| (part.to_owned(), ages.split(',').filter_map(|a| a.trim().parse::<f32>().ok()).collect::<Vec<_>>())));
    let rows: Vec<(Option<Coating>, Option<f32>)> = match &fresh {
        Some((_, ages)) => ages.iter().map(|age| (coats[0], Some(*age))).collect(),
        None => coats.iter().map(|c| (*c, None)).collect(),
    };

    let cloud = LayerCloud::build(&puppet, &splat::SplatParams { spacing, ..Default::default() });
    println!("{family} {pose_name}: {} anchors ({} px tiles)", cloud.len(), tile);
    // a fitted cloud worn as the skin
    let fitted = env("LAYERS_CLOUD").map(|v| {
        let path = if v == "1" { splat::cloud_path(&data, family) } else { std::path::PathBuf::from(v) };
        let mut fitted = splat::load_cloud(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        fitted.retain(|g| g.alpha >= 0.02);
        let flip = env("LAYERS_CLOUD_FLIP").unwrap_or_default();
        let turn = gs_puppet::math::rot_y(env("LAYERS_CLOUD_YAW").and_then(|v| v.parse().ok()).unwrap_or(0.0));
        let placed = |mut v: [f32; 3]| {
            for (k, axis) in ['x', 'y', 'z'].into_iter().enumerate() {
                if flip.contains(axis) {
                    v[k] = -v[k];
                }
            }
            gs_puppet::math::mul_mv(&turn, v)
        };
        for g in &mut fitted {
            g.c = placed(g.c);
            g.axes = g.axes.map(placed);
        }
        splat::fit_to_height(&mut fitted, puppet.height);
        let started = std::time::Instant::now();
        let skin = cloud.fit(&puppet, fitted);
        println!("{}: {} gaussians bound to anchors in {:.0} ms", path.display(), skin.len(), started.elapsed().as_secs_f32() * 1000.0);
        skin
    });

    // Frame as the turnaround does.
    let [lo, hi] = posed.bounds();
    let reach = (lo[0].abs().max(hi[0].abs())).hypot(lo[2].abs().max(hi[2].abs()));
    let size = (hi[1] - lo[1]).max(reach * 1.4).max(0.5) * 1.1;
    let (w, h) = (tile * yaws.len(), tile * rows.len());
    let mut canvas = Canvas::new(w, h, 1, BACKGROUND);
    let cam_at = |col: usize, row: usize| {
        let mut cam = Camera::framing(size, [tile as f32, tile as f32]);
        cam.target = [0.0, (hi[1] + lo[1]) * 0.5, 0.0];
        if let Some(z) = env("LAYERS_ZOOM").and_then(|v| v.parse::<f32>().ok()) {
            cam.focal *= z.max(0.1);
        }
        if let Some(y) = env("LAYERS_LOOK_Y").and_then(|v| v.parse::<f32>().ok()) {
            cam.target[1] = y;
        }
        cam.center = [tile as f32 * (col as f32 + 0.5), tile as f32 * (row as f32 + 0.5)];
        cam
    };
    for row in 0..rows.len() {
        for col in 0..yaws.len() {
            let cam = cam_at(col, row);
            canvas.draw(&ground_grid(&|p| cam.project(p), 5, GRID, 1.0));
        }
    }
    let mut rgba = canvas.to_rgba8();
    for (row, (coat, age)) in rows.iter().enumerate() {
        let hit = fresh.as_ref().zip(*age).map(|((part, _), age)| Fresh { part: part.as_str(), age });
        let look = LayerLook { look: Look { wounds: &wounds, scars: &scars, ..Look::default() }, texture: texture.as_ref(), coat: *coat, causes: &causes, crits: &crits, fresh: hit, fitted: fitted.as_ref(), effects, ..LayerLook::default() };
        let started = std::time::Instant::now();
        let dots = cloud.dots(&posed, &look);
        let of = |layer: Layer| dots.iter().filter(|d| d.layer == layer).count();
        println!(
            "  coat {:?} hit {:?}: {} skin, {} fat, {} flesh, {} bone, {} coat, {} loose, {} status, {} blood dots ({:.1} ms)",
            coat.map(|c| c.progress),
            age,
            of(Layer::Skin),
            of(Layer::Fat),
            of(Layer::Flesh),
            of(Layer::Bone),
            of(Layer::Coat),
            of(Layer::Loose),
            of(Layer::Status),
            dots.iter().filter(|d| matches!(d.layer, Layer::Blood { .. })).count(),
            started.elapsed().as_secs_f32() * 1000.0
        );
        let gaussians: Vec<splat::Gaussian> = dots.into_iter().map(|d| d.g).collect();
        for (col, yaw) in yaws.iter().enumerate() {
            let cam = cam_at(col, row);
            let image = splat::render(&gaussians, &|p| cam.project(p), *yaw);
            // keep each figure to its own tile
            let (tx0, ty0) = ((col * tile) as i64, (row * tile) as i64);
            for y in 0..image.h {
                for x in 0..image.w {
                    let (tx, ty) = (image.origin[0] as i64 + x as i64, image.origin[1] as i64 + y as i64);
                    if tx < tx0 || ty < ty0 || tx >= tx0 + tile as i64 || ty >= ty0 + tile as i64 {
                        continue;
                    }
                    let s = &image.rgba[(y * image.w + x) * 4..][..4];
                    let a = s[3] as u32;
                    let d = &mut rgba[(ty as usize * w + tx as usize) * 4..][..4];
                    for c in 0..3 {
                        d[c] = ((s[c] as u32 * a + d[c] as u32 * (255 - a)) / 255) as u8;
                    }
                }
            }
        }
    }
    let file = std::fs::File::create(out).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut wr| wr.write_image_data(&rgba)).expect("write png");
    println!("-> {out}");
}
