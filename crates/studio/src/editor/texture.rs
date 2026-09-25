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
//! | `front_normal.png`, `side_normal.png`, `back_normal.png` | the generator | tangent-space normal maps of the paintings (any subset), the same framing |
//! | `projected/<view>_render.png`, `<view>_unpainted.png` | Project | projection-first: what the paintings so far show from the next view in the family's order ([`projection_order`]), and what is left to paint |
//! | `sheet.png` | Build sheet, then the artist | the texture itself: the paintings projected onto the pattern, touched up by hand |
//! | `normal.png` | Build sheet | the normal sheet: the normal maps projected onto the pattern (`gs_puppet::prefill_normals`); only with a map to project |
//!
//! Save installs the family's `sheet.png` as
//! `<pool>/global/puppets/<family>.texture.png` (and `normal.png` as
//! `<family>.normal.png`), and each creature's as
//! `<pool>/global/puppets/skins/<family>/<creature>.png` (`.normal.png`
//! beside it) ([`ship_skins`]), and Publish ships them beside the puppet. The sheet's layout is rebuilt
//! from the puppet wherever it is used, so a sheet painted for an older rig
//! (another size) is refused rather than smeared.

use std::path::{Path, PathBuf};

use gs_puppet::{
    depth_guide, dressed_silhouette, prefill, prefill_normals, project_view, puppet_eyes, silhouette_template, Kit, Outfit, Outfits, PaintView, Painting,
    Paintings, Puppet, Texture, UvLayout, MASK_PX, SHEET_SIDE,
};
use rig::{Outlines, View};
use rig_bake::Mask;

pub const SHEET: &str = "sheet.png";
/// The normal sheet beside it.
pub const NORMAL_SHEET: &str = "normal.png";
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

pub fn normal_sheet_path(data_root: &Path, family: &str) -> PathBuf {
    dir(data_root, family).join(NORMAL_SHEET)
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

/// `front<suffix>.png`, `side<suffix>.png` and `back<suffix>.png` from
/// `folder`, those there are.
fn read_paintings(folder: &Path, suffix: &str) -> Result<[Pixels; 3], String> {
    let read = |view: &str| {
        let p = folder.join(format!("{view}{suffix}.png"));
        p.is_file().then(|| read_png(&p)).transpose()
    };
    Ok([read("front")?, read("side")?, read("back")?])
}

/// Project whichever of `front.png`, `side.png`, `back.png` are in `folder`
/// onto `d`'s pattern and write `folder/sheet.png` (replacing it), and
/// whichever of `front_normal.png`, `side_normal.png`, `back_normal.png`
/// are there as `folder/normal.png` (removed when there are none). Returns
/// the sheet's path and how many paintings went in.
pub fn build_sheet_in(folder: &Path, d: &Dressed) -> Result<(PathBuf, usize), String> {
    let [front_px, side_px, back_px] = read_paintings(folder, "")?;
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
    let [front_n, side_n, back_n] = read_paintings(folder, "_normal")?;
    let normal_path = folder.join(NORMAL_SHEET);
    if [&front_n, &side_n, &back_n].iter().any(|p| p.is_some()) {
        let maps = d.paintings(as_painting(&front_n), as_painting(&side_n), as_painting(&back_n));
        let normals = prefill_normals(&d.puppet, &layout, &maps);
        write_png(&normal_path, layout.w, layout.h, &normals)?;
    } else if normal_path.is_file() {
        std::fs::remove_file(&normal_path).map_err(|e| format!("{}: {e}", normal_path.display()))?;
    }
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

/// The creatures that can be shown on `family`'s puppet: every creature
/// folder under its textures, and every creature `outfits.toml` dresses
/// that the bestiary puts on this family (the doll cascade over the rigs
/// saved in `<data>/rigs/`, as the stage resolves it).
pub fn family_creatures(data_root: &Path, family: &str) -> Vec<String> {
    let mut out: std::collections::BTreeSet<String> = std::fs::read_dir(dir(data_root, family))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .filter(|name| name != OUTFITS && name != PROJECTED)
        .collect();
    if let Ok((_, outfits)) = outfits(data_root) {
        let saved: std::collections::BTreeSet<String> = std::fs::read_dir(data_root.join("rigs"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".rig.toml")).map(str::to_owned))
            .collect();
        for e in gs_field::bestiary::shared().entries() {
            let key = gs_field::naming::name_token(&e.name);
            if outfits.get(&key).is_some()
                && gs_field::dolls::resolve_doll(e.family.as_deref(), e.creature_type.as_deref(), &|f| saved.contains(f)).is_some_and(|d| d.family == family)
            {
                out.insert(key);
            }
        }
    }
    out.into_iter().collect()
}

/// The creatures of `family` with a built skin: `(creature, its sheet.png)`.
pub fn creature_sheets(data_root: &Path, family: &str) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = std::fs::read_dir(dir(data_root, family))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .filter(|name| name != OUTFITS)
        .map(|name| {
            let sheet = creature_dir(data_root, family, &name).join(SHEET);
            (name, sheet)
        })
        .filter(|(_, sheet)| sheet.is_file())
        .collect();
    out.sort();
    out
}

