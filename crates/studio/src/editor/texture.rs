//! Painted textures, without the drawing: the paint-over templates, building
//! a pattern sheet from paintings, projection-first renders for a generator,
//! and reading a sheet back.
//!
//! A family paints in `<data>/textures/<family>/`; a creature with its own
//! skin in `<data>/textures/<family>/<creature>/` (its bestiary name token,
//! `gs_field::naming::name_token`); a creature wearing an outfit
//! (`<data>/outfits/outfits.toml`, `gs_puppet::outfit`) paints over its
//! outfit's templates, `<data>/textures/<family>/outfits/<outfit>/`:
//!
//! | File | Who writes it | What it is |
//! |---|---|---|
//! | `front_paint.png`, `side_paint.png`, `back_paint.png` | Export | the silhouettes to paint over (the back is the front mirrored), dressed in the outfit's |
//! | `front_depth.png`, `side_depth.png`, `back_depth.png` | Export | the rest body's depth in each template's frame, at [`DEPTH_SCALE`] times its size: a depth guide for an image generator |
//! | `marks.json` | Export | where the puppet's eyes are in each template (a painting puts the eyes there) |
//! | `pattern.png`, `patches.txt` | Export | the skin cut open (`gs_puppet::uv`): the sheet's layout |
//! | `front.png`, `side.png`, `back.png` | the artist / generator | paintings over the templates (any subset), in the family's or the creature's folder |
//! | `projected/<view>_render.png`, `<view>_unpainted.png` | Project | projection-first: what the paintings so far show from `side` / `back`, and what is left to paint |
//! | `sheet.png` | Build sheet, then the artist | the texture itself: the paintings projected onto the pattern, touched up by hand |
//!
//! Save installs the family's `sheet.png` as
//! `<pool>/global/puppets/<family>.texture.png` and Publish ships it beside
//! the puppet. The sheet's layout is rebuilt from the puppet wherever it is
//! used, so a sheet painted for an older rig (another size) is refused rather
//! than smeared.

use std::path::{Path, PathBuf};

use gs_puppet::{
    depth_guide, dressed_silhouette, prefill, project_view, puppet_eyes, silhouette_template, Kit, Outfit, Outfits, PaintView, Painting,
    Paintings, Puppet, Texture, UvLayout, MASK_PX, SHEET_SIDE,
};
use rig::{Outlines, View};
use rig_bake::Mask;

pub const SHEET: &str = "sheet.png";
const TEMPLATE_GREY: [u8; 3] = [205, 205, 205];
/// The depth guides and projection renders are this many times their
/// template's size: the size the paintings are made at.
pub const DEPTH_SCALE: usize = 4;
/// Where a family's outfits' templates live, under its texture folder.
const OUTFITS: &str = "outfits";
/// Where projection renders go, under a paintings folder.
const PROJECTED: &str = "projected";

/// `<data>/textures/<family>/`.
pub fn dir(data_root: &Path, family: &str) -> PathBuf {
    data_root.join("textures").join(family)
}

pub fn sheet_path(data_root: &Path, family: &str) -> PathBuf {
    dir(data_root, family).join(SHEET)
}

/// A creature's own paintings and skin: `<data>/textures/<family>/<creature>/`.
pub fn creature_dir(data_root: &Path, family: &str, creature: &str) -> PathBuf {
    dir(data_root, family).join(creature)
}

/// An outfit's templates: `<data>/textures/<family>/outfits/<key>/`.
pub fn outfit_dir(data_root: &Path, family: &str, key: &str) -> PathBuf {
    dir(data_root, family).join(OUTFITS).join(key)
}

fn masks(outlines: &Outlines, family: &str) -> (Option<Mask>, Option<Mask>) {
    let mask = |v: View| outlines.get(family, v).map(|e| rig_bake::rasterize(&e.outline(), MASK_PX));
    (mask(View::Front), mask(View::Side))
}

/// The clothing kit (`<data>/outfits/kit.toml`, else the one gs_puppet
/// ships) and who wears what (`<data>/outfits/outfits.toml`, else nobody).
pub fn outfits(data_root: &Path) -> Result<(Kit, Outfits), String> {
    let folder = data_root.join("outfits");
    let kit = match std::fs::read_to_string(folder.join("kit.toml")) {
        Ok(text) => Kit::from_toml(&text)?,
        Err(_) => Kit::shipped().clone(),
    };
    let outfits = match std::fs::read_to_string(folder.join("outfits.toml")) {
        Ok(text) => Outfits::from_toml(&text, &kit)?,
        Err(_) => Outfits::default(),
    };
    Ok((kit, outfits))
}

