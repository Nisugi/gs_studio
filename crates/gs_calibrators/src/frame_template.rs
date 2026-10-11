//! Hydra's frame templates, without egui (NEW, 2026-10-11): the five-piece
//! folder and its `frame.toml`, the validator, the slicer that cuts the
//! pieces out of an assembled frame picture, and the layout that places
//! the pieces round a window, a row of cells and a grid. The Frames tab
//! (`frame_calibration`) draws what this module computes.
//!
//! A frame is five PNG pieces and a small TOML in one folder,
//! `<pool>/global/images/frames/<name>/` (Hydra: `<data>/images/frames/<name>/`):
//!
//! ```text
//! frame.toml
//! edge.png        required   the TOP run, seamless left to right
//! sticker.png     *          decoration laid over EVERY junction (corner, tee,
//!                            cross alike), no rail in it; the rails run on
//!                            under it. The simpler design (2026-10-11).
//! corner.png      *          the TOP-LEFT corner, a square centred on the joint,
//!                            its arms reading as the rail (the older design)
//! tee.png         optional   the T on the top run, stem down
//! cross.png       optional   both bands
//! ornament.png    optional   once per run, centred on the run's midpoint
//! ```
//!
//! `*`: one of sticker or corner. With a sticker, the rails are drawn full
//! length through the joints (outer rails over the dividers) and the
//! sticker over every joint; with a corner, the rails stop at the joints
//! and the knot pieces cover them.
//!
//! Hydra draws in points; art is authored at [`PX_PER_PT`] pixels per
//! point, and Hydra derives the scale from `thickness` against the edge's
//! pixel height. The knot is a square of `knot` points centred on the
//! joint where two edge centrelines cross; corner, tee and cross are all
//! that square. The top piece is turned for the other three sides, by
//! rotation or by reflection ([`Turn`]).

use std::path::{Path, PathBuf};

use image::RgbaImage;

/// Pixels per point the art is authored at.
pub const PX_PER_PT: u32 = 4;

/// How the top piece becomes the other three sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Turn {
    /// 90, 180, 270 degrees clockwise: a rose, a knot, a rope read right.
    #[default]
    Rotate,
    /// Left, right and bottom are reflections: an arrow keeps its handedness.
    Mirror,
}

/// How the edge fills a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EdgeMode {
    /// Tiled from the run's centre outward, the end cuts symmetric.
    #[default]
    Tile,
    /// A whole number of tiles between the knots, each stretched a little
    /// so they fit: no tile is cut at a knot.
    Fit,
    /// Tiled like `Tile`, every other tile reflected, so each join meets
    /// its own mirror image: no edge needs to be seamless.
    Mirror,
    /// One long element stretched over the run.
    Stretch,
}

/// `frame.toml`.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameToml {
    /// Points, 0 to 24.
    pub thickness: u32,
    /// Points, at least `thickness + 1`.
    pub knot: u32,
    pub turn: Turn,
    pub edge: EdgeMode,
    /// `#rrggbb`; when set the pieces are multiplied by it.
    pub tint: Option<String>,
    /// gs-studio's own, which Hydra ignores: each piece's cut box on its
    /// raw picture (`raw/<piece>.png`), `[x, y, w, h]`, so the calibration
    /// is kept; and the ornament's height in points.
    pub cuts: std::collections::BTreeMap<String, [f32; 4]>,
    pub ornament_height: Option<f32>,
    /// Which outer runs get the ornament (`ornament_on`), top, right,
    /// bottom, left: default all four.
    pub ornament_on: [bool; 4],
}

impl Default for FrameToml {
    fn default() -> Self {
        FrameToml { thickness: 12, knot: 18, turn: Turn::Rotate, edge: EdgeMode::Tile, tint: None, cuts: Default::default(), ornament_height: None, ornament_on: [true; 4] }
    }
}

impl FrameToml {
    pub fn parse(text: &str) -> Result<Self, String> {
        let table: toml::Table = text.parse().map_err(|e| format!("frame.toml: {e}"))?;
        let int = |key: &str| table.get(key).and_then(toml::Value::as_integer).and_then(|i| u32::try_from(i).ok());
        let mut out = FrameToml::default();
        if let Some(t) = int("thickness") {
            out.thickness = t;
        }
        if let Some(k) = int("knot") {
            out.knot = k;
        }
        match table.get("turn").and_then(toml::Value::as_str).map(str::trim) {
            Some("mirror") => out.turn = Turn::Mirror,
            Some("rotate") | None => out.turn = Turn::Rotate,
            Some(other) => return Err(format!("frame.toml: turn = \"{other}\" is neither rotate nor mirror")),
        }
        match table.get("edge").and_then(toml::Value::as_str).map(str::trim) {
            Some("stretch") => out.edge = EdgeMode::Stretch,
            Some("fit") => out.edge = EdgeMode::Fit,
            Some("mirror") => out.edge = EdgeMode::Mirror,
            Some("tile") | None => out.edge = EdgeMode::Tile,
            Some(other) => return Err(format!("frame.toml: edge = \"{other}\" is none of tile, fit, mirror, stretch")),
        }
        out.tint = table.get("tint").and_then(toml::Value::as_str).map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
        if let Some(cuts) = table.get("cuts").and_then(toml::Value::as_table) {
            for (name, v) in cuts {
                let nums: Option<Vec<f32>> = v.as_array().map(|a| a.iter().filter_map(|x| x.as_float().or_else(|| x.as_integer().map(|i| i as f64))).map(|f| f as f32).collect());
                if let Some(n) = nums {
                    if let [x, y, w, h] = n[..] {
                        out.cuts.insert(name.clone(), [x, y, w, h]);
                    }
                }
            }
        }
        if let Some(on) = table.get("ornament_on").and_then(toml::Value::as_array) {
            let names: Vec<String> = on.iter().filter_map(|v| v.as_str().map(|s| s.trim().to_ascii_lowercase())).collect();
            out.ornament_on = ["top", "right", "bottom", "left"].map(|side| names.iter().any(|n| n == side));
        }
        out.ornament_height = table.get("ornament_height").and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64))).map(|f| f as f32);
        Ok(out)
    }

    pub fn to_toml_string(&self) -> String {
        let mut s = format!(
            "thickness = {}        # points, 0 to 24\nknot = {}             # points, at least thickness + 1\nturn = \"{}\"       # or \"{}\"\nedge = \"{}\"         # or {}\n",
            self.thickness,
            self.knot,
            match self.turn {
                Turn::Rotate => "rotate",
                Turn::Mirror => "mirror",
            },
            match self.turn {
                Turn::Rotate => "mirror",
                Turn::Mirror => "rotate",
            },
            match self.edge {
                EdgeMode::Tile => "tile",
                EdgeMode::Fit => "fit",
                EdgeMode::Mirror => "mirror",
                EdgeMode::Stretch => "stretch",
            },
            match self.edge {
                EdgeMode::Tile => "fit, mirror, stretch",
                EdgeMode::Fit => "tile, mirror, stretch",
                EdgeMode::Mirror => "tile, fit, stretch",
                EdgeMode::Stretch => "tile, fit, mirror",
            },
        );
        if self.ornament_on != [true; 4] {
            let names: Vec<String> = ["top", "right", "bottom", "left"].iter().zip(self.ornament_on).filter(|(_, on)| *on).map(|(n, _)| format!("\"{n}\"")).collect();
            s.push_str(&format!("ornament_on = [{}]   # which runs carry the ornament\n", names.join(", ")));
        }
        match &self.tint {
            Some(tint) => s.push_str(&format!("tint = \"{tint}\"\n")),
            None => s.push_str("# tint = \"#c8b89a\"    # optional; when set the pieces are multiplied by it\n"),
        }
        if let Some(h) = self.ornament_height {
            s.push_str(&format!("ornament_height = {h}   # points; gs-studio's, Hydra ignores it\n"));
        }
        if !self.cuts.is_empty() {
            s.push_str("\n# gs-studio's calibration: each piece's cut box on raw/<piece>.png. Hydra ignores it.\n[cuts]\n");
            for (name, [x, y, w, h]) in &self.cuts {
                s.push_str(&format!("{name} = [{x}, {y}, {w}, {h}]\n"));
            }
        }
        s
    }

    pub fn read(dir: &Path) -> Result<Self, String> {
        let path = dir.join("frame.toml");
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text)
    }

    /// The thickness in authored pixels.
    pub fn thickness_px(&self) -> u32 {
        self.thickness * PX_PER_PT
    }

    /// The knot square's side in authored pixels.
    pub fn knot_px(&self) -> u32 {
        self.knot * PX_PER_PT
    }
}