/// Put `family`'s creature skins into a puppets folder (the pool's
/// `global/puppets/`, or the publish dir's `puppets/`) as
/// `skins/<family>/<creature>.png` (`gs_field::puppets::skin_file_name`),
/// each with its normal sheet as `<creature>.normal.png` when it has one,
/// and drop any there whose creature has no sheet (or normal sheet) any
/// more. Returns how many skins were copied.
pub fn ship_skins(data_root: &Path, family: &str, puppets: &Path) -> Result<usize, String> {
    let sheets = creature_sheets(data_root, family);
    let has_normal = |c: &str| sheets.iter().any(|(s, sheet)| s == c && sheet.with_file_name(NORMAL_SHEET).is_file());
    let into = puppets.join(gs_field::puppets::SKINS_DIR).join(family);
    for stale in std::fs::read_dir(&into).into_iter().flatten().flatten() {
        let name = stale.file_name().to_string_lossy().into_owned();
        let keep = match (name.strip_suffix(".normal.png"), name.strip_suffix(".png")) {
            (Some(c), _) => has_normal(c),
            (None, Some(c)) => sheets.iter().any(|(s, _)| s == c),
            _ => true,
        };
        if !keep {
            std::fs::remove_file(stale.path()).map_err(|e| format!("{}: {e}", stale.path().display()))?;
        }
    }
    if sheets.is_empty() {
        let _ = std::fs::remove_dir(&into);
        return Ok(0);
    }
    std::fs::create_dir_all(&into).map_err(|e| format!("{}: {e}", into.display()))?;
    for (creature, sheet) in &sheets {
        let to = puppets.join(gs_field::puppets::skin_file_name(family, creature));
        std::fs::copy(sheet, &to).map_err(|e| format!("{} -> {}: {e}", sheet.display(), to.display()))?;
        let normal = sheet.with_file_name(NORMAL_SHEET);
        if normal.is_file() {
            let to = puppets.join(gs_field::puppets::skin_normal_file_name(family, creature));
            std::fs::copy(&normal, &to).map_err(|e| format!("{} -> {}: {e}", normal.display(), to.display()))?;
        }
    }
    Ok(sheets.len())
}