/// A name for an outfit, for its templates' folder: its pieces joined by
/// `+`, and `~<hash>` when a piece is tweaked away from the kit (another
/// shape, so other templates).
pub fn outfit_key(outfit: &Outfit, kit: &Kit) -> String {
    let names: Vec<&str> = outfit.pieces.iter().map(|(n, _)| n.as_str()).collect();
    let tweaked: Vec<String> = outfit
        .pieces
        .iter()
        .filter(|(n, p)| kit.piece(n) != Some(p))
        .map(|(n, p)| format!("{n}:{}:{}:{}", p.length, p.flare, p.size))
        .collect();
    let mut key = names.join("+");
    if !tweaked.is_empty() {
        // FNV-1a: stable across runs and builds.
        let hash = tweaked.join(";").bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3));
        key += &format!("~{:08x}", hash as u32);
    }
    key
}

/// What something is painted as: the family's puppet, wearing an outfit or
/// not, and the silhouettes its paintings are made over -- the art ones,
/// with the outfit's garments added.
pub struct Dressed {
    pub puppet: Puppet,
    pub front: Option<Mask>,
    pub side: Option<Mask>,
    /// The outfit's key, when there is one.
    pub outfit: Option<String>,
}

impl Dressed {
    /// The family bare, or wearing `outfit`.
    pub fn new(puppet: &Puppet, outlines: &Outlines, family: &str, outfit: Option<(&Outfit, &Kit)>) -> Dressed {
        let (front, side) = masks(outlines, family);
        let Some((outfit, kit)) = outfit.filter(|(o, _)| !o.pieces.is_empty()) else {
            return Dressed { puppet: puppet.clone(), front, side, outfit: None };
        };
        let worn = puppet.wearing(outfit);
        Dressed {
            front: front.map(|m| dressed_silhouette(&worn, &m, false)),
            side: side.map(|m| dressed_silhouette(&worn, &m, true)),
            puppet: worn,
            outfit: Some(outfit_key(outfit, kit)),
        }
    }

    /// A creature: the family's puppet in the creature's outfit, if it has
    /// one in `<data>/outfits/outfits.toml`.
    pub fn creature(data_root: &Path, puppet: &Puppet, outlines: &Outlines, family: &str, creature: &str) -> Result<Dressed, String> {
        let (kit, outfits) = outfits(data_root)?;
        Ok(Dressed::new(puppet, outlines, family, outfits.get(creature).map(|o| (o, &kit))))
    }

    /// Where this one's templates are.
    pub fn templates_dir(&self, data_root: &Path, family: &str) -> PathBuf {
        match &self.outfit {
            Some(key) => outfit_dir(data_root, family, key),
            None => dir(data_root, family),
        }
    }

    /// The paintings' grids: the silhouettes', so a painting over a dressed
    /// template projects exactly as one over the bare.
    fn paintings<'a>(&self, front: Option<Painting<'a>>, side: Option<Painting<'a>>, back: Option<Painting<'a>>) -> Paintings<'a> {
        Paintings {
            front_grid: self.front.as_ref().map(|m| m.grid),
            side_grid: self.side.as_ref().map(|m| m.grid),
            front,
            side,
            back,
        }
    }

    /// The silhouette `view` is painted over (the front one for the back).
    fn mask(&self, view: PaintView) -> Option<&Mask> {
        match view {
            PaintView::Side => self.side.as_ref(),
            PaintView::Front | PaintView::Back => self.front.as_ref(),
        }
    }
}

pub fn write_png(path: &Path, w: usize, h: usize, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut wr| wr.write_image_data(rgba))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// An 8-bit RGB or RGBA PNG as straight RGBA.
pub fn read_png(path: &Path) -> Result<(usize, usize, Vec<u8>), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| format!("{}: {e}", path.display()))?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|e| format!("{}: {e}", path.display()))?;
    let (w, h) = (info.width as usize, info.height as usize);
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..w * h * 4].to_vec(),
        png::ColorType::Rgb => buf[..w * h * 3].chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf[..w * h * 2].chunks_exact(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => buf[..w * h].iter().flat_map(|&g| [g, g, g, 255]).collect(),
        other => return Err(format!("{}: unsupported colour type {other:?}", path.display())),
    };
    Ok((w, h, rgba))
}