/// The pieces, decoded.
#[derive(Debug, Clone, Default)]
pub struct Pieces {
    pub edge: Option<RgbaImage>,
    pub corner: Option<RgbaImage>,
    pub tee: Option<RgbaImage>,
    pub cross: Option<RgbaImage>,
    pub ornament: Option<RgbaImage>,
    pub sticker: Option<RgbaImage>,
}

/// The six files, in the order the tab shows them.
pub const PIECE_NAMES: [&str; 6] = ["edge", "sticker", "corner", "tee", "cross", "ornament"];

impl Pieces {
    pub fn get(&self, name: &str) -> Option<&RgbaImage> {
        match name {
            "edge" => self.edge.as_ref(),
            "corner" => self.corner.as_ref(),
            "tee" => self.tee.as_ref(),
            "cross" => self.cross.as_ref(),
            "ornament" => self.ornament.as_ref(),
            "sticker" => self.sticker.as_ref(),
            _ => None,
        }
    }

    /// Read whatever pieces `dir` holds; a file that is there but will not
    /// decode is an error, a file that is not there is `None`.
    pub fn read(dir: &Path) -> Result<Self, String> {
        let mut out = Pieces::default();
        for name in PIECE_NAMES {
            let path = dir.join(format!("{name}.png"));
            if !path.is_file() {
                continue;
            }
            let img = image::open(&path).map_err(|e| format!("{}: {e}", path.display()))?.to_rgba8();
            match name {
                "edge" => out.edge = Some(img),
                "corner" => out.corner = Some(img),
                "tee" => out.tee = Some(img),
                "cross" => out.cross = Some(img),
                "sticker" => out.sticker = Some(img),
                _ => out.ornament = Some(img),
            }
        }
        Ok(out)
    }

    /// The raw pictures to calibrate from: `dir/raw/<piece>.png` where it
    /// is, else the piece itself.
    pub fn read_raw(dir: &Path) -> Result<Self, String> {
        let raw = Self::read(&dir.join("raw"))?;
        let fitted = Self::read(dir)?;
        let pick = |r: Option<RgbaImage>, f: Option<RgbaImage>| r.or(f);
        Ok(Pieces { edge: pick(raw.edge, fitted.edge), corner: pick(raw.corner, fitted.corner), tee: pick(raw.tee, fitted.tee), cross: pick(raw.cross, fitted.cross), ornament: pick(raw.ornament, fitted.ornament), sticker: pick(raw.sticker, fitted.sticker) })
    }

    /// Whether `dir` keeps raw pictures under `raw/`.
    pub fn has_raw(dir: &Path) -> bool {
        PIECE_NAMES.iter().any(|n| dir.join("raw").join(format!("{n}.png")).is_file())
    }

    /// Write these as the raw pictures, `dir/raw/<piece>.png`.
    pub fn write_raw(&self, dir: &Path) -> Result<(), String> {
        let raw = dir.join("raw");
        std::fs::create_dir_all(&raw).map_err(|e| format!("{}: {e}", raw.display()))?;
        for name in PIECE_NAMES {
            if let Some(img) = self.get(name) {
                let path = raw.join(format!("{name}.png"));
                img.save(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            }
        }
        Ok(())
    }

    /// Write the pieces that are there into `dir` (made if need be) with
    /// `toml` as its `frame.toml`. A piece that is `None` is removed from
    /// the folder, so the folder says what the tab showed.
    pub fn write(&self, dir: &Path, toml: &FrameToml) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for name in PIECE_NAMES {
            let path = dir.join(format!("{name}.png"));
            match self.get(name) {
                Some(img) => img.save(&path).map_err(|e| format!("{}: {e}", path.display()))?,
                None => {
                    if path.exists() {
                        std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                    }
                }
            }
        }
        std::fs::write(dir.join("frame.toml"), toml.to_toml_string()).map_err(|e| format!("{}: {e}", dir.display()))
    }
}

/// One thing the validator found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// A warning is said and export goes ahead; an error blocks it.
    pub warning: bool,
    pub text: String,
}

fn has_transparency(img: &RgbaImage) -> bool {
    img.pixels().any(|p| p.0[3] < 255)
}

/// The spec's checks: thickness and knot in range; corner, tee and cross the
/// same square; the edge's height to the knot's side as thickness to knot;
/// alpha present; the ornament no taller than the knot (a warning); the
/// edge's height a whole number of pixels per point (a warning).
pub fn validate(toml: &FrameToml, pieces: &Pieces) -> Vec<Problem> {
    let mut out = Vec::new();
    let error = |text: String| Problem { warning: false, text };
    let warn = |text: String| Problem { warning: true, text };
    if toml.thickness > 24 {
        out.push(error(format!("thickness {} pt is over 24", toml.thickness)));
    }
    if toml.knot < toml.thickness + 1 {
        out.push(error(format!("knot {} pt must be at least thickness + 1 ({})", toml.knot, toml.thickness + 1)));
    }
    let Some(edge) = &pieces.edge else {
        out.push(error("edge.png is required".to_owned()));
        return out;
    };
    let (ew, eh) = edge.dimensions();
    let (knot_name, knot) = match (&pieces.sticker, &pieces.corner) {
        (Some(s), _) => ("sticker", s),
        (None, Some(c)) => ("corner", c),
        (None, None) => {
            out.push(error("sticker.png or corner.png is required".to_owned()));
            return out;
        }
    };
    let (cw, ch) = knot.dimensions();
    if cw != ch {
        out.push(error(format!("{knot_name}.png is {cw}x{ch}, not a square")));
    }
    if let (Some(s), Some(c)) = (&pieces.sticker, &pieces.corner) {
        if s.dimensions() != c.dimensions() {
            out.push(warn(format!("sticker.png is {}x{} and corner.png {}x{}: the sticker is used, the corner kept", s.width(), s.height(), c.width(), c.height())));
        }
    }
    for (name, piece) in [("tee", &pieces.tee), ("cross", &pieces.cross)] {
        if let Some(p) = piece {
            if p.dimensions() != (cw, ch) {
                out.push(error(format!("{name}.png is {}x{}; corner, tee and cross must all be the {cw}x{ch} square", p.width(), p.height())));
            }
        }
    }
    if toml.thickness > 0 && toml.knot > 0 && eh * toml.knot != cw * toml.thickness {
        let want = eh * toml.knot / toml.thickness.max(1);
        out.push(error(format!(
            "edge.png is {eh} px tall for thickness {} pt ({:.2} px/pt), so the knot of {} pt wants a {want} px square; {knot_name}.png is {cw} px",
            toml.thickness,
            eh as f32 / toml.thickness as f32,
            toml.knot
        )));
    }
    if toml.thickness > 0 && eh % toml.thickness != 0 {
        out.push(warn(format!("edge.png's {eh} px over thickness {} pt is not a whole number of pixels per point; Hydra scales by that ratio", toml.thickness)));
    }
    if ew < eh {
        out.push(warn(format!("edge.png is {ew} px long for {eh} px tall: a very short tile")));
    }
    if !has_transparency(knot) {
        out.push(warn(format!("{knot_name}.png has no transparent pixel: the square overhangs the band and should be clear there")));
    }
    if let Some(orn) = &pieces.ornament {
        if orn.height() > ch {
            out.push(warn(format!("ornament.png is {} px tall, taller than the {ch} px knot: it reaches the knots on a short run", orn.height())));
        }
    }
    out
}