/// Put `<data>/outfits/kit.toml` and `outfits.toml` into `into` (the pool's
/// `global/outfits/`, or the publish dir's `outfits/`) -- only when both
/// parse. Returns the files copied (none when the data root has none).
pub fn ship_outfits(data_root: &Path, into: &Path) -> Result<Vec<&'static str>, String> {
    outfits(data_root)?;
    let mut shipped = Vec::new();
    for name in ["kit.toml", "outfits.toml"] {
        let from = data_root.join("outfits").join(name);
        if from.is_file() {
            std::fs::create_dir_all(into).map_err(|e| format!("{}: {e}", into.display()))?;
            std::fs::copy(&from, into.join(name)).map_err(|e| format!("{} -> {}: {e}", from.display(), into.display()))?;
            shipped.push(name);
        }
    }
    Ok(shipped)
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

/// A view's name, as its files and `generator.toml` spell it.
pub fn view_name(view: PaintView) -> &'static str {
    match view {
        PaintView::Front => "front",
        PaintView::Side => "side",
        PaintView::Back => "back",
    }
}

/// The views in the order they are painted by default: the front first.
pub const DEFAULT_ORDER: [PaintView; 3] = [PaintView::Front, PaintView::Side, PaintView::Back];

/// Projection-first (`docs/projection-first-export-spec.md`): what the
/// paintings in `folder` show of `d` from `view` -- those of the views
/// before it in `order` ([`projection_order`]): the front's for the side and
/// the front's and side's for the back by default; a quadruped's side's for
/// its front -- written to `folder/projected/<view>_render.png` and
/// `<view>_unpainted.png`, framed exactly as that view's template at
/// [`DEPTH_SCALE`] times its size.
pub fn export_projected(folder: &Path, d: &Dressed, view: PaintView, grazing: f32, order: [PaintView; 3]) -> Result<Vec<PathBuf>, String> {
    let name = view_name(view);
    let at = order.iter().position(|v| *v == view).ok_or_else(|| format!("{name} is not in the painting order"))?;
    if at == 0 {
        return Err(format!(
            "the {name} is painted first; project it to the {}, then to the {}",
            view_name(order[1]),
            view_name(order[2])
        ));
    }
    let [front_px, side_px, back_px] = read_paintings(folder, "")?;
    let first = view_name(order[0]);
    let painted = |v: PaintView| match v {
        PaintView::Front => &front_px,
        PaintView::Side => &side_px,
        PaintView::Back => &back_px,
    };
    if painted(order[0]).is_none() {
        return Err(format!("no {first}.png in {}: paint the {first} first", folder.display()));
    }
    // Only the views painted before this one: a later view's painting (from
    // an earlier round) must not paint the render it is to be made from.
    let before = |v: PaintView| if order[..at].contains(&v) { as_painting(painted(v)) } else { None };
    let paintings = d.paintings(before(PaintView::Front), before(PaintView::Side), before(PaintView::Back));
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

/// `<data>/generator.toml`: what gs-studio tells the texture generator.
#[derive(serde::Deserialize, Default)]
struct GeneratorSettings {
    /// The families whose puppets are approved: only their depth guides go
    /// into the jobs (`depth_families = ["humanoid"]`).
    #[serde(default)]
    depth_families: Vec<String>,
    /// Per family, the order its views are painted in, projection-first
    /// (`projection_order = { canine = ["side", "front", "back"] }`).
    #[serde(default)]
    projection_order: std::collections::BTreeMap<String, Vec<String>>,
}

fn generator_settings(data_root: &Path) -> Result<GeneratorSettings, String> {
    match std::fs::read_to_string(data_root.join("generator.toml")) {
        Ok(text) => toml::from_str::<GeneratorSettings>(&text).map_err(|e| format!("generator.toml: {e}")),
        Err(_) => Ok(GeneratorSettings::default()),
    }
}

impl GeneratorSettings {
    /// `family`'s painting order: its `projection_order` entry -- all three
    /// views, each once -- else [`DEFAULT_ORDER`].
    fn order(&self, family: &str) -> Result<[PaintView; 3], String> {
        let Some(names) = self.projection_order.get(family) else { return Ok(DEFAULT_ORDER) };
        let bad = || format!("generator.toml: projection_order.{family} must name front, side and back once each, got {names:?}");
        let views: Vec<PaintView> = names
            .iter()
            .map(|n| DEFAULT_ORDER.into_iter().find(|v| view_name(*v) == n.trim()).ok_or_else(bad))
            .collect::<Result<_, _>>()?;
        match views[..] {
            [a, b, c] if a != b && b != c && a != c => Ok([a, b, c]),
            _ => Err(bad()),
        }
    }
}

/// The order `family`'s views are painted in, projection-first: each view
/// after the first is rendered from the paintings of those before it
/// ([`export_projected`]). `projection_order` in `<data>/generator.toml`,
/// else front, side, back. A quadruped's front sees only its face and
/// chest, so its side goes first.
pub fn projection_order(data_root: &Path, family: &str) -> Result<[PaintView; 3], String> {
    generator_settings(data_root)?.order(family)
}

/// `path` relative to `root`, with forward slashes (the jobs file's form).
fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

/// What [`export_jobs`] wrote.
#[derive(Debug)]
pub struct JobsExport {
    pub path: PathBuf,
    pub jobs: usize,
    /// What was left out and why: a rig that failed to build, a creature
    /// whose templates could not be made (a panic included, named).
    pub skipped: Vec<String>,
}

/// Run `work`, turning a panic into an error naming `what`: one bad rig or
/// outfit must not take the whole export down with it.
fn guarded<T>(what: &str, work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)) {
        Ok(result) => result,
        Err(payload) => {
            let why = payload.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| payload.downcast_ref::<String>().cloned());
            Err(format!("{what}: panicked: {}", why.unwrap_or_else(|| "(no message)".into())))
        }
    }
}