const VIEWS: [(&str, PaintView, bool); 3] = [("front", PaintView::Front, false), ("back", PaintView::Back, true), ("side", PaintView::Side, false)];

/// Write `d`'s paint-over templates, depth guides, eye marks and the
/// pattern sheet's layout into `out`.
fn write_templates(out: &Path, label: &str, d: &Dressed) -> Result<(), String> {
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut marks = Vec::new();
    for (name, view, mirror) in VIEWS {
        let Some(m) = d.mask(view) else { continue };
        let rgba = silhouette_template(m, mirror, TEMPLATE_GREY);
        write_png(&out.join(format!("{name}_paint.png")), m.grid.w, m.grid.h, &rgba)?;
        let depth = depth_guide(&d.puppet, m, view, DEPTH_SCALE);
        write_png(&out.join(format!("{name}_depth.png")), m.grid.w * DEPTH_SCALE, m.grid.h * DEPTH_SCALE, &depth)?;
        let eyes: Vec<serde_json::Value> = puppet_eyes(&d.puppet, &m.grid, view)
            .iter()
            .map(|e| serde_json::json!({ "x": round1(e[0]), "y": round1(e[1]), "r": round1(e[2]) }))
            .collect();
        marks.push((name, serde_json::json!({ "size": [m.grid.w, m.grid.h], "eyes": eyes })));
    }
    let marks: serde_json::Map<String, serde_json::Value> = marks.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
    let marks = serde_json::json!({
        "_": "Where the puppet's eyes are in each template, in its pixels (x, y, radius r); multiply by the painting's scale. Paint the eyes there: a painted skin wears no eye dots.",
        "views": marks,
    });
    std::fs::write(out.join("marks.json"), serde_json::to_string_pretty(&marks).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    let layout = UvLayout::fit(&d.puppet, SHEET_SIDE);
    let mut pattern = layout.template();
    for px in pattern.chunks_exact_mut(4) {
        if px[3] == 0 {
            px.copy_from_slice(&[255, 255, 255, 255]);
        }
    }
    write_png(&out.join("pattern.png"), layout.w, layout.h, &pattern)?;
    let mut legend = format!(
        "# {label}: {}x{} px, {:.2} px per foot. Paint sheet.png at exactly this size.\n# patch  x y w h  segment (part)\n",
        layout.w, layout.h, layout.px_per_ft
    );
    for (i, q) in layout.patches.iter().enumerate() {
        let s = &d.puppet.segments[q.segment];
        legend += &format!(
            "{i:>3}  {:.0} {:.0} {:.0} {:.0}  {} ({})\n",
            q.origin[0],
            q.origin[1],
            q.size[0],
            q.size[1],
            s.name,
            s.part.as_deref().unwrap_or(if s.kind.is_garment() { "garment" } else { "-" })
        );
    }
    std::fs::write(out.join("patches.txt"), legend).map_err(|e| e.to_string())
}

fn round1(v: f32) -> f64 {
    (v as f64 * 10.0).round() / 10.0
}

/// Write the family's paint-over templates, depth guides, eye marks and the
/// pattern sheet's layout into its texture folder. Returns the folder.
pub fn export_templates(data_root: &Path, family: &str, puppet: &Puppet, outlines: &Outlines) -> Result<PathBuf, String> {
    let out = dir(data_root, family);
    write_templates(&out, family, &Dressed::new(puppet, outlines, family, None))?;
    Ok(out)
}

fn as_painting(p: &Option<(usize, usize, Vec<u8>)>) -> Option<Painting<'_>> {
    p.as_ref().map(|(w, h, rgba)| Painting { rgba, w: *w, h: *h })
}

type Pixels = Option<(usize, usize, Vec<u8>)>;

/// `front.png`, `side.png` and `back.png` from `folder`, those there are.
fn read_paintings(folder: &Path) -> Result<[Pixels; 3], String> {
    let read = |name: &str| {
        let p = folder.join(name);
        p.is_file().then(|| read_png(&p)).transpose()
    };
    Ok([read("front.png")?, read("side.png")?, read("back.png")?])
}

