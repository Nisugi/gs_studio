//! mesh_pose: a modelled mesh bound to a family's rig and posed by it
//! (`mesh::BoundMesh`), CPU-rendered -- the test of a mesh as a puppet's
//! body.
//!
//!   cargo run --release -p gs_puppet --example mesh_pose -- <out.png> <mesh dir> <family> [pose[,pose...]] [yaw ...]
//!
//! `<mesh dir>` holds `body.obj` (y up, facing +z, standing as the rig was
//! drawn) and `color.png`, its painted sheet. A row per pose (`rest`: as
//! bound); per yaw (default 0 45 90 180) the puppet's own skin, then the
//! mesh. `MESH_TILE` sets a tile's size, `MESH_ONLY=1` leaves the puppet's
//! skin out, `MESH_ZOOM` / `MESH_LOOK_Y` (0 feet .. 1 head) look closer.
//!
//! It writes the rig's `bones.txt` into the mesh dir; `tools/mesh_weights.py`
//! (Blender) makes `weights.txt` there from it, which is then used in place
//! of the nearest-segment binding (`MESH_NEAREST=1`: not). `MESH_MEASURE=1`
//! draws the puppet's skin with its rings measured from the mesh
//! (`BoundMesh::measured`) and writes that puppet beside the mesh as
//! `puppet.toml`. `MESH_BLEND=dual`
//! moves shared vertices by dual quaternions (`mesh::Blend`).

use std::path::Path;

use gs_puppet::math::{len, sub};
use gs_puppet::mesh::{self, BoundMesh, Mesh, Paint};
use gs_puppet::raster::Canvas;
use gs_puppet::{ground_grid, Camera, Look, Pose, PoseSet, Puppet, MASK_PX};
use rig::{Outlines, Rig, View};

const BACKGROUND: [u8; 3] = [0x2b, 0x30, 0x36];
const GRID: [u8; 4] = [0x4a, 0x52, 0x5c, 255];