/// The texture generator's job list (`docs/texture-generator-plan.md` §2):
/// every bestiary creature a rig in `<data>/rigs/` draws (the doll cascade,
/// stand-ins included), with its family, its outfit, the templates, depth
/// guides and eye marks it is painted over, the order its views are painted
/// in (`projection_order`), and the folder its paintings go in. Every
/// family's templates, and every outfit's its creatures wear, are exported
/// first so the paths exist. Writes `<data>/textures/jobs.json`. A family
/// whose rig will not build, or a creature whose templates fail, is left out
/// and named in [`JobsExport::skipped`]; the rest are still written.
pub fn export_jobs(data_root: &Path, outlines: &Outlines) -> Result<JobsExport, String> {
    let (kit, outfits) = outfits(data_root)?;
    let settings = generator_settings(data_root)?;
    let approved = &settings.depth_families;
    let mut skipped = Vec::new();
    let mut puppets: std::collections::BTreeMap<String, Puppet> = std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(data_root.join("rigs")).map_err(|e| format!("rigs: {e}"))?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(family) = name.strip_suffix(".rig.toml") else { continue };
        let built = guarded(family, || {
            let rig = rig::Rig::load(&entry.path()).map_err(|e| format!("{family}: {e}"))?;
            Puppet::build(&rig, outlines).map_err(|e| format!("{family}: {e}"))
        });
        match built {
            Ok(p) => {
                puppets.insert(family.to_owned(), p);
            }
            Err(e) => skipped.push(e),
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
        let Some(puppet) = puppets.get(&family) else { continue };
        let outfit = outfits.get(&key);
        let order = match settings.order(&family) {
            Ok(o) => o,
            Err(why) => {
                skipped.push(format!("{}: {why}", e.name));
                continue;
            }
        };
        let made = guarded(&format!("{family}/{key}"), || {
            let d = Dressed::new(puppet, outlines, &family, outfit.map(|o| (o, &kit)));
            let templates = d.templates_dir(data_root, &family);
            if !exported.contains(&(family.clone(), d.outfit.clone())) {
                write_templates(&templates, &family, &d)?;
            }
            Ok((d, templates))
        });
        let (d, templates) = match made {
            Ok(m) => m,
            Err(e) => {
                skipped.push(e);
                continue;
            }
        };
        exported.insert((family.clone(), d.outfit.clone()));
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
            "projection_order": order.map(view_name),
            "marks": relative(data_root, &templates.join("marks.json")),
            "out_dir": relative(data_root, &creature_dir(data_root, &family, &key)),
        }));
    }
    let file = serde_json::json!({ "version": 1, "data_root": data_root.to_string_lossy().replace('\\', "/"), "jobs": jobs });
    let path = data_root.join("textures").join("jobs.json");
    std::fs::create_dir_all(path.parent().expect("a folder")).map_err(|e| e.to_string())?;
    std::fs::write(&path, serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(JobsExport { path, jobs: jobs.len(), skipped })
}