/// Whether export may go ahead: no error among `problems`.
pub fn exportable(problems: &[Problem]) -> bool {
    problems.iter().all(|p| p.warning)
}

/// The pool's frame templates: subfolders of `frames/` holding a `frame.toml`.
pub fn list_templates() -> Vec<String> {
    let Ok(root) = creature_field::pool_dirs::global_images_dir() else { return Vec::new() };
    let mut out: Vec<String> = std::fs::read_dir(root.join("frames"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("frame.toml").is_file())
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .collect();
    out.sort();
    out
}

/// The pool's assembled frame pictures, to slice: `frames/*.png`, as (stem, path).
pub fn list_pictures() -> Vec<(String, PathBuf)> {
    let Ok(root) = creature_field::pool_dirs::global_images_dir() else { return Vec::new() };
    let mut out: Vec<(String, PathBuf)> = std::fs::read_dir(root.join("frames"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("png")))
        .filter_map(|p| Some((p.file_stem()?.to_str()?.to_owned(), p)))
        .collect();
    out.sort();
    out
}

/// A template's folder in the pool.
pub fn template_dir(name: &str) -> Option<PathBuf> {
    Some(creature_field::pool_dirs::global_images_dir().ok()?.join("frames").join(name.trim()))
}

/// Where the pieces lie on an assembled frame picture, in the picture's
/// pixels. The picture is taken to be symmetric about its vertical centre.
#[derive(Debug, Clone, PartialEq)]
pub struct SliceGuide {
    /// The top-left joint: where the top and left edge centrelines cross.
    pub joint: [f32; 2],
    /// The edge band's thickness.
    pub band: f32,
    /// The knot square's side on the picture, and its centre: sized and
    /// placed on their own (a painted corner is seldom exactly 1.5 bands
    /// centred on the joint). The cut is resampled to the template's knot.
    pub knot: f32,
    pub knot_at: [f32; 2],
    /// The edge tile's length, cut centred on the picture's middle, shifted by `tile_offset`.
    pub tile_len: f32,
    pub tile_offset: f32,
    /// The ornament's width and height, centred on the top run's midpoint; None for no ornament.
    pub ornament: Option<[f32; 2]>,
}

impl SliceGuide {
    /// A first guess for a `w` by `h` picture: the band a tenth of the
    /// width, the joint half a band in from the corner, the knot 1.5 bands
    /// on the joint.
    pub fn guess(w: u32, h: u32) -> Self {
        let band = (w.min(h) as f32 / 10.0).round().max(4.0);
        Self::from_band([band / 2.0, band / 2.0], band)
    }

    /// A guide from a joint and a band: the knot 1.5 bands, centred on the
    /// joint; the tile two bands long.
    pub fn from_band(joint: [f32; 2], band: f32) -> Self {
        SliceGuide { joint, band, knot: (band * 1.5).round(), knot_at: joint, tile_len: (band * 2.0).round(), tile_offset: 0.0, ornament: None }
    }

    /// A first guess measured off the picture: the band is how far the
    /// paint reaches in from the top along the middle column and from the
    /// left along the middle row (the first clear pixel), the joint half a
    /// band in. A picture painted to its centre gets [`Self::guess`].
    pub fn measure(picture: &RgbaImage) -> Self {
        let (w, h) = picture.dimensions();
        let clear = |p: &image::Rgba<u8>| p.0[3] < 16;
        let top = (0..h).find(|&y| clear(picture.get_pixel(w / 2, y)));
        let left = (0..w).find(|&x| clear(picture.get_pixel(x, h / 2)));
        match (top, left) {
            (Some(top), Some(left)) if top > 0 && left > 0 => Self::from_band([left as f32 / 2.0, top as f32 / 2.0], (top + left) as f32 / 2.0),
            _ => Self::guess(w, h),
        }
    }

    /// Picture pixels per authored pixel: the band against the thickness.
    pub fn scale(&self, toml: &FrameToml) -> f32 {
        self.band / toml.thickness_px().max(1) as f32
    }

    /// The rectangles the slicer cuts, in picture pixels: (name, [x, y, w, h]).
    pub fn crops(&self, picture_w: u32) -> Vec<(&'static str, [f32; 4])> {
        let [_, jy] = self.joint;
        let [kx, ky] = self.knot_at;
        let k = self.knot;
        let mid = picture_w as f32 / 2.0;
        let mut out = vec![
            ("corner", [kx - k / 2.0, ky - k / 2.0, k, k]),
            ("edge", [mid + self.tile_offset - self.tile_len / 2.0, jy - self.band / 2.0, self.tile_len, self.band]),
        ];
        if let Some([w, h]) = self.ornament {
            out.push(("ornament", [mid - w / 2.0, jy - h / 2.0, w, h]));
        }
        out
    }
}

/// `picture` cut at `[x, y, w, h]` and scaled down by `scale`. A cut that
/// leaves the picture is padded with transparent pixels (a knot square
/// overhangs the picture's edge by (k - t)/2: that overhang is clipped at
/// the window's bound anyway); a cut wholly off it is refused.
fn crop_scaled(picture: &RgbaImage, [x, y, w, h]: [f32; 4], scale: f32, name: &str) -> Result<RgbaImage, String> {
    let (pw, ph) = picture.dimensions();
    if w < 1.0 || h < 1.0 {
        return Err(format!("{name}: an empty cut"));
    }
    let (x0, y0) = (x.round() as i64, y.round() as i64);
    let (cw, ch) = (w.round().max(1.0) as u32, h.round().max(1.0) as u32);
    if x0 + cw as i64 <= 0 || y0 + ch as i64 <= 0 || x0 >= pw as i64 || y0 >= ph as i64 {
        return Err(format!("{name}: the cut [{x:.0}, {y:.0}, {w:.0}x{h:.0}] lies off the {pw}x{ph} picture"));
    }
    let mut cut = RgbaImage::new(cw, ch);
    for cy in 0..ch {
        let sy = y0 + cy as i64;
        if sy < 0 || sy >= ph as i64 {
            continue;
        }
        for cx in 0..cw {
            let sx = x0 + cx as i64;
            if sx >= 0 && sx < pw as i64 {
                cut.put_pixel(cx, cy, *picture.get_pixel(sx as u32, sy as u32));
            }
        }
    }
    let (tw, th) = ((cw as f32 / scale).round().max(1.0) as u32, (ch as f32 / scale).round().max(1.0) as u32);
    Ok(if (tw, th) == (cw, ch) { cut } else { image::imageops::resize(&cut, tw, th, image::imageops::FilterType::Lanczos3) })
}

/// Cut edge, corner and ornament out of `picture` where `guide` says, at
/// the template's authored size: the band comes out `thickness` points at
/// [`PX_PER_PT`] and the edge and ornament scale with it; the knot square
/// is resampled to `knot` points whatever its size on the picture. Tee and
/// cross have no place on a single frame picture and are left `None`.
pub fn slice(picture: &RgbaImage, guide: &SliceGuide, toml: &FrameToml) -> Result<Pieces, String> {
    let scale = guide.scale(toml);
    if !(scale > 0.0) || toml.thickness == 0 {
        return Err("thickness must be above 0 to slice".to_owned());
    }
    let mut out = Pieces::default();
    for (name, rect) in guide.crops(picture.width()) {
        let mut img = crop_scaled(picture, rect, scale, name)?;
        match name {
            "corner" => {
                let k = toml.knot_px();
                if img.dimensions() != (k, k) {
                    img = image::imageops::resize(&img, k, k, image::imageops::FilterType::Lanczos3);
                }
                out.corner = Some(img);
            }
            "edge" => {
                let t = toml.thickness_px();
                if img.height() != t {
                    img = image::imageops::resize(&img, img.width().max(1), t, image::imageops::FilterType::Lanczos3);
                }
                out.edge = Some(img);
            }
            _ => out.ornament = Some(img),
        }
    }
    Ok(out)
}

// ---- Raw pieces from an image model: keying and fitting ----------------

/// A binary mask over an image.
struct Mask {
    w: usize,
    h: usize,
    on: Vec<bool>,
}

impl Mask {
    /// Square dilation (`grow` true) or erosion by `r` pixels: separable
    /// max / min runs.
    fn morph(&self, r: usize, grow: bool) -> Mask {
        let pass = |src: &[bool], w: usize, h: usize, along_x: bool| -> Vec<bool> {
            let mut out = vec![!grow; w * h];
            for y in 0..h {
                for x in 0..w {
                    let mut v = !grow;
                    let (n, i) = if along_x { (w, x) } else { (h, y) };
                    let lo = i.saturating_sub(r);
                    let hi = (i + r).min(n - 1);
                    for j in lo..=hi {
                        let s = if along_x { src[y * w + j] } else { src[j * w + x] };
                        if s == grow {
                            v = grow;
                            break;
                        }
                    }
                    out[y * w + x] = v;
                }
            }
            out
        };
        let a = pass(&self.on, self.w, self.h, true);
        Mask { w: self.w, h: self.h, on: pass(&a, self.w, self.h, false) }
    }

    /// Connected components (4-way) of the set pixels: a label per pixel
    /// (0 = unset) and each label's size.
    fn components(&self) -> (Vec<u32>, Vec<usize>) {
        let mut label = vec![0u32; self.w * self.h];
        let mut sizes = vec![0usize];
        let mut stack = Vec::new();
        for start in 0..self.on.len() {
            if !self.on[start] || label[start] != 0 {
                continue;
            }
            let id = sizes.len() as u32;
            sizes.push(0);
            label[start] = id;
            stack.push(start);
            while let Some(i) = stack.pop() {
                sizes[id as usize] += 1;
                let (x, y) = (i % self.w, i / self.w);
                let mut push = |j: usize| {
                    if self.on[j] && label[j] == 0 {
                        label[j] = id;
                        stack.push(j);
                    }
                };
                if x > 0 {
                    push(i - 1);
                }
                if x + 1 < self.w {
                    push(i + 1);
                }
                if y > 0 {
                    push(i - self.w);
                }
                if y + 1 < self.h {
                    push(i + self.w);
                }
            }
        }
        (label, sizes)
    }
}

/// Whether every pixel is opaque: an image model that was asked for
/// transparency and painted a checkerboard instead.
pub fn is_opaque(img: &RgbaImage) -> bool {
    img.pixels().all(|p| p.0[3] == 255)
}

/// Key a painted checkerboard out of a piece: the checker is neutral grey
/// in the range its top-left block shows; the piece is what is left once its thin engraved
/// lines are closed over, kept as the large connected parts, with small
/// holes (engraving noise) filled and large enclosed pockets left clear.
pub fn key_checker(img: &RgbaImage) -> RgbaImage {
    let (w, h) = (img.width() as usize, img.height() as usize);
    // The checker's tones, read off the four 256x256 corner blocks: checker
    // in every piece, both tones of a coarse checker, and the lighter
    // checker some pictures have toward the bottom.
    let mut block: Vec<i32> = Vec::new();
    for (x0, y0) in [(0, 0), (w.saturating_sub(256), 0), (0, h.saturating_sub(256)), (w.saturating_sub(256), h.saturating_sub(256))] {
        for y in y0..(y0 + 256).min(h) {
            for x in x0..(x0 + 256).min(w) {
                let [r, g, b, _] = img.get_pixel(x as u32, y as u32).0;
                block.push((r as i32 + g as i32 + b as i32) / 3);
            }
        }
    }
    block.sort_unstable();
    let pct = |f: f32| block[((block.len() - 1) as f32 * f) as usize];
    let (lo, hi) = (pct(0.005) - 8, pct(0.995) + 8);
    let checker: Vec<bool> = img
        .pixels()
        .map(|p| {
            let [r, g, b, _] = p.0;
            let (mn, mx) = (r.min(g).min(b) as i32, r.max(g).max(b) as i32);
            let v = (r as i32 + g as i32 + b as i32) / 3;
            mx - mn <= 12 && (lo..=hi).contains(&v)
        })
        .collect();
    let piece = Mask { w, h, on: checker.iter().map(|c| !c).collect() };
    // Closing: dilate then erode, by 4 px (a 9x9 window).
    let closed = piece.morph(4, true).morph(4, false);
    let (label, sizes) = closed.components();
    let big = (w * h) as f32 * 0.002;
    let mut keep: Vec<bool> = label.iter().map(|&l| l != 0 && sizes[l as usize] as f32 > big).collect();
    // Holes: unset pixels not connected to the border; the small ones are filled.
    let holes = Mask { w, h, on: keep.iter().map(|k| !k).collect() };
    let (hl, hs) = holes.components();
    let mut touches_border = vec![false; hs.len()];
    for x in 0..w {
        touches_border[hl[x] as usize] = true;
        touches_border[hl[(h - 1) * w + x] as usize] = true;
    }
    for y in 0..h {
        touches_border[hl[y * w] as usize] = true;
        touches_border[hl[y * w + w - 1] as usize] = true;
    }
    for i in 0..keep.len() {
        let l = hl[i] as usize;
        if l != 0 && !touches_border[l] && hs[l] < 400 {
            keep[i] = true;
        }
    }
    let mut out = img.clone();
    for (i, p) in out.pixels_mut().enumerate() {
        // One pixel of softness at the cut: the mean of the 3x3 neighbourhood.
        let (x, y) = (i % w, i / w);
        let mut n = 0.0f32;
        let mut on = 0.0f32;
        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                    n += 1.0;
                    if keep[ny as usize * w + nx as usize] {
                        on += 1.0;
                    }
                }
            }
        }
        let a = if keep[i] { (0.5 + 0.5 * on / n).min(1.0) } else { 0.5 * on / n };
        p.0[3] = (a * 255.0).round() as u8;
    }
    out
}