/// Project whichever of `front.png`, `side.png`, `back.png` are in `folder`
/// onto `d`'s pattern and write `folder/sheet.png` (replacing it). Returns
/// the sheet's path and how many paintings went in.
pub fn build_sheet_in(folder: &Path, d: &Dressed) -> Result<(PathBuf, usize), String> {
    let [front_px, side_px, back_px] = read_paintings(folder)?;
    let count = [&front_px, &side_px, &back_px].iter().filter(|p| p.is_some()).count();
    if count == 0 {
        return Err(format!(
            "no paintings in {}: paint front.png, side.png or back.png over the exported *_paint.png templates",
            folder.display()
        ));
    }
    let paintings = d.paintings(as_painting(&front_px), as_painting(&side_px), as_painting(&back_px));
    let layout = UvLayout::fit(&d.puppet, SHEET_SIDE);
    let rgba = prefill(&d.puppet, &layout, &paintings);
    let path = folder.join(SHEET);
    write_png(&path, layout.w, layout.h, &rgba)?;
    Ok((path, count))
}

/// The family's sheet from the paintings in its folder.
pub fn build_sheet(data_root: &Path, family: &str, puppet: &Puppet, outlines: &Outlines) -> Result<(PathBuf, usize), String> {
    build_sheet_in(&dir(data_root, family), &Dressed::new(puppet, outlines, family, None))
}

/// The creatures of `family` that have a folder of their own paintings.
pub fn painted_creatures(data_root: &Path, family: &str) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir(data_root, family))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .filter(|name| name != OUTFITS)
        .filter(|name| ["front.png", "side.png", "back.png"].iter().any(|f| creature_dir(data_root, family, name).join(f).is_file()))
        .collect();
    out.sort();
    out
}

/// Build every creature's skin in `family` (see [`painted_creatures`]): each
/// on the family's puppet in the creature's outfit. Returns the ones built
/// and what went wrong with the rest.
pub fn build_every_skin(data_root: &Path, family: &str, puppet: &Puppet, outlines: &Outlines) -> Result<(Vec<String>, Vec<String>), String> {
    let (kit, outfits) = outfits(data_root)?;
    let (mut built, mut failed) = (Vec::new(), Vec::new());
    for creature in painted_creatures(data_root, family) {
        let d = Dressed::new(puppet, outlines, family, outfits.get(&creature).map(|o| (o, &kit)));
        match build_sheet_in(&creature_dir(data_root, family, &creature), &d) {
            Ok(_) => built.push(creature),
            Err(e) => failed.push(format!("{creature}: {e}")),
        }
    }
    Ok((built, failed))
}

/// Projection-first (`docs/projection-first-export-spec.md`): what the
/// paintings in `folder` show of `d` from `view` -- the front's for the side;
/// the front's and the side's for the back -- written to
/// `folder/projected/<view>_render.png` and `<view>_unpainted.png`, framed
/// exactly as that view's template at [`DEPTH_SCALE`] times its size.
pub fn export_projected(folder: &Path, d: &Dressed, view: PaintView, grazing: f32) -> Result<Vec<PathBuf>, String> {
    let name = match view {
        PaintView::Side => "side",
        PaintView::Back => "back",
        PaintView::Front => return Err("the front is painted first; project it to the side, then to the back".into()),
    };
    let [front_px, side_px, _] = read_paintings(folder)?;
    if front_px.is_none() {
        return Err(format!("no front.png in {}: paint the front first", folder.display()));
    }
    let side_px = if view == PaintView::Back { side_px } else { None };
    let paintings = d.paintings(as_painting(&front_px), as_painting(&side_px), None);
    let mask = d.mask(view).ok_or_else(|| format!("no {name} silhouette"))?;
    let (render, unpainted) = project_view(&d.puppet, mask, view, DEPTH_SCALE, &paintings, grazing);
    let out = folder.join(PROJECTED);
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let (w, h) = (mask.grid.w * DEPTH_SCALE, mask.grid.h * DEPTH_SCALE);
    let files = [out.join(format!("{name}_render.png")), out.join(format!("{name}_unpainted.png"))];
    write_png(&files[0], w, h, &render)?;
    write_png(&files[1], w, h, &unpainted)?;
    Ok(files.to_vec())
}

