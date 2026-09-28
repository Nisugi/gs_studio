//! normal_guide: a family's normal guides ([`gs_puppet::normal_guide`]),
//! the rest body's facing in each paint-over template's frame, beside the
//! depth guides gs-studio exports -- what a painting's own light can be read
//! against.
//!
//!   cargo run --release -p gs_puppet --example normal_guide -- <out_dir> <family> [scale]
//!
//! Writes `<view>_normal_guide.png` for front, back and side at `scale`
//! (default 4, the depth guides' `DEPTH_SCALE`) times the template.

use std::path::Path;

use gs_puppet::{normal_guide, PaintView, Puppet, MASK_PX};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(out), Some(family)) = (args.first(), args.get(1)) else {
        eprintln!("usage: normal_guide <out_dir> <family> [scale]");
        std::process::exit(2);
    };
    let scale: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(4);
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let rig = rig::Rig::load(&data.join("rigs").join(rig::Rig::file_name(family))).expect("rig");
    let outlines = rig::Outlines::load(&data.join("outlines.json")).expect("outlines");
    let puppet = Puppet::build(&rig, &outlines).expect("puppet builds");
    std::fs::create_dir_all(out).expect("out dir");
    for (name, view, v) in [("front", PaintView::Front, rig::View::Front), ("back", PaintView::Back, rig::View::Front), ("side", PaintView::Side, rig::View::Side)] {
        let Some(entry) = outlines.get(family, v) else { continue };
        let mask = rig_bake::rasterize(&entry.outline(), MASK_PX);
        let rgba = normal_guide(&puppet, &mask, view, scale);
        let (w, h) = (mask.grid.w * scale, mask.grid.h * scale);
        let path = Path::new(out).join(format!("{name}_normal_guide.png"));
        let file = std::fs::File::create(&path).expect("create png");
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().and_then(|mut wr| wr.write_image_data(&rgba)).expect("write png");
        println!("wrote {} ({w}x{h})", path.display());
    }
}