/// The set pixels' bounding box, `[x0, y0, x1, y1]` inclusive, of the
/// mostly opaque pixels; None for an empty image.
fn opaque_bbox(img: &RgbaImage) -> Option<[u32; 4]> {
    let mut b: Option<[u32; 4]> = None;
    for (x, y, p) in img.enumerate_pixels() {
        if p.0[3] > 128 {
            b = Some(match b {
                None => [x, y, x, y],
                Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
            });
        }
    }
    b
}

/// The opaque run along column `x` (or row `y` when `along_x`), as (first, last).
fn opaque_run(img: &RgbaImage, at: u32, along_x: bool) -> Option<(u32, u32)> {
    let n = if along_x { img.width() } else { img.height() };
    let on = |i: u32| if along_x { img.get_pixel(i, at).0[3] > 128 } else { img.get_pixel(at, i).0[3] > 128 };
    let first = (0..n).find(|&i| on(i))?;
    let last = (0..n).rev().find(|&i| on(i))?;
    Some((first, last))
}

fn resized(img: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    if img.dimensions() == (w, h) {
        img.clone()
    } else {
        image::imageops::resize(img, w.max(1), h.max(1), image::imageops::FilterType::Lanczos3)
    }
}

/// Where a raw piece (as an image model paints it: any canvas, any scale)
/// should be cut, measured from what it shows: `[x, y, w, h]` on the
/// picture. The edge: its rail's rows over the opaque width. A knot: its
/// two rails measured where they leave the picture, and the knot square
/// on their centrelines at `knot` to the rail's thickness. The ornament:
/// its opaque bounds. The cut is what [`cut_piece`] turns into the piece;
/// the tab lets the user move and size it.
pub fn measure_cut(name: &str, img: &RgbaImage, toml: &FrameToml) -> Result<[f32; 4], String> {
    let [x0, y0, x1, y1] = opaque_bbox(img).ok_or_else(|| format!("{name}.png: nothing opaque in it"))?;
    let rail = |at: u32, along_x: bool| opaque_run(img, at, along_x).ok_or_else(|| format!("{name}.png: no rail found"));
    match name {
        "edge" => {
            let (ya, yb) = rail(x0 + 3.min(x1 - x0), false)?;
            Ok([x0 as f32, ya as f32, (x1 - x0 + 1) as f32, (yb - ya + 1) as f32])
        }
        "ornament" => Ok([x0 as f32, y0 as f32, (x1 - x0 + 1) as f32, (y1 - y0 + 1) as f32]),
        "sticker" => {
            // Its opaque bounds, squared up about their centre.
            let (w, h) = ((x1 - x0 + 1) as f32, (y1 - y0 + 1) as f32);
            let side = w.max(h);
            Ok([x0 as f32 + (w - side) / 2.0, y0 as f32 + (h - side) / 2.0, side, side])
        }
        "corner" | "tee" | "cross" => {
            // Where the horizontal rail is read (a column), and the vertical (a row).
            let (hcol, vrow) = match name {
                "corner" => (x1.saturating_sub(3), y1.saturating_sub(3)),
                "tee" => (x0 + 3, y1.saturating_sub(3)),
                _ => (x0 + 3, y0 + 3),
            };
            let (ha, hb) = rail(hcol, false)?;
            let (va, vb) = rail(vrow, true)?;
            let cy = (ha + hb) as f32 / 2.0;
            let cx = (va + vb) as f32 / 2.0;
            let band = ((hb - ha + 1) as f32 + (vb - va + 1) as f32) / 2.0;
            let k = band * toml.knot as f32 / toml.thickness.max(1) as f32;
            Ok([cx - k / 2.0, cy - k / 2.0, k, k])
        }
        other => Err(format!("{other}: not a piece")),
    }
}