/// `<data>/generator.toml`: the families whose puppets are approved -- only
/// their depth guides go into the jobs (`depth_families = ["humanoid"]`).
fn depth_families(data_root: &Path) -> Result<Vec<String>, String> {
    #[derive(serde::Deserialize, Default)]
    struct Settings {
        #[serde(default)]
        depth_families: Vec<String>,
    }
    match std::fs::read_to_string(data_root.join("generator.toml")) {
        Ok(text) => toml::from_str::<Settings>(&text).map(|s| s.depth_families).map_err(|e| format!("generator.toml: {e}")),
        Err(_) => Ok(Vec::new()),
    }
}

/// `path` relative to `root`, with forward slashes (the jobs file's form).
fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

/// The texture generator's job list (`docs/texture-generator-plan.md` §2):
/// every bestiary creature a rig in `<data>/rigs/` draws (the doll cascade,
/// stand-ins included), with its family, its outfit, the templates, depth
/// guides and eye marks it is painted over, and the folder its paintings go
/// in. Every family's templates, and every outfit's its creatures wear, are
/// exported first so the paths exist. Writes `<data>/textures/jobs.json`;
/// returns it and how many jobs it holds.
pub fn export_jobs(data_root: &Path, outlines: &Outlines) -> Result<(PathBuf, usize), String> {
    let (kit, outfits) = outfits(data_root)?;
    let approved = depth_families(data_root)?;
    let mut puppets: std::collections::BTreeMap<String, Puppet> = std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(data_root.join("rigs")).map_err(|e| format!("rigs: {e}"))?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(family) = name.strip_suffix(".rig.toml") else { continue };
        let Ok(rig) = rig::Rig::load(&entry.path()) else { continue };
        if let Ok(p) = Puppet::build(&rig, outlines) {
            puppets.insert(family.to_owned(), p);
        }
    }
    let mut exported: std::collections::BTreeSet<(String, Option<String>)> = std::collections::BTreeSet::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut jobs = Vec::new();
    for e in gs_field::bestiary::shared().entries() {
        let key = gs_field::naming::name_token(&e.name);
        if key.is_empty() || !seen.insert(key.clone()) {
            continue;
        }
        let Some(doll) = gs_field::dolls::resolve_doll(e.family.as_deref(), e.creature_type.as_deref(), &|f| puppets.contains_key(f)) else {
            continue;
        };
        let family = doll.family;
        let puppet = &puppets[&family];
        let outfit = outfits.get(&key);
        let d = Dressed::new(puppet, outlines, &family, outfit.map(|o| (o, &kit)));
        let templates = d.templates_dir(data_root, &family);
        if exported.insert((family.clone(), d.outfit.clone())) {
            write_templates(&templates, &family, &d)?;
        }
        let mut views = serde_json::Map::new();
        for (name, view, _) in VIEWS {
            let Some(m) = d.mask(view) else { continue };
            let mut v = serde_json::json!({
                "template": relative(data_root, &templates.join(format!("{name}_paint.png"))),
                "size": [m.grid.w, m.grid.h],
            });
            if approved.contains(&family) {
                v["depth"] = relative(data_root, &templates.join(format!("{name}_depth.png"))).into();
            }
            views.insert(name.to_owned(), v);
        }
        jobs.push(serde_json::json!({
            "creature": e.name,
            "key": key,
            "family": family,
            "body_type": puppet.body_type,
            "height_ft": e.height,
            "description": e.description.clone().unwrap_or_default(),
            "brief": "",
            "outfit": outfit.map(|o| o.pieces.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>()).unwrap_or_default(),
            "outfit_key": d.outfit,
            "views": views,
            "marks": relative(data_root, &templates.join("marks.json")),
            "out_dir": relative(data_root, &creature_dir(data_root, &family, &key)),
        }));
    }
    let file = serde_json::json!({ "version": 1, "data_root": data_root.to_string_lossy().replace('\\', "/"), "jobs": jobs });
    let path = data_root.join("textures").join("jobs.json");
    std::fs::create_dir_all(path.parent().expect("a folder")).map_err(|e| e.to_string())?;
    std::fs::write(&path, serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((path, jobs.len()))
}