/// `--project <family>[/<creature>]:front|side|back` (the command line, for
/// a generator to call): build the family's puppet from `<data>/rigs/`,
/// dress the creature in its outfit, and write its projection renders -- of
/// that view, from the paintings before it in the family's order
/// ([`projection_order`]) -- into the creature's folder (or the family's).
/// Returns the files written.
pub fn project_cli(data_root: &Path, target: &str, grazing: Option<f32>) -> Result<Vec<PathBuf>, String> {
    let usage = || format!("--project wants <family>[/<creature>]:front|side|back, got '{target}'");
    let (who, view) = target.rsplit_once(':').ok_or_else(usage)?;
    let view = DEFAULT_ORDER.into_iter().find(|v| view_name(*v) == view).ok_or_else(usage)?;
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
    export_projected(&folder, &d, view, grazing.unwrap_or(gs_puppet::GRAZING), projection_order(data_root, family)?)
}

/// `--build-sheet <family>[/<creature>]`: [`build_sheet_in`] for that
/// folder, headless. Returns the sheet's path and how many paintings went
/// in.
pub fn build_sheet_cli(data_root: &Path, target: &str) -> Result<(PathBuf, usize), String> {
    let (family, creature) = match target.split_once('/') {
        Some((f, c)) => (f, Some(c)),
        None => (target, None),
    };
    if family.is_empty() {
        return Err(format!("--build-sheet wants <family>[/<creature>], got '{target}'"));
    }
    let outlines = Outlines::load(&data_root.join("outlines.json")).map_err(|e| format!("outlines.json: {e}"))?;
    let rig = rig::Rig::load(&data_root.join("rigs").join(rig::Rig::file_name(family))).map_err(|e| format!("{family}: {e}"))?;
    let puppet = Puppet::build(&rig, &outlines).map_err(|e| format!("{family}: {e}"))?;
    let (d, folder) = match creature {
        Some(c) => (Dressed::creature(data_root, &puppet, &outlines, family, c)?, creature_dir(data_root, family, c)),
        None => (Dressed::new(&puppet, &outlines, family, None), dir(data_root, family)),
    };
    build_sheet_in(&folder, &d)
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
    let mut texture = sheet_texture(puppet, w, h, rgba)?;
    let normal = normal_sheet_path(data_root, family);
    if normal.is_file() {
        let (nw, nh, normals) = read_png(&normal)?;
        if (nw, nh) == (texture.layout.w, texture.layout.h) {
            texture.normal = Some(normals);
        } else {
            return Err(format!("normal.png is {nw}x{nh} but this rig's pattern is {}x{}: build the sheet again", texture.layout.w, texture.layout.h));
        }
    }
    Ok(Some(texture))
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
    Ok(Texture { layout, rgba, normal: None })
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
        assert!(export_projected(&folder, &d, PaintView::Back, gs_puppet::GRAZING, DEFAULT_ORDER).unwrap_err().contains("front"));
        paint_red(&folder.join("front_paint.png"), &folder.join("front.png"));
        let files = export_projected(&folder, &d, PaintView::Side, gs_puppet::GRAZING, DEFAULT_ORDER).unwrap();
        let (tw, th, _) = read_png(&folder.join("side_paint.png")).unwrap();
        for f in &files {
            let (w, h, _) = read_png(f).unwrap();
            assert_eq!((w, h), (tw * DEPTH_SCALE, th * DEPTH_SCALE), "{}", f.display());
        }
        let _ = std::fs::remove_dir_all(&data);
    }

    /// A side-first family (a quadruped's front sees only its face and
    /// chest): the side is painted first and the front is rendered from it;
    /// a front painting from an earlier round does not paint its own render.
    #[test]
    fn a_side_first_family_projects_its_front_from_the_side() {
        let (puppet, outlines) = humanoid();
        let data = scratch("side_first");
        let folder = export_templates(&data, "humanoid", &puppet, &outlines).unwrap();
        let d = Dressed::new(&puppet, &outlines, "humanoid", None);
        let order = [PaintView::Side, PaintView::Front, PaintView::Back];
        assert!(export_projected(&folder, &d, PaintView::Front, gs_puppet::GRAZING, order).unwrap_err().contains("paint the side first"));
        assert!(export_projected(&folder, &d, PaintView::Side, gs_puppet::GRAZING, order).unwrap_err().contains("painted first"));
        paint_red(&folder.join("side_paint.png"), &folder.join("side.png"));
        let files = export_projected(&folder, &d, PaintView::Front, gs_puppet::GRAZING, order).unwrap();
        assert!(files[0].ends_with("projected/front_render.png"), "{}", files[0].display());
        let (_, _, render) = read_png(&files[0]).unwrap();
        let red = render.chunks_exact(4).filter(|c| c[3] > 0 && c[0] > 180 && c[1] < 80).count();
        assert!(red > 0, "the side's paint shows on the front render");
        // A front painted in an earlier round is not a view before the front.
        let (w, h, px) = read_png(&folder.join("front_paint.png")).unwrap();
        let blue: Vec<u8> = px.chunks_exact(4).flat_map(|c| if c[3] > 0 { [0, 0, 255, 255] } else { [0, 0, 0, 0] }).collect();
        write_png(&folder.join("front.png"), w, h, &blue).unwrap();
        let again = export_projected(&folder, &d, PaintView::Front, gs_puppet::GRAZING, order).unwrap();
        let (_, _, render) = read_png(&again[0]).unwrap();
        assert!(!render.chunks_exact(4).any(|c| c[3] > 0 && c[2] > 200 && c[0] < 60), "the old front painting leaked into its own render");
        let _ = std::fs::remove_dir_all(&data);
    }

    /// `projection_order` in generator.toml: a family's own order, the
    /// default for the rest, and a clear error for a bad one.
    #[test]
    fn a_family_s_projection_order_comes_from_generator_toml() {
        let data = scratch("order");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("generator.toml"), "projection_order = { canine = [\"side\", \"front\", \"back\"], worm = [\"side\", \"side\", \"back\"] }\n").unwrap();
        assert_eq!(projection_order(&data, "canine").unwrap(), [PaintView::Side, PaintView::Front, PaintView::Back]);
        assert_eq!(projection_order(&data, "humanoid").unwrap(), DEFAULT_ORDER);
        assert!(projection_order(&data, "worm").unwrap_err().contains("once each"));
        let _ = std::fs::remove_dir_all(&data);
        assert_eq!(projection_order(&data, "canine").unwrap(), DEFAULT_ORDER, "no file: the default");
    }

    /// A panic while exporting is an error naming what it was working on.
    #[test]
    fn a_panic_in_the_export_is_named_not_fatal() {
        let e = guarded("giant/goliath", || -> Result<(), String> { panic!("no such station") }).unwrap_err();
        assert!(e.contains("giant/goliath") && e.contains("no such station"), "{e}");
        assert_eq!(guarded("x", || Ok::<_, String>(3)), Ok(3));
    }
}