/// The piece cut from its raw picture at `cut`, at the template's authored
/// size: the edge's cut height becomes `thickness` (its width scaling with
/// it), a knot's cut becomes the `knot` square (padded clear where it
/// overhangs the picture), the ornament's cut becomes `ornament_px` tall.
pub fn cut_piece(name: &str, img: &RgbaImage, cut: [f32; 4], toml: &FrameToml, ornament_px: u32) -> Result<RgbaImage, String> {
    let [_, _, w, h] = cut;
    let (t_px, k_px) = (toml.thickness_px().max(1), toml.knot_px().max(1));
    let raw = crop_scaled(img, cut, 1.0, name)?;
    Ok(match name {
        "edge" => resized(&raw, (w * t_px as f32 / h.max(1.0)).round() as u32, t_px),
        "ornament" => resized(&raw, (w * ornament_px as f32 / h.max(1.0)).round() as u32, ornament_px.max(1)),
        "corner" | "tee" | "cross" | "sticker" => resized(&raw, k_px, k_px),
        other => return Err(format!("{other}: not a piece")),
    })
}

/// [`measure_cut`] then [`cut_piece`], the ornament at the knot's height;
/// a painted checkerboard is keyed out first.
pub fn fit_piece(name: &str, img: &RgbaImage, toml: &FrameToml) -> Result<RgbaImage, String> {
    let img = if is_opaque(img) { key_checker(img) } else { img.clone() };
    let cut = measure_cut(name, &img, toml)?;
    cut_piece(name, &img, cut, toml, toml.knot_px())
}

/// [`fit_piece`] over every piece that is there.
pub fn fit_all(pieces: &Pieces, toml: &FrameToml) -> Result<Pieces, String> {
    let fit = |name: &str, p: &Option<RgbaImage>| p.as_ref().map(|img| fit_piece(name, img, toml)).transpose();
    Ok(Pieces { edge: fit("edge", &pieces.edge)?, corner: fit("corner", &pieces.corner)?, tee: fit("tee", &pieces.tee)?, cross: fit("cross", &pieces.cross)?, ornament: fit("ornament", &pieces.ornament)?, sticker: fit("sticker", &pieces.sticker)? })
}

/// Which way a piece is turned when placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