/// `--project <family>[/<creature>]:side|back` (the command line, for a
/// generator to call): build the family's puppet from `<data>/rigs/`, dress
/// the creature in its outfit, and write its projection renders into the
/// creature's folder (or the family's). Returns the files written.
pub fn project_cli(data_root: &Path, target: &str, grazing: Option<f32>) -> Result<Vec<PathBuf>, String> {
    let usage = || format!("--project wants <family>[/<creature>]:side|back, got '{target}'");
    let (who, view) = target.rsplit_once(':').ok_or_else(usage)?;
    let view = match view {
        "side" => PaintView::Side,
        "back" => PaintView::Back,
        _ => return Err(usage()),
    };
    let (family, creature) = match who.split_once('/') {
        Some((f, c)) => (f, Some(c)),
        None => (who, None),
    };
    let outlines = Outlines::load(&data_root.join("outlines.json")).map_err(|e| format!("outlines.json: {e}"))?;
    let rig = rig::Rig::load(&data_root.join("rigs").join(rig::Rig::file_name(family))).map_err(|e| format!("{family}: {e}"))?;
    let puppet = Puppet::build(&rig, &outlines).map_err(|e| format!("{family}: {e}"))?;
    let (d, folder) = match creature {
        Some(c) => (Dressed::creature(data_root, &puppet, &outlines, family, c)?, creature_dir(data_root, family, c)),
        None => (Dressed::new(&puppet, &outlines, family, None), dir(data_root, family)),
    };
    export_projected(&folder, &d, view, grazing.unwrap_or(gs_puppet::GRAZING))
}

/// The family's `sheet.png` as a texture for `puppet`. Ok(None) when there
/// is no sheet; Err when it is unreadable or laid out for another rig.
/// (The Rigs tab caches the pixels and calls [`sheet_texture`] itself.)
#[cfg(test)]
pub fn load_sheet(data_root: &Path, family: &str, puppet: &Puppet) -> Result<Option<Texture>, String> {
    let path = sheet_path(data_root, family);
    if !path.is_file() {
        return Ok(None);
    }
    let (w, h, rgba) = read_png(&path)?;
    sheet_texture(puppet, w, h, rgba).map(Some)
}