fn load_png(path: &Path) -> Option<Paint> {
    let mut reader = png::Decoder::new(std::fs::File::open(path).ok()?).read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let rgba = match (info.color_type, info.bit_depth) {
        (png::ColorType::Rgba, png::BitDepth::Eight) => buf[..w * h * 4].to_vec(),
        (png::ColorType::Rgb, png::BitDepth::Eight) => buf[..w * h * 3].chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        _ => return None,
    };
    Some(Paint { w, h, rgba })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(out), Some(dir), Some(family)) = (args.first(), args.get(1), args.get(2)) else {
        eprintln!("usage: mesh_pose <out.png> <mesh dir> <family> [pose[,pose...]] [yaw ...]");
        std::process::exit(2);
    };
    let pose_names: Vec<&str> = args.get(3).map(String::as_str).unwrap_or("stand").split(',').collect();
    let mut yaws: Vec<f32> = args.iter().skip(4).filter_map(|a| a.parse().ok()).collect();
    if yaws.is_empty() {
        yaws = vec![0.0, 45.0, 90.0, 180.0];
    }
    let env = |k: &str| std::env::var(k).ok();
    let tile: usize = env("MESH_TILE").and_then(|v| v.parse().ok()).unwrap_or(420);
    let only = env("MESH_ONLY").is_some();
    let zoom: f32 = env("MESH_ZOOM").and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let look_y: f32 = env("MESH_LOOK_Y").and_then(|v| v.parse().ok()).unwrap_or(0.5);

    let data = env("SPLAT_DATA").map(std::path::PathBuf::from).unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data"));
    let outlines = Outlines::load(&data.join("outlines.json")).expect("outlines.json");
    let rig = Rig::load(&data.join("rigs").join(Rig::file_name(family))).expect("rig");
    let mask = |v: View| outlines.get(family, v).map(|e| rig_bake::rasterize(&e.outline(), MASK_PX));
    let puppet = Puppet::build_masks(&rig, mask(View::Front).as_ref(), mask(View::Side).as_ref()).expect("puppet builds");
    let set = PoseSet::load(&data.join("poses"), &rig.body_type).expect("pose set").for_family(family).into_owned();

    let dir = Path::new(dir);
    let mut body = Mesh::load_obj(&dir.join("body.obj")).unwrap_or_else(|e| {
        eprintln!("{}: {e}", dir.join("body.obj").display());
        std::process::exit(1);
    });
    let _ = body.fit_to_height(puppet.height);
    let paint = load_png(&dir.join("color.png"));
    let t0 = std::time::Instant::now();
    std::fs::write(dir.join("bones.txt"), mesh::bones_text(&puppet)).expect("write bones.txt");
    let file_verts = body.source.iter().max().map_or(0, |m| *m as usize + 1);
    let given = match mesh::load_weights(&dir.join("weights.txt"), &puppet) {
        _ if env("MESH_NEAREST").is_some() => Vec::new(),
        Ok(w) if w.len() == file_verts => {
            println!("weights.txt: {} of {} vertices have shares", w.iter().flatten().count(), w.len());
            w
        }
        Ok(w) => {
            eprintln!("weights.txt is for {} vertices, body.obj has {file_verts}: not used", w.len());
            Vec::new()
        }
        Err(_) => Vec::new(),
    };
    let bound = BoundMesh::with_weights(&puppet, body, &given);
    println!("{}: {} vertices, {} triangles, bound in {:.0} ms", dir.display(), bound.mesh.pos.len(), bound.mesh.tris.len(), t0.elapsed().as_secs_f32() * 1000.0);

    // the puppet's rings from the mesh: what its skin is then drawn with
    let measured = env("MESH_MEASURE").map(|_| bound.measured(&puppet));
    if let Some(m) = &measured {
        std::fs::write(dir.join("puppet.toml"), m.to_baked_toml().expect("a baked puppet")).expect("write puppet.toml");
        println!("puppet.toml: the puppet with its rings measured from the mesh");
    }
    let skin_puppet = measured.as_ref().unwrap_or(&puppet);
    let per = if only { 1 } else { 2 };
    let (w, h) = (tile * yaws.len() * per, tile * pose_names.len());
    let mut canvas = Canvas::new(w, h, 1, BACKGROUND);
    let size = puppet.height * 1.25 / zoom;
    let mut cams = Vec::new();
    for row in 0..pose_names.len() {
        for k in 0..yaws.len() * per {
            let mut cam = Camera::framing(size, [tile as f32, tile as f32]);
            cam.target = [0.0, puppet.height * look_y, 0.0];
            cam.center = [tile as f32 * (k as f32 + 0.5), tile as f32 * (row as f32 + 0.5)];
            canvas.draw(&ground_grid(&|p| cam.project(p), 5, GRID, 1.0));
            cams.push(cam);
        }
    }
    let mut rgba = canvas.to_rgba8();
    let paste = |rgba: &mut Vec<u8>, image: &gs_puppet::skin::Skin, clip: [usize; 4]| {
        for y in 0..image.h {
            for x in 0..image.w {
                let (tx, ty) = (image.origin[0] as i64 + x as i64, image.origin[1] as i64 + y as i64);
                if tx < clip[0] as i64 || ty < clip[1] as i64 || tx >= clip[2] as i64 || ty >= clip[3] as i64 {
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
    };
    for (row, name) in pose_names.iter().enumerate() {
        let (pos, nrm, posed) = if *name == "rest" {
            (bound.mesh.pos.clone(), bound.mesh.nrm.clone(), skin_puppet.pose_as_drawn(&Pose::default()))
        } else {
            let pose = set.clamp(set.get(name).unwrap_or_else(|| {
                eprintln!("no pose {name}");
                std::process::exit(1);
            }));
            let posed = puppet.pose(&pose);
            let blend = if env("MESH_BLEND").as_deref() == Some("dual") { mesh::Blend::DualQuat } else { mesh::Blend::Linear };
            let (pos, nrm) = bound.posed_by(&posed, blend);
            (pos, nrm, skin_puppet.pose(&pose))
        };
        let _ = len(sub(posed.bounds()[1], posed.bounds()[0]));
        let t1 = std::time::Instant::now();
        for (t, yaw) in yaws.iter().enumerate() {
            let k = row * yaws.len() * per + t * per;
            let clip = |k: usize| [(k % (yaws.len() * per)) * tile, row * tile, (k % (yaws.len() * per) + 1) * tile, (row + 1) * tile];
            if !only {
                let cam = &cams[k];
                let skin = posed.skin(&|p| cam.project(p), &Look { yaw: *yaw, outline_px: 1.5, ..Look::default() });
                paste(&mut rgba, &skin, clip(k));
            }
            let cam = &cams[k + per - 1];
            // MESH_FLAT=1: the sheet's colours as they are, with none of this renderer's shading
            let flat = vec![[0.0f32; 3]; nrm.len()];
            let image = mesh::render(&bound.mesh, &pos, if env("MESH_FLAT").is_some() { &flat } else { &nrm }, paint.as_ref(), &|p| cam.project(p), *yaw);
            paste(&mut rgba, &image, clip(k + per - 1));
        }
        println!("{name}: {} views in {:.0} ms", yaws.len(), t1.elapsed().as_secs_f32() * 1000.0);
    }
    let file = std::fs::File::create(out).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut wr| wr.write_image_data(&rgba)).expect("write png");
    println!("-> {out}");
}