/// The four corners of a placed piece, as the piece's own (u, v): u along
/// the authored run left to right, v outside to inside. Order: the
/// destination's top-left, top-right, bottom-right, bottom-left.
pub fn corner_uvs(side: Side, turn: Turn) -> [[f32; 2]; 4] {
    match (side, turn) {
        (Side::Top, _) => [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        (Side::Right, Turn::Rotate) => [[0.0, 1.0], [0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
        (Side::Bottom, Turn::Rotate) => [[1.0, 1.0], [0.0, 1.0], [0.0, 0.0], [1.0, 0.0]],
        (Side::Left, _) => [[1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]],
        // Mirror: the right is the left reflected, the bottom the top reflected.
        (Side::Right, Turn::Mirror) => [[1.0, 1.0], [1.0, 0.0], [0.0, 0.0], [0.0, 1.0]],
        (Side::Bottom, Turn::Mirror) => [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
    }
}

/// The knot at a window's corner: which turn of the top-left piece.
/// Rotate: top-right is 90 clockwise, and so on. Mirror: the top-right is
/// the top-left reflected across, the bottom-left reflected down, the
/// bottom-right both.
pub fn knot_corner_uvs(side: Side, turn: Turn) -> [[f32; 2]; 4] {
    match (side, turn) {
        (_, Turn::Rotate) | (Side::Top, _) | (Side::Left, _) => match side {
            Side::Top => corner_uvs(Side::Top, turn),      // top-left
            Side::Right => corner_uvs(Side::Right, turn),  // top-right
            Side::Bottom => corner_uvs(Side::Bottom, turn), // bottom-right
            Side::Left => corner_uvs(Side::Left, turn),    // bottom-left
        },
        (Side::Right, Turn::Mirror) => [[1.0, 0.0], [0.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
        (Side::Bottom, Turn::Mirror) => [[1.0, 1.0], [0.0, 1.0], [0.0, 0.0], [1.0, 0.0]],
    }
}

/// Which piece a placement draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PieceKind {
    Edge,
    Corner,
    Tee,
    Cross,
    Ornament,
    Sticker,
}

/// One piece placed on the plan, in points from the plan's origin.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub kind: PieceKind,
    /// `[x, y, w, h]`.
    pub rect: [f32; 4],
    /// The piece's (u, v) at the rect's top-left, top-right, bottom-right, bottom-left.
    pub uvs: [[f32; 2]; 4],
}

/// The pieces' sizes in points (authored pixels over [`PX_PER_PT`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PieceSizes {
    pub edge: [f32; 2],
    pub knot: f32,
    pub ornament: Option<[f32; 2]>,
    pub has_tee: bool,
    pub has_cross: bool,
    /// The junctions are stickers over full-length rails, not knot pieces.
    pub sticker: bool,
}

impl PieceSizes {
    pub fn of(pieces: &Pieces, toml: &FrameToml) -> Option<Self> {
        let edge = pieces.edge.as_ref()?;
        let sticker = pieces.sticker.is_some();
        let corner = pieces.sticker.as_ref().or(pieces.corner.as_ref())?;
        // Hydra's rule: points come from thickness against the edge's pixel height.
        let ppp = if toml.thickness > 0 { edge.height() as f32 / toml.thickness as f32 } else { PX_PER_PT as f32 };
        let pt = |px: u32| px as f32 / ppp.max(1e-6);
        Some(PieceSizes {
            edge: [pt(edge.width()), pt(edge.height())],
            knot: pt(corner.width()),
            ornament: pieces.ornament.as_ref().map(|o| [pt(o.width()), pt(o.height())]),
            has_tee: pieces.tee.is_some(),
            has_cross: pieces.cross.is_some(),
            sticker,
        })
    }
}

/// A plan: cells to frame, as a grid of `cols` by `rows` cells inside
/// `[x, y, w, h]`. One cell is a window; two across is the row; 2x2 the grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plan {
    pub rect: [f32; 4],
    pub cols: u32,
    pub rows: u32,
}

/// Where the pieces go for `plan`: edges along every run (outer and
/// dividing), tiled from each run's centre outward with symmetric end
/// cuts (or one stretched tile), ornaments once per outer run, then the
/// knots over them: corners, tees where a divider meets the outside,
/// crosses where dividers meet. Edges first so knots cover them.
pub fn layout(plan: &Plan, sizes: &PieceSizes, toml: &FrameToml) -> Vec<Placed> {
    let [px, py, pw, ph] = plan.rect;
    let t = sizes.edge[1];
    let k = sizes.knot;
    let (cols, rows) = (plan.cols.max(1), plan.rows.max(1));
    // Joint lines: the centrelines of every run.
    let xs: Vec<f32> = (0..=cols).map(|c| px + t / 2.0 + (pw - t) * c as f32 / cols as f32).collect();
    let ys: Vec<f32> = (0..=rows).map(|r| py + t / 2.0 + (ph - t) * r as f32 / rows as f32).collect();
    let mut out = Vec::new();

    // Runs: the dividers first, so their ends lie under the outer rails
    // where no tee covers the junction.
    let runs: Vec<(Side, f32, f32, f32, bool)> = {
        let mut v = Vec::new();
        for (ri, &y) in ys.iter().enumerate() {
            let outer = ri == 0 || ri == rows as usize;
            let side = if ri == rows as usize { Side::Bottom } else { Side::Top };
            v.push((side, y, xs[0], xs[cols as usize], outer));
        }
        for (ci, &x) in xs.iter().enumerate() {
            let outer = ci == 0 || ci == cols as usize;
            let side = if ci == cols as usize { Side::Right } else { Side::Left };
            v.push((side, x, ys[0], ys[rows as usize], outer));
        }
        v.sort_by_key(|run| run.4);
        v
    };
    for (side, at, from, to, outer) in &runs {
        let horizontal = matches!(side, Side::Top | Side::Bottom);
        // With stickers the rails run the window's full length, through the joints.
        let (from, to) = if sizes.sticker {
            if horizontal { (px, px + pw) } else { (py, py + ph) }
        } else {
            (*from, *to)
        };
        let (from, to) = (&from, &to);
        let len = to - from;
        let uvs = corner_uvs(*side, toml.turn);
        let place = |u0: f32, u1: f32, a: f32, b: f32| -> Placed {
            // The run's cross-section is t wide about its centreline.
            let rect = if horizontal { [a, at - t / 2.0, b - a, t] } else { [at - t / 2.0, a, t, b - a] };
            let uvs = uvs.map(|[u, v]| [u0 + u * (u1 - u0), v]);
            Placed { kind: PieceKind::Edge, rect, uvs }
        };
        match toml.edge {
            EdgeMode::Stretch => out.push(place(0.0, 1.0, *from, *to)),
            EdgeMode::Fit => {
                // The run between the knots' inner faces holds a whole number
                // of tiles, each stretched to make up the difference; under
                // each knot a part tile, which the knot hides.
                let tile = sizes.edge[0].max(0.5);
                let inner = (len - k).max(tile);
                let n = (inner / tile).round().max(1.0);
                let stretched = inner / n;
                let start = from + k / 2.0;
                out.push(place(1.0 - (k / 2.0) / stretched, 1.0, *from, start));
                for i in 0..n as i32 {
                    let a = start + i as f32 * stretched;
                    out.push(place(0.0, 1.0, a, a + stretched));
                }
                out.push(place(0.0, (k / 2.0) / stretched, to - k / 2.0, *to));
            }
            EdgeMode::Tile | EdgeMode::Mirror => {
                let tile = sizes.edge[0].max(0.5);
                let mid = (from + to) / 2.0;
                let n = (len / tile / 2.0).ceil() as i32 + 1;
                for i in -n..n {
                    let a = mid + i as f32 * tile;
                    let b = a + tile;
                    let (ca, cb) = (a.max(*from), b.min(*to));
                    if cb <= ca {
                        continue;
                    }
                    // The tile as authored runs left-to-right / top-to-bottom along the run
                    // in its own u; the turn handles which way the side reads. Mirror
                    // mode reflects every other tile so the joins match.
                    let (u0, u1) = ((ca - a) / tile, (cb - a) / tile);
                    if toml.edge == EdgeMode::Mirror && i.rem_euclid(2) == 1 {
                        out.push(place(1.0 - u0, 1.0 - u1, ca, cb));
                    } else {
                        out.push(place(u0, u1, ca, cb));
                    }
                }
            }
        }
        let wanted = match side {
            Side::Top => toml.ornament_on[0],
            Side::Right => toml.ornament_on[1],
            Side::Bottom => toml.ornament_on[2],
            Side::Left => toml.ornament_on[3],
        };
        if *outer && wanted {
            if let Some([ow, oh]) = sizes.ornament {
                let mid = (from + to) / 2.0;
                let rect = if horizontal { [mid - ow / 2.0, at - oh / 2.0, ow, oh] } else { [at - oh / 2.0, mid - ow / 2.0, oh, ow] };
                out.push(Placed { kind: PieceKind::Ornament, rect, uvs: corner_uvs(*side, toml.turn) });
            }
        }
    }

    // Knots on every joint.
    for (ri, &y) in ys.iter().enumerate() {
        for (ci, &x) in xs.iter().enumerate() {
            let top = ri == 0;
            let bottom = ri == rows as usize;
            let left = ci == 0;
            let right = ci == cols as usize;
            let rect = [x - k / 2.0, y - k / 2.0, k, k];
            let (kind, uvs) = match (top, bottom, left, right) {
                (true, _, true, _) => (PieceKind::Corner, knot_corner_uvs(Side::Top, toml.turn)),
                (true, _, _, true) => (PieceKind::Corner, knot_corner_uvs(Side::Right, toml.turn)),
                (_, true, _, true) => (PieceKind::Corner, knot_corner_uvs(Side::Bottom, toml.turn)),
                (_, true, true, _) => (PieceKind::Corner, knot_corner_uvs(Side::Left, toml.turn)),
                (true, _, _, _) => (PieceKind::Tee, corner_uvs(Side::Top, toml.turn)),
                (_, true, _, _) => (PieceKind::Tee, corner_uvs(Side::Bottom, toml.turn)),
                (_, _, true, _) => (PieceKind::Tee, corner_uvs(Side::Left, toml.turn)),
                (_, _, _, true) => (PieceKind::Tee, corner_uvs(Side::Right, toml.turn)),
                _ => (PieceKind::Cross, corner_uvs(Side::Top, toml.turn)),
            };
            if sizes.sticker {
                // One sticker over every joint, turned as the knot there would be.
                out.push(Placed { kind: PieceKind::Sticker, rect, uvs });
                continue;
            }
            let have = match kind {
                PieceKind::Tee => sizes.has_tee,
                PieceKind::Cross => sizes.has_cross,
                _ => true,
            };
            if have {
                out.push(Placed { kind, rect, uvs });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: u32, h: u32, alpha: u8) -> RgbaImage {
        RgbaImage::from_pixel(w, h, image::Rgba([100, 100, 100, alpha]))
    }

    #[test]
    fn the_toml_round_trips_and_rejects_bad_words() {
        let mut t = FrameToml { thickness: 24, knot: 36, turn: Turn::Mirror, edge: EdgeMode::Stretch, tint: Some("#c8b89a".into()), ..Default::default() };
        t.cuts.insert("corner".into(), [10.0, 20.5, 300.0, 300.0]);
        t.ornament_height = Some(30.0);
        t.ornament_on = [true, false, true, false];
        t.edge = EdgeMode::Fit;
        assert_eq!(FrameToml::parse(&t.to_toml_string()).unwrap(), t);
        let d = FrameToml::default();
        assert_eq!(FrameToml::parse(&d.to_toml_string()).unwrap(), d);
        assert_eq!(FrameToml::parse("").unwrap(), d, "every key has a default");
        assert!(FrameToml::parse("turn = \"spin\"").is_err());
        assert!(FrameToml::parse("edge = \"wrap\"").is_err());
    }

    #[test]
    fn the_validator_checks_the_squares_and_the_ratio() {
        let toml = FrameToml::default(); // 12 / 18 -> 48 px edge, 72 px knot
        let mut pieces = Pieces { edge: Some(img(96, 48, 255)), corner: Some(img(72, 72, 0)), ..Default::default() };
        assert!(validate(&toml, &pieces).is_empty(), "{:?}", validate(&toml, &pieces));
        let stuck = Pieces { edge: Some(img(96, 48, 255)), sticker: Some(img(72, 72, 0)), ..Default::default() };
        assert!(validate(&toml, &stuck).is_empty(), "a sticker stands in for the corner: {:?}", validate(&toml, &stuck));
        assert!(!exportable(&validate(&toml, &Pieces { edge: Some(img(96, 48, 255)), ..Default::default() })), "neither: an error");
        pieces.tee = Some(img(72, 80, 0));
        let p = validate(&toml, &pieces);
        assert!(p.iter().any(|p| !p.warning && p.text.contains("tee.png")), "{p:?}");
        pieces.tee = None;
        pieces.corner = Some(img(80, 80, 0));
        let p = validate(&toml, &pieces);
        assert!(p.iter().any(|p| !p.warning && p.text.contains("72 px square")), "{p:?}");
        pieces.corner = Some(img(72, 72, 255));
        pieces.ornament = Some(img(40, 90, 0));
        let p = validate(&toml, &pieces);
        assert!(p.iter().all(|p| p.warning) && p.len() == 2, "no alpha, tall ornament: warnings only: {p:?}");
        assert!(exportable(&p));
        assert!(!exportable(&validate(&toml, &Pieces::default())), "no edge: an error");
    }

    #[test]
    fn the_slicer_cuts_at_the_template_size() {
        let mut picture = img(400, 400, 255);
        for y in 0..400 {
            for x in 0..400 {
                if (40..80).contains(&x) || (40..80).contains(&y) || (320..360).contains(&x) || (320..360).contains(&y) {
                    picture.put_pixel(x, y, image::Rgba([200, 150, 50, 255]));
                } else if !(80..320).contains(&x) || !(80..320).contains(&y) {
                    picture.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
                }
            }
        }
        let toml = FrameToml::default();
        let guide = SliceGuide { joint: [60.0, 60.0], band: 40.0, knot: 60.0, knot_at: [60.0, 60.0], tile_len: 120.0, tile_offset: 0.0, ornament: Some([30.0, 30.0]) };
        let pieces = slice(&picture, &guide, &toml).unwrap();
        assert_eq!(pieces.edge.as_ref().unwrap().dimensions(), (144, 48), "120 px at 40/48 scale");
        assert_eq!(pieces.corner.as_ref().unwrap().dimensions(), (72, 72));
        assert_eq!(pieces.ornament.as_ref().unwrap().dimensions(), (36, 36));
        assert!(validate(&toml, &pieces).is_empty(), "{:?}", validate(&toml, &pieces));
        // The knot is its own size and place: a 90 px square off the joint
        // still comes out the template's 72, and one that overhangs the
        // picture's edge is padded clear, not refused.
        let own = SliceGuide { knot: 90.0, knot_at: [45.0, 45.0], ..guide.clone() };
        let padded = slice(&picture, &own, &toml).unwrap();
        let corner = padded.corner.unwrap();
        assert_eq!(corner.dimensions(), (72, 72));
        assert_eq!(corner.get_pixel(0, 0).0[3], 0, "the overhang is clear");
        let off = SliceGuide { knot_at: [-500.0, -500.0], ..guide };
        assert!(slice(&picture, &off, &toml).is_err(), "a cut wholly off the picture is refused");
        // The guess reads the band off the picture's alpha; the knot starts 1.5 bands on the joint.
        let measured = SliceGuide::measure(&picture);
        assert_eq!((measured.band, measured.joint, measured.knot, measured.knot_at), (80.0, [40.0, 40.0], 120.0, [40.0, 40.0]));
    }

    /// A model's corner piece: an L of rail on a painted checkerboard, the
    /// rails 60 px thick with a 2 px engraved line, keyed and cut to the
    /// template's 72 px square on the rails' centrelines.
    #[test]
    fn a_raw_knot_on_a_painted_checker_is_keyed_and_fitted() {
        let mut pic = RgbaImage::new(400, 400);
        for y in 0..400 {
            for x in 0..400 {
                let checker = if (x / 16 + y / 16) % 2 == 0 { 58 } else { 92 };
                let on_rail = (x >= 100 && y >= 100 && y < 160) || (y >= 100 && x >= 100 && x < 160);
                let line = on_rail && (y == 130 || x == 130);
                let px = if line { [70, 70, 70, 255] } else if on_rail { [210, 215, 225, 255] } else { [checker, checker, checker, 255] };
                pic.put_pixel(x, y, image::Rgba(px));
            }
        }
        assert!(is_opaque(&pic));
        let keyed = key_checker(&pic);
        assert_eq!(keyed.get_pixel(10, 10).0[3], 0, "checker goes");
        assert_eq!(keyed.get_pixel(130, 130).0[3], 255, "the engraved line stays");
        assert_eq!(keyed.get_pixel(300, 120).0[3], 255);
        let toml = FrameToml::default();
        let corner = fit_piece("corner", &pic, &toml).unwrap();
        assert_eq!(corner.dimensions(), (72, 72));
        // The joint is at (130, 130); the square is 90 src px centred there -> rail covers
        // from (130-30) to (130+30) in src, i.e. the middle 48 of 72 px, and the top-left
        // quarter outside both rails is clear.
        assert!(corner.get_pixel(3, 3).0[3] < 40, "outside the rails: clear");
        assert_eq!(corner.get_pixel(60, 36).0[3], 255, "on the horizontal rail");
        assert_eq!(corner.get_pixel(36, 60).0[3], 255, "on the vertical rail");
        let edge = fit_piece("edge", &pic, &toml).unwrap();
        assert_eq!(edge.height(), 48);
    }

    #[test]
    fn the_layout_tiles_from_the_centre_and_puts_knots_on_the_joints() {
        let toml = FrameToml::default();
        let sizes = PieceSizes { edge: [24.0, 12.0], knot: 18.0, ornament: None, has_tee: true, has_cross: true, sticker: false };
        let one = layout(&Plan { rect: [0.0, 0.0, 200.0, 100.0], cols: 1, rows: 1 }, &sizes, &toml);
        let corners: Vec<&Placed> = one.iter().filter(|p| p.kind == PieceKind::Corner).collect();
        assert_eq!(corners.len(), 4);
        assert_eq!(corners[0].rect, [6.0 - 9.0, 6.0 - 9.0, 18.0, 18.0], "centred on the joint, t/2 in");
        assert!(one.iter().all(|p| !matches!(p.kind, PieceKind::Tee | PieceKind::Cross)));
        // The top run: 200 - 12 = 188 pt long, tiles of 24 from the centre: symmetric end cuts.
        let top: Vec<&Placed> = one.iter().filter(|p| p.kind == PieceKind::Edge && p.rect[1] == 0.0).collect();
        let first = top.iter().min_by(|a, b| a.rect[0].total_cmp(&b.rect[0])).unwrap();
        let last = top.iter().max_by(|a, b| a.rect[0].total_cmp(&b.rect[0])).unwrap();
        assert!((first.rect[2] - last.rect[2]).abs() < 1e-3, "{} vs {}", first.rect[2], last.rect[2]);
        assert!((first.uvs[0][0] - (1.0 - last.uvs[1][0])).abs() < 1e-3, "the end cuts mirror each other");
        let grid = layout(&Plan { rect: [0.0, 0.0, 200.0, 200.0], cols: 2, rows: 2 }, &sizes, &toml);
        // The dividers' edges come before the outer rails', which cover their ends.
        let edges: Vec<&Placed> = grid.iter().filter(|p| p.kind == PieceKind::Edge).collect();
        let divider = |p: &Placed| (p.rect[1] - (100.0 - 6.0)).abs() < 1e-3 || (p.rect[0] - (100.0 - 6.0)).abs() < 1e-3;
        let last_divider = edges.iter().rposition(|p| divider(p)).unwrap();
        let first_outer = edges.iter().position(|p| !divider(p)).unwrap();
        assert!(last_divider < first_outer, "dividers {last_divider} before outer {first_outer}");
        assert_eq!(grid.iter().filter(|p| p.kind == PieceKind::Tee).count(), 4);
        assert_eq!(grid.iter().filter(|p| p.kind == PieceKind::Cross).count(), 1);
        let stretch = FrameToml { edge: EdgeMode::Stretch, ..toml.clone() };
        let one = layout(&Plan { rect: [0.0, 0.0, 200.0, 100.0], cols: 1, rows: 1 }, &sizes, &stretch);
        assert_eq!(one.iter().filter(|p| p.kind == PieceKind::Edge).count(), 4);
        // Fit: whole tiles between the knots' inner faces, every one the same length.
        let fit = FrameToml { edge: EdgeMode::Fit, ..toml.clone() };
        let one = layout(&Plan { rect: [0.0, 0.0, 200.0, 100.0], cols: 1, rows: 1 }, &sizes, &fit);
        let top: Vec<&Placed> = one.iter().filter(|p| p.kind == PieceKind::Edge && p.rect[1] == 0.0 && p.uvs[0][0] == 0.0 && p.uvs[1][0] == 1.0).collect();
        // 188 - 18 = 170 between the knots' faces; 170 / 24 = 7.08 -> 7 tiles of 24.29
        assert_eq!(top.len(), 7);
        assert!(top.iter().all(|p| (p.rect[2] - 170.0 / 7.0).abs() < 1e-3));
        assert!((top[0].rect[0] - 15.0).abs() < 1e-3, "starts at the knot's inner face");
        // The ornament only where asked.
        let sized = PieceSizes { ornament: Some([30.0, 12.0]), ..sizes };
        let some = FrameToml { ornament_on: [true, false, false, false], ..toml.clone() };
        let one = layout(&Plan { rect: [0.0, 0.0, 200.0, 100.0], cols: 1, rows: 1 }, &sized, &some);
        assert_eq!(one.iter().filter(|p| p.kind == PieceKind::Ornament).count(), 1);
        let all = layout(&Plan { rect: [0.0, 0.0, 200.0, 100.0], cols: 1, rows: 1 }, &sized, &toml);
        assert_eq!(all.iter().filter(|p| p.kind == PieceKind::Ornament).count(), 4);
        // Stickers: rails full length, a sticker on every joint, no knot pieces.
        let stuck = PieceSizes { sticker: true, ..sizes };
        let grid = layout(&Plan { rect: [0.0, 0.0, 200.0, 200.0], cols: 2, rows: 2 }, &stuck, &toml);
        assert_eq!(grid.iter().filter(|p| p.kind == PieceKind::Sticker).count(), 9);
        assert!(grid.iter().all(|p| !matches!(p.kind, PieceKind::Corner | PieceKind::Tee | PieceKind::Cross)));
        let top: Vec<&Placed> = grid.iter().filter(|p| p.kind == PieceKind::Edge && p.rect[1] == 0.0).collect();
        let (lo, hi) = top.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.rect[0]), hi.max(p.rect[0] + p.rect[2])));
        assert!((lo - 0.0).abs() < 1e-3 && (hi - 200.0).abs() < 1e-3, "the top rail spans the window: {lo}..{hi}");
    }

    #[test]
    fn the_turns_keep_the_outside_out() {
        // v = 0 is the outside. On every side the outside must face away
        // from the window: top-left corner of the dest for Top, top-right for Right...
        for turn in [Turn::Rotate, Turn::Mirror] {
            assert_eq!(corner_uvs(Side::Top, turn)[0][1], 0.0);
            assert_eq!(corner_uvs(Side::Right, turn)[1][1], 0.0, "{turn:?}: the right side's outside is its right");
            assert_eq!(corner_uvs(Side::Bottom, turn)[2][1], 0.0);
            assert_eq!(corner_uvs(Side::Left, turn)[3][1], 0.0);
        }
        // Mirror keeps handedness: u runs left to right along the bottom too.
        assert!(corner_uvs(Side::Bottom, Turn::Mirror)[0][0] < corner_uvs(Side::Bottom, Turn::Mirror)[1][0]);
        assert!(corner_uvs(Side::Bottom, Turn::Rotate)[0][0] > corner_uvs(Side::Bottom, Turn::Rotate)[1][0]);
    }
}