/// Decoded sheet pixels as a texture for `puppet`, if the sizes agree.
pub fn sheet_texture(puppet: &Puppet, w: usize, h: usize, rgba: Vec<u8>) -> Result<Texture, String> {
    let layout = UvLayout::fit(puppet, SHEET_SIDE);
    if (w, h) != (layout.w, layout.h) {
        return Err(format!(
            "sheet.png is {w}x{h} but this rig's pattern is {}x{}: the rig changed since it was painted -- export and build again",
            layout.w, layout.h
        ));
    }
    Ok(Texture { layout, rgba })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn humanoid() -> (Puppet, Outlines) {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let outlines = Outlines::load(&repo.join("outlines.json")).unwrap();
        let rig = rig::Rig::load(&repo.join("rigs").join(rig::Rig::file_name("humanoid"))).unwrap();
        (Puppet::build(&rig, &outlines).unwrap(), outlines)
    }

    fn scratch(tag: &str) -> PathBuf {
        let data = std::env::temp_dir().join(format!("gs_studio_texture_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        data
    }

    /// "Paint" a template solid red, as `<folder>/<view>.png`.
    fn paint_red(template: &Path, into: &Path) {
        let (w, h, mut px) = read_png(template).unwrap();
        for p in px.chunks_exact_mut(4).filter(|p| p[3] > 0) {
            p[..3].copy_from_slice(&[200, 30, 30]);
        }
        std::fs::create_dir_all(into.parent().unwrap()).unwrap();
        write_png(into, w, h, &px).unwrap();
    }

    /// Export, paint (a solid front), build, read back: the sheet is the
    /// pattern's size and carries the paint; a wrong-size sheet is refused.
    #[test]
    fn export_build_and_load_round_trip() {
        let (puppet, outlines) = humanoid();
        let data = scratch("round_trip");
        assert!(load_sheet(&data, "humanoid", &puppet).unwrap().is_none(), "no sheet yet");
        let folder = export_templates(&data, "humanoid", &puppet, &outlines).unwrap();
        for f in ["front_paint.png", "side_paint.png", "back_paint.png", "pattern.png", "patches.txt", "marks.json"] {
            assert!(folder.join(f).is_file(), "{f}");
        }
        // Each depth guide is its template at DEPTH_SCALE.
        for v in ["front", "side", "back"] {
            let (tw, th, _) = read_png(&folder.join(format!("{v}_paint.png"))).unwrap();
            let (dw, dh, _) = read_png(&folder.join(format!("{v}_depth.png"))).unwrap();
            assert_eq!((dw, dh), (tw * DEPTH_SCALE, th * DEPTH_SCALE), "{v}_depth.png");
        }
        // Two eyes face the front, none the back.
        let marks: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(folder.join("marks.json")).unwrap()).unwrap();
        assert_eq!(marks["views"]["front"]["eyes"].as_array().unwrap().len(), 2);
        assert_eq!(marks["views"]["back"]["eyes"].as_array().unwrap().len(), 0);
        assert!(build_sheet(&data, "humanoid", &puppet, &outlines).unwrap_err().contains("no paintings"));

        paint_red(&folder.join("front_paint.png"), &folder.join("front.png"));
        let (_, count) = build_sheet(&data, "humanoid", &puppet, &outlines).unwrap();
        assert_eq!(count, 1);
        let tex = load_sheet(&data, "humanoid", &puppet).unwrap().expect("a sheet");
        assert!(tex.rgba.chunks_exact(4).any(|p| p[3] > 0 && p[0] > 150 && p[1] < 80), "the paint is on the sheet");

        write_png(&sheet_path(&data, "humanoid"), 10, 10, &[0; 400]).unwrap();
        assert!(load_sheet(&data, "humanoid", &puppet).unwrap_err().contains("rig changed"));
        let _ = std::fs::remove_dir_all(&data);
    }

    /// A robed creature paints over its outfit's templates -- wider than the
    /// bare body's at the hem -- and its skin is built on the robed puppet,
    /// in its own folder.
    #[test]
    fn a_creature_in_an_outfit_paints_and_builds_on_its_own() {
        let (puppet, outlines) = humanoid();
        let data = scratch("outfit");
        std::fs::create_dir_all(data.join("outfits")).unwrap();
        std::fs::write(data.join("outfits/outfits.toml"), "[grim_gigas_skald]\nwear = [\"robe\", \"hood\"]\n").unwrap();
        let d = Dressed::creature(&data, &puppet, &outlines, "humanoid", "grim_gigas_skald").unwrap();
        assert_eq!(d.outfit.as_deref(), Some("robe+hood"));
        let folder = d.templates_dir(&data, "humanoid");
        write_templates(&folder, "humanoid", &d).unwrap();
        assert!(folder.ends_with("humanoid/outfits/robe+hood"), "{}", folder.display());
        let bare = export_templates(&data, "humanoid", &puppet, &outlines).unwrap();
        let count = |p: &Path| read_png(p).unwrap().2.chunks_exact(4).filter(|px| px[3] > 0).count();
        assert!(count(&folder.join("front_paint.png")) > count(&bare.join("front_paint.png")), "the robe widens the outline");

        let creature = creature_dir(&data, "humanoid", "grim_gigas_skald");
        paint_red(&folder.join("front_paint.png"), &creature.join("front.png"));
        assert_eq!(painted_creatures(&data, "humanoid"), ["grim_gigas_skald"]);
        let (built, failed) = build_every_skin(&data, "humanoid", &puppet, &outlines).unwrap();
        assert_eq!((built.len(), failed.len()), (1, 0), "{failed:?}");
        let (w, h, _) = read_png(&creature.join(SHEET)).unwrap();
        let robed = UvLayout::fit(&d.puppet, SHEET_SIDE);
        assert_eq!((w, h), (robed.w, robed.h), "the sheet is laid out for the robed puppet");
        let _ = std::fs::remove_dir_all(&data);
    }

    /// Projection-first: after the front, the side's render and mask are the
    /// side template's frame at DEPTH_SCALE; the back refuses without a front.
    #[test]
    fn projection_writes_the_side_render_and_its_mask() {
        let (puppet, outlines) = humanoid();
        let data = scratch("project");
        let folder = export_templates(&data, "humanoid", &puppet, &outlines).unwrap();
        let d = Dressed::new(&puppet, &outlines, "humanoid", None);
        assert!(export_projected(&folder, &d, PaintView::Back, gs_puppet::GRAZING).unwrap_err().contains("front"));
        paint_red(&folder.join("front_paint.png"), &folder.join("front.png"));
        let files = export_projected(&folder, &d, PaintView::Side, gs_puppet::GRAZING).unwrap();
        let (tw, th, _) = read_png(&folder.join("side_paint.png")).unwrap();
        for f in &files {
            let (w, h, _) = read_png(f).unwrap();
            assert_eq!((w, h), (tw * DEPTH_SCALE, th * DEPTH_SCALE), "{}", f.display());
        }
        let _ = std::fs::remove_dir_all(&data);
    }
}
