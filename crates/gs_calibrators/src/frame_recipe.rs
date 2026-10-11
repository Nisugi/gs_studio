//! Frame recipes over a parts library (NEW, 2026-10-11, agreed with Hydra's
//! side): a frame is not a folder of pieces but a TOML naming parts from
//! one shared library, which is what the composer (the Frames tab)
//! writes and Hydra reads. No egui here.
//!
//! The library, under the pool's `global/images/frames/`:
//!
//! ```text
//! parts/edges/<name>.png        a plain seamless rail, any size: drawn `thickness` points tall
//! parts/stickers/<name>.png     decoration with no rail in it, square-ish: drawn in the `size` square
//! parts/ornaments/<name>.png    a centrepiece, no rail: drawn `height` points tall
//! <frame>.toml                  one recipe per frame
//! ```
//!
//! A recipe:
//!
//! ```toml
//! thickness = 12
//! turn = "rotate"                 # or "mirror", for every turned piece
//!
//! [edge]
//! image = "oak_rope"
//! run = "tile"                    # tile | fit | mirror | stretch
//! inset = 3                       # points the rails sit in from the window's edge, so a
//!                                 # sticker wider than the rail centres on it with nothing
//!                                 # cut: (stickers.size - thickness) / 2
//!
//! [stickers]
//! size = 18                       # points, the square every sticker is drawn in
//! inset = 3                       # points each sticker slides from its joint toward the inside
//!                                 # (corners diagonally, tees along the stem, crosses not at all);
//!                                 # (size - thickness) / 2 puts the square flush with the outer edge
//! corner = "compass"
//! tee = "nailhead"
//! cross = "nailhead"              # any of the three left out: bare seam there
//! orient = "follow"               # how stickers turn: follow (the frame's `turn`) | upright (never turned)
//! corners = ["0", "h", "upright", "follow"]   # per corner TL, TR, BR, BL: 0 | 90 | 180 | 270 (clockwise) |
//!                                 # h | v | hv (flips) | upright | follow; left out = orient
//! tees = ["follow", "0", "0", "0"]            # per tee side top, right, bottom, left, the same words
//!
//! [[ornament]]
//! image = "leaf"
//! sides = ["top"]                 # any of top, bottom, left, right
//! place = "middle"                # middle | ends | every
//! spacing = 64                    # points, for every
//! height = 16                     # points; width follows the picture
//! outer = true                    # only the outline's runs, not the lines between cells
//! ```
//!
//! Effects, on `[edge]`, `[stickers]` (all three) or an `[[ornament]]`,
//! each optional, as inline tables:
//!
//! ```toml
//! tint = "#c8b89a"                                                        # the part multiplied by it
//! emissive = { color = "#ff6a20", tolerance = 0.3, radius = 2, strength = 1.0, pulse = 3, cast = 2 }
//!     # the part's own pixels within `tolerance` of the colour, weighted by closeness, blurred
//!     # `radius` points and ADDED over the part (a bloom from inside the art), clipped to the
//!     # part's box grown by `cast` points: cast 0 lets no light leave the part. A slot may carry several, as an array of
//!     # these tables (stained glass: one per pane colour).
//! ```
//!
//! The rails run the window's full length through every joint (outer
//! rails over dividers), the stickers over the joints, the ornaments
//! where their rules say. Parts are named, never pathed: a pack carries
//! the recipes and the parts they name.

use std::path::{Path, PathBuf};

use image::RgbaImage;

use crate::frame_template::{corner_uvs, knot_corner_uvs, EdgeMode, PieceKind, Placed, Plan, Side, Turn};

/// How a sticker at a junction is turned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Orient {
    /// As the frame's `turn` turns that junction (a corner's rotation or reflection, a tee's side).
    #[default]
    Follow,
    /// As authored, never turned.
    Upright,
    /// Clockwise degrees from as authored.
    Rot90,
    Rot180,
    Rot270,
    /// Flipped across (left-right), down (top-bottom), both.
    FlipH,
    FlipV,
    FlipHV,
}

impl Orient {
    pub const ALL: [(Orient, &'static str); 8] = [
        (Orient::Follow, "follow"),
        (Orient::Upright, "upright"),
        (Orient::Rot90, "90"),
        (Orient::Rot180, "180"),
        (Orient::Rot270, "270"),
        (Orient::FlipH, "h"),
        (Orient::FlipV, "v"),
        (Orient::FlipHV, "hv"),
    ];

    pub fn word(self) -> &'static str {
        Self::ALL.iter().find(|(o, _)| *o == self).map(|(_, w)| *w).unwrap_or("follow")
    }

    pub fn of_word(w: &str) -> Option<Orient> {
        let w = w.trim().to_ascii_lowercase();
        Self::ALL.iter().find(|(_, s)| *s == w).map(|(o, _)| *o)
    }

    /// The (u, v) at the destination's four corners for a fixed turn
    /// (`Follow` is resolved by the caller from the junction).
    pub fn uvs(self) -> Option<[[f32; 2]; 4]> {
        Some(match self {
            Orient::Follow => return None,
            Orient::Upright => [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            Orient::Rot90 => [[0.0, 1.0], [0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
            Orient::Rot180 => [[1.0, 1.0], [0.0, 1.0], [0.0, 0.0], [1.0, 0.0]],
            Orient::Rot270 => [[1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]],
            Orient::FlipH => [[1.0, 0.0], [0.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
            Orient::FlipV => [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
            Orient::FlipHV => [[1.0, 1.0], [0.0, 1.0], [0.0, 0.0], [1.0, 0.0]],
        })
    }
}

fn orients_of(v: Option<&toml::Value>) -> [Option<Orient>; 4] {
    let mut out = [None; 4];
    if let Some(a) = v.and_then(toml::Value::as_array) {
        for (i, x) in a.iter().take(4).enumerate() {
            out[i] = x.as_str().and_then(Orient::of_word);
        }
    }
    out
}

/// Where an ornament rule puts its picture along a run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Place {
    /// Once, at the run's midpoint.
    Middle,
    /// Twice, one just inside each end's sticker.
    Ends,
    /// Repeated along the run at `spacing` points, centred on the midpoint.
    Every,
}

/// A bloom from inside the art: the part's pixels near `color`, weighted
/// by closeness, blurred and added over the part.
#[derive(Debug, Clone, PartialEq)]
pub struct Emissive {
    pub color: String,
    /// How far from the colour a pixel may be and still glow, 0 to 1 of the colour cube's diagonal.
    pub tolerance: f32,
    pub radius: f32,
    pub strength: f32,
    pub pulse: Option<f32>,
    /// Points the bloom reaches inward over the window's content.
    pub cast: f32,
}

impl Default for Emissive {
    fn default() -> Self {
        Emissive { color: "#ff6a20".to_owned(), tolerance: 0.3, radius: 2.0, strength: 1.0, pulse: None, cast: 2.0 }
    }
}

/// The radii the composer offers for an emissive: one bloom texture per
/// part and setting, so few values.
pub const EMISSIVE_RADII: [f32; 7] = [1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0];

/// What a slot's part is drawn with.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Effects {
    pub tint: Option<String>,
    /// Any number; each its own bloom.
    pub emissive: Vec<Emissive>,
}

fn hex_of(v: Option<&toml::Value>) -> Option<String> {
    v.and_then(toml::Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned)
}

impl Effects {
    fn of(t: &toml::Table) -> Self {
        let mut e = Effects::default();
        e.tint = hex_of(t.get("tint"));
        // One table, or an array of them.
        let tables: Vec<&toml::Table> = match t.get("emissive") {
            Some(toml::Value::Table(em)) => vec![em],
            Some(toml::Value::Array(a)) => a.iter().filter_map(toml::Value::as_table).collect(),
            _ => Vec::new(),
        };
        for em in tables {
            let mut e2 = Emissive::default();
            if let Some(c) = hex_of(em.get("color")) {
                e2.color = c;
            }
            for (key, slot) in [("tolerance", &mut e2.tolerance), ("radius", &mut e2.radius), ("strength", &mut e2.strength), ("cast", &mut e2.cast)] {
                if let Some(v) = em.get(key).and_then(num) {
                    *slot = v;
                }
            }
            e2.pulse = em.get("pulse").and_then(num).filter(|p| *p > 0.0);
            e.emissive.push(e2);
        }
        e
    }

    fn write(&self, s: &mut String) {
        if let Some(t) = &self.tint {
            s.push_str(&format!("tint = \"{t}\"\n"));
        }
        let one = |e: &Emissive| {
            format!(
                "{{ color = \"{}\", tolerance = {}, radius = {}, strength = {}{}{} }}",
                e.color,
                e.tolerance,
                e.radius,
                e.strength,
                e.pulse.map(|p| format!(", pulse = {p}")).unwrap_or_default(),
                if e.cast > 0.0 { format!(", cast = {}", e.cast) } else { String::new() }
            )
        };
        match self.emissive.as_slice() {
            [] => {}
            [e] => s.push_str(&format!("emissive = {}\n", one(e))),
            many => s.push_str(&format!("emissive = [{}]\n", many.iter().map(one).collect::<Vec<_>>().join(", "))),
        }
    }
}

/// One ornament rule.
#[derive(Debug, Clone, PartialEq)]
pub struct OrnamentRule {
    pub image: String,
    /// Top, right, bottom, left.
    pub sides: [bool; 4],
    pub place: Place,
    pub spacing: f32,
    pub height: f32,
    /// Only the outline's runs, not the lines between cells.
    pub outer: bool,
    pub effects: Effects,
}

impl Default for OrnamentRule {
    fn default() -> Self {
        OrnamentRule { image: String::new(), sides: [true, false, false, false], place: Place::Middle, spacing: 64.0, height: 16.0, outer: true, effects: Effects::default() }
    }
}

/// A frame: which parts, where.
#[derive(Debug, Clone, PartialEq)]
pub struct Recipe {
    pub thickness: f32,
    pub turn: Turn,
    pub edge: String,
    pub run: EdgeMode,
    /// Points the rails sit in from the window's edge.
    pub edge_inset: f32,
    /// The square every sticker is drawn in, points.
    pub size: f32,
    /// Points each sticker slides from its joint toward the inside.
    pub inset: f32,
    pub corner: Option<String>,
    pub tee: Option<String>,
    pub cross: Option<String>,
    /// How stickers turn unless a junction says otherwise.
    pub orient: Orient,
    /// Per corner (TL, TR, BR, BL) and per tee side (top, right, bottom, left).
    pub corners: [Option<Orient>; 4],
    pub tees: [Option<Orient>; 4],
    pub ornaments: Vec<OrnamentRule>,
    pub edge_effects: Effects,
    /// On all three stickers.
    pub sticker_effects: Effects,
}

impl Default for Recipe {
    fn default() -> Self {
        Recipe { thickness: 12.0, turn: Turn::Rotate, edge: String::new(), run: EdgeMode::Tile, edge_inset: 0.0, size: 18.0, inset: 0.0, corner: None, tee: None, cross: None, orient: Orient::Follow, corners: [None; 4], tees: [None; 4], ornaments: Vec::new(), edge_effects: Effects::default(), sticker_effects: Effects::default() }
    }
}

fn num(v: &toml::Value) -> Option<f32> {
    v.as_float().or_else(|| v.as_integer().map(|i| i as f64)).map(|f| f as f32)
}

fn name_of(v: Option<&toml::Value>) -> Option<String> {
    v.and_then(toml::Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned)
}

fn sides_of(v: Option<&toml::Value>) -> [bool; 4] {
    let names: Vec<String> = v.and_then(toml::Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.trim().to_ascii_lowercase())).collect()).unwrap_or_default();
    ["top", "right", "bottom", "left"].map(|s| names.iter().any(|n| n == s))
}

fn sides_text(sides: [bool; 4]) -> String {
    let names: Vec<String> = ["top", "right", "bottom", "left"].iter().zip(sides).filter(|(_, on)| *on).map(|(n, _)| format!("\"{n}\"")).collect();
    format!("[{}]", names.join(", "))
}

impl Recipe {
    /// What puts the sticker square flush with the window's outer edge,
    /// given where the rails sit.
    pub fn flush_inset(&self) -> f32 {
        ((self.size - self.thickness) / 2.0 - self.edge_inset).max(0.0)
    }

    /// What sets the rails in so a sticker centred on its joint sits
    /// against the window's edge: stickers centred on the rail, nothing cut.
    pub fn centred_edge_inset(&self) -> f32 {
        ((self.size - self.thickness) / 2.0).max(0.0)
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let t: toml::Table = text.parse().map_err(|e| format!("recipe: {e}"))?;
        let mut r = Recipe::default();
        if let Some(v) = t.get("thickness").and_then(num) {
            r.thickness = v;
        }
        r.turn = match t.get("turn").and_then(toml::Value::as_str).map(str::trim) {
            Some("mirror") => Turn::Mirror,
            Some("rotate") | None => Turn::Rotate,
            Some(o) => return Err(format!("recipe: turn = \"{o}\" is neither rotate nor mirror")),
        };
        if let Some(e) = t.get("edge").and_then(toml::Value::as_table) {
            r.edge = name_of(e.get("image")).unwrap_or_default();
            r.run = match e.get("run").and_then(toml::Value::as_str).map(str::trim) {
                Some("fit") => EdgeMode::Fit,
                Some("mirror") => EdgeMode::Mirror,
                Some("stretch") => EdgeMode::Stretch,
                Some("tile") | None => EdgeMode::Tile,
                Some(o) => return Err(format!("recipe: edge.run = \"{o}\" is none of tile, fit, mirror, stretch")),
            };
            if let Some(v) = e.get("inset").and_then(num) {
                r.edge_inset = v;
            }
            r.edge_effects = Effects::of(e);
        }
        if let Some(s) = t.get("stickers").and_then(toml::Value::as_table) {
            if let Some(v) = s.get("size").and_then(num) {
                r.size = v;
            }
            if let Some(v) = s.get("inset").and_then(num) {
                r.inset = v;
            }
            r.corner = name_of(s.get("corner"));
            r.tee = name_of(s.get("tee"));
            r.cross = name_of(s.get("cross"));
            r.orient = s.get("orient").and_then(toml::Value::as_str).and_then(Orient::of_word).unwrap_or_default();
            r.corners = orients_of(s.get("corners"));
            r.tees = orients_of(s.get("tees"));
            r.sticker_effects = Effects::of(s);
        }
        for o in t.get("ornament").and_then(toml::Value::as_array).into_iter().flatten() {
            let Some(o) = o.as_table() else { continue };
            let mut rule = OrnamentRule { image: name_of(o.get("image")).unwrap_or_default(), ..Default::default() };
            if o.get("sides").is_some() {
                rule.sides = sides_of(o.get("sides"));
            }
            rule.place = match o.get("place").and_then(toml::Value::as_str).map(str::trim) {
                Some("ends") => Place::Ends,
                Some("every") => Place::Every,
                Some("middle") | None => Place::Middle,
                Some(p) => return Err(format!("recipe: ornament place = \"{p}\" is none of middle, ends, every")),
            };
            if let Some(v) = o.get("spacing").and_then(num) {
                rule.spacing = v;
            }
            if let Some(v) = o.get("height").and_then(num) {
                rule.height = v;
            }
            if let Some(v) = o.get("outer").and_then(toml::Value::as_bool) {
                rule.outer = v;
            }
            rule.effects = Effects::of(o);
            r.ornaments.push(rule);
        }
        Ok(r)
    }

    pub fn to_toml_string(&self) -> String {
        let mut s = format!(
            "thickness = {}\nturn = \"{}\"                 # or \"{}\", for every turned piece\n\n[edge]\nimage = \"{}\"\nrun = \"{}\"                    # tile | fit | mirror | stretch\n",
            self.thickness,
            if self.turn == Turn::Mirror { "mirror" } else { "rotate" },
            if self.turn == Turn::Mirror { "rotate" } else { "mirror" },
            self.edge,
            match self.run {
                EdgeMode::Tile => "tile",
                EdgeMode::Fit => "fit",
                EdgeMode::Mirror => "mirror",
                EdgeMode::Stretch => "stretch",
            },
        );
        s.push_str(&format!("inset = {}                       # points the rails sit in from the window's edge\n", self.edge_inset));
        self.edge_effects.write(&mut s);
        s.push_str(&format!("\n[stickers]\nsize = {}                       # points, the square every sticker is drawn in\ninset = {}                       # points each slides from its joint toward the inside\n", self.size, self.inset));
        for (key, v) in [("corner", &self.corner), ("tee", &self.tee), ("cross", &self.cross)] {
            match v {
                Some(n) => s.push_str(&format!("{key} = \"{n}\"\n")),
                None => s.push_str(&format!("# {key} left out: a bare seam there\n")),
            }
        }
        if self.orient != Orient::Follow {
            s.push_str(&format!("orient = \"{}\"\n", self.orient.word()));
        }
        for (key, arr) in [("corners", &self.corners), ("tees", &self.tees)] {
            if arr.iter().any(|o| o.is_some()) {
                let words: Vec<String> = arr.iter().map(|o| format!("\"{}\"", o.map(Orient::word).unwrap_or("follow"))).collect();
                s.push_str(&format!("{key} = [{}]\n", words.join(", ")));
            }
        }
        self.sticker_effects.write(&mut s);
        for o in &self.ornaments {
            s.push_str(&format!(
                "\n[[ornament]]\nimage = \"{}\"\nsides = {}\nplace = \"{}\"\n{}height = {}\nouter = {}\n",
                o.image,
                sides_text(o.sides),
                match o.place {
                    Place::Middle => "middle",
                    Place::Ends => "ends",
                    Place::Every => "every",
                },
                if o.place == Place::Every { format!("spacing = {}\n", o.spacing) } else { String::new() },
                o.height,
                o.outer
            ));
            o.effects.write(&mut s);
        }
        s
    }

    pub fn read(path: &Path) -> Result<Self, String> {
        Self::parse(&std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?)
    }

    /// Every part the recipe names, as (kind folder, name).
    pub fn parts(&self) -> Vec<(&'static str, String)> {
        let mut out = vec![("edges", self.edge.clone())];
        for s in [&self.corner, &self.tee, &self.cross].into_iter().flatten() {
            out.push(("stickers", s.clone()));
        }
        for o in &self.ornaments {
            out.push(("ornaments", o.image.clone()));
        }
        out.retain(|(_, n)| !n.is_empty());
        out.dedup();
        out
    }
}

/// The library's folders.
pub const PART_KINDS: [&str; 3] = ["edges", "stickers", "ornaments"];

/// The pool's `frames/` folder.
pub fn frames_dir() -> Option<PathBuf> {
    Some(creature_field::pool_dirs::global_images_dir().ok()?.join("frames"))
}

/// The library's `parts/<kind>/` folder.
pub fn parts_dir(kind: &str) -> Option<PathBuf> {
    Some(frames_dir()?.join("parts").join(kind))
}

/// The library's part names of one kind, sorted.
pub fn list_parts(kind: &str) -> Vec<String> {
    let Some(dir) = parts_dir(kind) else { return Vec::new() };
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("png")))
        .filter_map(|p| p.file_stem()?.to_str().map(str::to_owned))
        .collect();
    out.sort();
    out
}

/// A part's file.
pub fn part_path(kind: &str, name: &str) -> Option<PathBuf> {
    Some(parts_dir(kind)?.join(format!("{}.png", name.trim())))
}

pub fn load_part(kind: &str, name: &str) -> Option<RgbaImage> {
    image::open(part_path(kind, name)?).ok().map(|i| i.to_rgba8())
}

/// A part's raw keyed picture, `parts/raw/<name>.png`, as the ingest left
/// it; the part is a cut of it.
pub fn raw_path(name: &str) -> Option<PathBuf> {
    Some(parts_dir("raw")?.join(format!("{}.png", name.trim())))
}

pub fn load_raw(name: &str) -> Option<RgbaImage> {
    image::open(raw_path(name)?).ok().map(|i| i.to_rgba8())
}

/// The cut a part was last made with, kept beside it as
/// `parts/<kind>/<name>.toml` (`cut = [x, y, w, h]` on the raw picture).
pub fn read_cut(kind: &str, name: &str) -> Option<[f32; 4]> {
    let text = std::fs::read_to_string(part_path(kind, name)?.with_extension("toml")).ok()?;
    let t: toml::Table = text.parse().ok()?;
    let v: Vec<f32> = t.get("cut")?.as_array()?.iter().filter_map(num).collect();
    match v[..] {
        [x, y, w, h] => Some([x, y, w, h]),
        _ => None,
    }
}

/// Cut the part again from its raw at `cut` (a sticker is squared up to
/// the cut's longer side), write it and the cut beside it.
pub fn recut_part(kind: &str, name: &str, cut: [f32; 4]) -> Result<(), String> {
    let raw = load_raw(name).ok_or_else(|| format!("no raw picture for {name}"))?;
    let [x, y, w, h] = cut;
    let (w, h) = if kind == "stickers" { let s = w.max(h); (s, s) } else { (w, h) };
    let (pw, ph) = raw.dimensions();
    let (x0, y0) = (x.round().max(0.0) as u32, y.round().max(0.0) as u32);
    let (cw, ch) = ((w.round().max(1.0) as u32).min(pw.saturating_sub(x0).max(1)), (h.round().max(1.0) as u32).min(ph.saturating_sub(y0).max(1)));
    let img = image::imageops::crop_imm(&raw, x0, y0, cw, ch).to_image();
    let path = part_path(kind, name).ok_or_else(|| "no pool".to_owned())?;
    img.save(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    std::fs::write(path.with_extension("toml"), format!("cut = [{}, {}, {}, {}]   # on parts/raw/{name}.png\n", x0, y0, cw, ch)).map_err(|e| e.to_string())
}

/// The emissive bloom of `img`: each pixel's own colour times how close it
/// is to `color` (within `tolerance` of the colour cube's diagonal) times
/// its alpha, blurred `radius` pixels, as a picture whose RGB is that light
/// and whose alpha is 0: uploaded premultiplied it ADDS to what is under it.
pub fn emission_bloom(img: &RgbaImage, color: [u8; 3], tolerance: f32, radius: f32) -> RgbaImage {
    // Padded by the radius on every side, so the light that spreads past the
    // part has room and the picture is drawn on the part's box grown by the
    // radius at one to one, nothing stretched.
    let pad = radius.round().max(0.0) as u32;
    let (w, h) = (img.width() + 2 * pad, img.height() + 2 * pad);
    let reach = (tolerance.clamp(0.01, 1.0) * 441.67) as f32;
    let mut lit = RgbaImage::new(w, h);
    for (x, y, p) in img.enumerate_pixels() {
        let [r, g, b, a] = p.0;
        let d = (((r as f32 - color[0] as f32).powi(2) + (g as f32 - color[1] as f32).powi(2) + (b as f32 - color[2] as f32).powi(2)) as f32).sqrt();
        let close = (1.0 - d / reach).clamp(0.0, 1.0);
        let wgt = close * close * a as f32 / 255.0;
        lit.put_pixel(x + pad, y + pad, image::Rgba([(r as f32 * wgt) as u8, (g as f32 * wgt) as u8, (b as f32 * wgt) as u8, 255]));
    }
    // Blur the light: each channel through the alpha blur's machinery by turns.
    let mut out = RgbaImage::new(w, h);
    for c in 0..3 {
        let mut chan = RgbaImage::new(w, h);
        for (p, q) in lit.pixels().zip(chan.pixels_mut()) {
            q.0 = [0, 0, 0, p.0[c]];
        }
        let blurred = alpha_blur(&chan, radius);
        if out.dimensions() != blurred.dimensions() {
            out = RgbaImage::new(blurred.width(), blurred.height());
        }
        for (p, q) in blurred.pixels().zip(out.pixels_mut()) {
            q.0[c] = p.0[3];
        }
    }
    for q in out.pixels_mut() {
        q.0[3] = 0;
    }
    out
}

/// `#rrggbb` to bytes.
pub fn hex_rgb(hex: &str) -> Option<[u8; 3]> {
    let h = hex.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// A blur of `img`'s alpha, `radius` pixels, as a white picture with that
/// alpha: what a glow or a shadow is drawn from (Hydra does the same at
/// load). Three box blurs approximate a Gaussian; the picture is first
/// scaled down so a wide blur stays cheap.
pub fn alpha_blur(img: &RgbaImage, radius: f32) -> RgbaImage {
    let (w, h) = img.dimensions();
    let scale = (512.0 / w.max(h) as f32).min(1.0);
    let (sw, sh) = (((w as f32 * scale).round() as u32).max(1), ((h as f32 * scale).round() as u32).max(1));
    let small = if (sw, sh) == (w, h) { img.clone() } else { image::imageops::resize(img, sw, sh, image::imageops::FilterType::Triangle) };
    let mut a: Vec<f32> = small.pixels().map(|p| p.0[3] as f32 / 255.0).collect();
    let r = ((radius * scale) / 1.7).round().max(1.0) as usize;
    for _ in 0..3 {
        // horizontal then vertical box blur, each a running sum
        let mut tmp = vec![0.0f32; a.len()];
        for y in 0..sh as usize {
            let row = &a[y * sw as usize..(y + 1) * sw as usize];
            let mut sum = 0.0;
            for x in 0..sw as usize {
                let lo = x.saturating_sub(r);
                let hi = (x + r).min(sw as usize - 1);
                if x == 0 {
                    sum = row[lo..=hi].iter().sum();
                } else {
                    if x + r < sw as usize {
                        sum += row[x + r];
                    }
                    if x > r {
                        sum -= row[x - r - 1];
                    }
                }
                tmp[y * sw as usize + x] = sum / (hi - lo + 1) as f32;
            }
        }
        for x in 0..sw as usize {
            let mut sum = 0.0;
            for y in 0..sh as usize {
                let lo = y.saturating_sub(r);
                let hi = (y + r).min(sh as usize - 1);
                if y == 0 {
                    sum = (lo..=hi).map(|yy| tmp[yy * sw as usize + x]).sum();
                } else {
                    if y + r < sh as usize {
                        sum += tmp[(y + r) * sw as usize + x];
                    }
                    if y > r {
                        sum -= tmp[(y - r - 1) * sw as usize + x];
                    }
                }
                a[y * sw as usize + x] = sum / (hi - lo + 1) as f32;
            }
        }
    }
    let mut out = RgbaImage::new(sw, sh);
    for (p, v) in out.pixels_mut().zip(a) {
        p.0 = [255, 255, 255, (v * 255.0).round().min(255.0) as u8];
    }
    out
}

/// The pool's recipes, sorted: `frames/<name>.toml`.
pub fn list_recipes() -> Vec<String> {
    let Some(dir) = frames_dir() else { return Vec::new() };
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("toml")))
        // A recipe has an [edge] table; the old nine-slice sidecars beside a frame picture do not.
        .filter(|p| std::fs::read_to_string(p).is_ok_and(|t| t.contains("[edge]")))
        .filter_map(|p| p.file_stem()?.to_str().map(str::to_owned))
        .collect();
    out.sort();
    out
}

pub fn recipe_path(name: &str) -> Option<PathBuf> {
    Some(frames_dir()?.join(format!("{}.toml", name.trim())))
}

/// The pixel sizes of a recipe's parts, as the layout needs them:
/// `[w, h]` each; the composer fills it from the library.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PartSizes {
    pub edge: Option<[f32; 2]>,
    pub corner: Option<[f32; 2]>,
    pub tee: Option<[f32; 2]>,
    pub cross: Option<[f32; 2]>,
    /// Per ornament rule, in the recipe's order.
    pub ornaments: Vec<Option<[f32; 2]>>,
}

/// One part placed on the plan, in points: which library part, where, turned how.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedPart {
    pub kind: &'static str,
    pub name: String,
    pub placed: Placed,
    pub effects: Effects,
}

/// Where everything goes for `plan`: rails full length on every run
/// (dividers first, outer rails over them), the junctions' stickers, then
/// the ornaments by rule. The edge is drawn `thickness` tall; its tile
/// length follows from its picture's aspect.
pub fn layout(plan: &Plan, recipe: &Recipe, sizes: &PartSizes) -> Vec<PlacedPart> {
    // The rails sit `edge_inset` in from the window's edge.
    let e = recipe.edge_inset.max(0.0);
    let [px, py, pw, ph] = [plan.rect[0] + e, plan.rect[1] + e, (plan.rect[2] - 2.0 * e).max(1.0), (plan.rect[3] - 2.0 * e).max(1.0)];
    let t = recipe.thickness;
    let k = recipe.size;
    let (cols, rows) = (plan.cols.max(1), plan.rows.max(1));
    let xs: Vec<f32> = (0..=cols).map(|c| px + t / 2.0 + (pw - t) * c as f32 / cols as f32).collect();
    let ys: Vec<f32> = (0..=rows).map(|r| py + t / 2.0 + (ph - t) * r as f32 / rows as f32).collect();
    let mut out = Vec::new();
    let Some([ew, eh]) = sizes.edge else { return out };
    let tile = (ew / eh.max(1.0) * t).max(0.5);

    // Runs: (side, centreline, outer?), dividers first.
    let mut runs: Vec<(Side, f32, bool)> = Vec::new();
    for (ri, &y) in ys.iter().enumerate() {
        let outer = ri == 0 || ri == rows as usize;
        runs.push((if ri == rows as usize { Side::Bottom } else { Side::Top }, y, outer));
    }
    for (ci, &x) in xs.iter().enumerate() {
        let outer = ci == 0 || ci == cols as usize;
        runs.push((if ci == cols as usize { Side::Right } else { Side::Left }, x, outer));
    }
    runs.sort_by_key(|r| r.2);

    for (side, at, outer) in &runs {
        let horizontal = matches!(side, Side::Top | Side::Bottom);
        let (from, to) = if horizontal { (px, px + pw) } else { (py, py + ph) };
        let len = to - from;
        let uvs = corner_uvs(*side, recipe.turn);
        let mut place = |u0: f32, u1: f32, a: f32, b: f32| {
            let rect = if horizontal { [a, at - t / 2.0, b - a, t] } else { [at - t / 2.0, a, t, b - a] };
            out.push(PlacedPart { kind: "edges", name: recipe.edge.clone(), placed: Placed { kind: PieceKind::Edge, rect, uvs: uvs.map(|[u, v]| [u0 + u * (u1 - u0), v]) }, effects: recipe.edge_effects.clone() });
        };
        match recipe.run {
            EdgeMode::Stretch => place(0.0, 1.0, from, to),
            EdgeMode::Fit => {
                // Whole tiles between the first and last joint's stickers, part tiles beyond.
                let (j0, j1) = if horizontal { (xs[0], xs[cols as usize]) } else { (ys[0], ys[rows as usize]) };
                let inner = (j1 - j0 - k).max(tile);
                let n = (inner / tile).round().max(1.0);
                let stretched = inner / n;
                let start = j0 + k / 2.0;
                place(1.0 - (start - from) / stretched, 1.0, from, start);
                for i in 0..n as i32 {
                    let a = start + i as f32 * stretched;
                    place(0.0, 1.0, a, a + stretched);
                }
                place(0.0, (to - (start + n * stretched)) / stretched, start + n * stretched, to);
            }
            EdgeMode::Tile | EdgeMode::Mirror => {
                let mid = (from + to) / 2.0;
                let n = (len / tile / 2.0).ceil() as i32 + 1;
                for i in -n..n {
                    let a = mid + i as f32 * tile;
                    let b = a + tile;
                    let (ca, cb) = (a.max(from), b.min(to));
                    if cb > ca {
                        let (u0, u1) = ((ca - a) / tile, (cb - a) / tile);
                        // Mirror: every other tile reflected, so each join meets its own image.
                        if recipe.run == EdgeMode::Mirror && i.rem_euclid(2) == 1 {
                            place(1.0 - u0, 1.0 - u1, ca, cb);
                        } else {
                            place(u0, u1, ca, cb);
                        }
                    }
                }
            }
        }

        // Ornaments on this run, by rule.
        let side_index = match side {
            Side::Top => 0,
            Side::Right => 1,
            Side::Bottom => 2,
            Side::Left => 3,
        };
        for (i, rule) in recipe.ornaments.iter().enumerate() {
            if !rule.sides[side_index] || (rule.outer && !outer) {
                continue;
            }
            let Some(Some([ow, oh])) = sizes.ornaments.get(i) else { continue };
            let h = rule.height.max(0.5);
            let w = ow / oh.max(1.0) * h;
            let (j0, j1) = if horizontal { (xs[0], xs[cols as usize]) } else { (ys[0], ys[rows as usize]) };
            let centres: Vec<f32> = match rule.place {
                Place::Middle => vec![(from + to) / 2.0],
                Place::Ends => vec![j0 + k / 2.0 + w / 2.0, j1 - k / 2.0 - w / 2.0],
                Place::Every => {
                    let sp = rule.spacing.max(w.max(1.0));
                    let mid = (from + to) / 2.0;
                    let n = ((j1 - j0 - k) / 2.0 / sp).floor() as i32;
                    (-n..=n).map(|i| mid + i as f32 * sp).collect()
                }
            };
            for c in centres {
                let rect = if horizontal { [c - w / 2.0, at - h / 2.0, w, h] } else { [at - h / 2.0, c - w / 2.0, h, w] };
                out.push(PlacedPart { kind: "ornaments", name: rule.image.clone(), placed: Placed { kind: PieceKind::Ornament, rect, uvs: corner_uvs(*side, recipe.turn) }, effects: rule.effects.clone() });
            }
        }
    }

    // Stickers over the joints.
    for (ri, &y) in ys.iter().enumerate() {
        for (ci, &x) in xs.iter().enumerate() {
            let (top, bottom, left, right) = (ri == 0, ri == rows as usize, ci == 0, ci == cols as usize);
            // The inset slides an outer joint's sticker toward the inside.
            let dx = if left { recipe.inset } else if right { -recipe.inset } else { 0.0 };
            let dy = if top { recipe.inset } else if bottom { -recipe.inset } else { 0.0 };
            let rect = [x + dx - k / 2.0, y + dy - k / 2.0, k, k];
            // (part, how the frame's turn would turn it, which override applies)
            let (name, follow, over) = match (top, bottom, left, right) {
                (true, _, true, _) => (&recipe.corner, knot_corner_uvs(Side::Top, recipe.turn), recipe.corners[0]),
                (true, _, _, true) => (&recipe.corner, knot_corner_uvs(Side::Right, recipe.turn), recipe.corners[1]),
                (_, true, _, true) => (&recipe.corner, knot_corner_uvs(Side::Bottom, recipe.turn), recipe.corners[2]),
                (_, true, true, _) => (&recipe.corner, knot_corner_uvs(Side::Left, recipe.turn), recipe.corners[3]),
                (true, _, _, _) => (&recipe.tee, corner_uvs(Side::Top, recipe.turn), recipe.tees[0]),
                (_, true, _, _) => (&recipe.tee, corner_uvs(Side::Bottom, recipe.turn), recipe.tees[2]),
                (_, _, true, _) => (&recipe.tee, corner_uvs(Side::Left, recipe.turn), recipe.tees[3]),
                (_, _, _, true) => (&recipe.tee, corner_uvs(Side::Right, recipe.turn), recipe.tees[1]),
                _ => (&recipe.cross, corner_uvs(Side::Top, recipe.turn), None),
            };
            let uvs = over.unwrap_or(recipe.orient).uvs().unwrap_or(follow);
            if let Some(name) = name {
                out.push(PlacedPart { kind: "stickers", name: name.clone(), placed: Placed { kind: PieceKind::Sticker, rect, uvs }, effects: recipe.sticker_effects.clone() });
            }
        }
    }
    out
}

/// What the validator says of a recipe against the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub warning: bool,
    pub text: String,
}

/// The recipe's checks: its parts exist in the library; the sticker square
/// is at least thickness + 1; the edge has a rail (some opaque row) and the
/// stickers and ornaments have alpha; an ornament taller than the sticker
/// square is a warning.
pub fn validate(recipe: &Recipe, exists: impl Fn(&str, &str) -> Option<RgbaImage>) -> Vec<Problem> {
    let mut out = Vec::new();
    let error = |t: String| Problem { warning: false, text: t };
    let warn = |t: String| Problem { warning: true, text: t };
    if recipe.edge.is_empty() {
        out.push(error("no edge chosen".to_owned()));
    }
    if recipe.size < recipe.thickness + 1.0 {
        out.push(error(format!("stickers.size {} must be at least thickness + 1 ({})", recipe.size, recipe.thickness + 1.0)));
    }
    if recipe.corner.is_none() && recipe.tee.is_none() && recipe.cross.is_none() {
        out.push(warn("no stickers: every joint is a bare seam".to_owned()));
    }
    for (kind, name) in recipe.parts() {
        match exists(kind, &name) {
            None => out.push(error(format!("{kind}/{name}.png is not in the library"))),
            Some(img) => {
                if kind != "edges" && !img.pixels().any(|p| p.0[3] < 255) {
                    out.push(warn(format!("{kind}/{name}.png has no transparent pixel")));
                }
            }
        }
    }
    for o in &recipe.ornaments {
        if o.height > recipe.size {
            out.push(warn(format!("ornament {} is {} pt tall, taller than the {} pt sticker square: it reaches the stickers on a short run", o.image, o.height, recipe.size)));
        }
        if !o.sides.iter().any(|s| *s) {
            out.push(warn(format!("ornament {} has no sides: it is never drawn", o.image)));
        }
    }
    out
}

pub fn exportable(problems: &[Problem]) -> bool {
    problems.iter().all(|p| p.warning)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r##"
thickness = 12
turn = "rotate"

[edge]
image = "oak_rope"
run = "tile"

[stickers]
size = 18
corner = "compass"
tee = "nailhead"

[[ornament]]
image = "leaf"
sides = ["top"]
place = "middle"
height = 16
outer = true

[[ornament]]
image = "acorn"
sides = ["left", "right"]
place = "ends"
height = 10
tint = "#c8b89a"
emissive = { color = "#ff6a20", tolerance = 0.3, radius = 8, strength = 1.2, pulse = 3, cast = 4 }
"##;

    #[test]
    fn a_recipe_reads_and_writes_the_same() {
        let r = Recipe::parse(EXAMPLE).unwrap();
        assert_eq!((r.thickness, r.size, r.edge.as_str(), r.corner.as_deref(), r.tee.as_deref(), r.cross.clone()), (12.0, 18.0, "oak_rope", Some("compass"), Some("nailhead"), None));
        assert_eq!(r.ornaments.len(), 2);
        assert_eq!(r.ornaments[1].sides, [false, true, false, true]);
        assert_eq!(r.ornaments[1].place, Place::Ends);
        let fx = &r.ornaments[1].effects;
        assert_eq!(fx.tint.as_deref(), Some("#c8b89a"));
        assert_eq!(fx.emissive.first().map(|e| (e.tolerance, e.cast, e.pulse)), Some((0.3, 4.0, Some(3.0))));
        let two = Recipe::parse("[edge]\nimage = \"x\"\nemissive = [{ color = \"#ffd040\" }, { color = \"#ff2030\", radius = 4 }]\n").unwrap();
        assert_eq!(two.edge_effects.emissive.len(), 2);
        assert_eq!(Recipe::parse(&two.to_toml_string()).unwrap().edge_effects.emissive, two.edge_effects.emissive);
        assert_eq!(r.ornaments[0].effects, Effects::default());
        assert_eq!(Recipe::parse(&r.to_toml_string()).unwrap(), r);
        assert_eq!(r.parts(), vec![("edges", "oak_rope".to_owned()), ("stickers", "compass".to_owned()), ("stickers", "nailhead".to_owned()), ("ornaments", "leaf".to_owned()), ("ornaments", "acorn".to_owned())]);
        assert!(Recipe::parse("[[ornament]]\nplace = \"corners\"").is_err());
    }

    #[test]
    fn the_layout_runs_rails_through_the_joints_and_follows_the_rules() {
        let r = Recipe::parse(EXAMPLE).unwrap();
        let sizes = PartSizes { edge: Some([200.0, 100.0]), corner: Some([72.0, 72.0]), tee: Some([72.0, 72.0]), cross: None, ornaments: vec![Some([80.0, 40.0]), Some([20.0, 20.0])] };
        let one = layout(&Plan { rect: [0.0, 0.0, 200.0, 100.0], cols: 1, rows: 1 }, &r, &sizes);
        // Rails: the top one spans the whole window.
        let top: Vec<&PlacedPart> = one.iter().filter(|p| p.kind == "edges" && p.placed.rect[1] == 0.0).collect();
        let (lo, hi) = top.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.placed.rect[0]), hi.max(p.placed.rect[0] + p.placed.rect[2])));
        assert!((lo - 0.0).abs() < 1e-3 && (hi - 200.0).abs() < 1e-3);
        assert!((top[0].placed.rect[3] - 12.0).abs() < 1e-3, "the rail is thickness tall");
        // Four corner stickers, 18 pt squares on the joints.
        let stickers: Vec<&PlacedPart> = one.iter().filter(|p| p.kind == "stickers").collect();
        assert_eq!(stickers.len(), 4);
        assert!(stickers.iter().all(|s| s.name == "compass" && s.placed.rect[2] == 18.0));
        // Flush: the top-left sticker's square starts at the window's corner.
        let mut flush = r.clone();
        flush.inset = flush.flush_inset();
        assert_eq!(flush.inset, 3.0);
        let one = layout(&Plan { rect: [0.0, 0.0, 200.0, 100.0], cols: 1, rows: 1 }, &flush, &sizes);
        let tl = one.iter().filter(|p| p.kind == "stickers").min_by(|a, b| a.placed.rect[0].total_cmp(&b.placed.rect[0])).unwrap();
        assert_eq!([tl.placed.rect[0], tl.placed.rect[1]], [0.0, 0.0]);
        // Upright stickers: every corner as authored; one corner overridden to a flip.
        let mut up = r.clone();
        up.orient = Orient::Upright;
        up.corners[2] = Some(Orient::FlipH);
        let one = layout(&Plan { rect: [0.0, 0.0, 200.0, 100.0], cols: 1, rows: 1 }, &up, &sizes);
        let st: Vec<&PlacedPart> = one.iter().filter(|p| p.kind == "stickers").collect();
        let tl = st.iter().find(|p| p.placed.rect[0] < 50.0 && p.placed.rect[1] < 50.0).unwrap();
        let br = st.iter().find(|p| p.placed.rect[0] > 150.0 && p.placed.rect[1] > 50.0).unwrap();
        assert_eq!(tl.placed.uvs, Orient::Upright.uvs().unwrap());
        assert_eq!(br.placed.uvs, Orient::FlipH.uvs().unwrap());
        assert_eq!(Recipe::parse(&up.to_toml_string()).unwrap(), up, "orient and corners round-trip");
        // The rails set in instead: the sticker stays on its joint, which now sits k/2 in, so it is flush too.
        let mut centred = r.clone();
        centred.edge_inset = centred.centred_edge_inset();
        assert_eq!(centred.edge_inset, 3.0);
        assert_eq!(centred.flush_inset(), 0.0, "with the rails in, no sticker inset is needed");
        let one = layout(&Plan { rect: [0.0, 0.0, 200.0, 100.0], cols: 1, rows: 1 }, &centred, &sizes);
        let tl = one.iter().filter(|p| p.kind == "stickers").min_by(|a, b| a.placed.rect[0].total_cmp(&b.placed.rect[0])).unwrap();
        assert_eq!([tl.placed.rect[0], tl.placed.rect[1]], [0.0, 0.0]);
        let top_rail = one.iter().find(|p| p.kind == "edges" && (p.placed.rect[1] - 3.0).abs() < 1e-3).expect("the top rail sits 3 in");
        assert!((top_rail.placed.rect[1] + top_rail.placed.rect[3] / 2.0 - 9.0).abs() < 1e-3, "its centreline through the sticker's centre");
        // The leaf once on top, 16 tall, 32 wide; the acorns at both ends of left and right: four.
        let leaves: Vec<&PlacedPart> = one.iter().filter(|p| p.name == "leaf").collect();
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaves[0].placed.rect[3], 16.0);
        assert_eq!(leaves[0].placed.rect[2], 32.0);
        assert_eq!(one.iter().filter(|p| p.name == "acorn").count(), 4);
        // A grid: tees on the outer joints, nothing at the centre (no cross named), the leaf only on the outer top.
        let grid = layout(&Plan { rect: [0.0, 0.0, 200.0, 200.0], cols: 2, rows: 2 }, &r, &sizes);
        assert_eq!(grid.iter().filter(|p| p.name == "nailhead").count(), 4);
        assert_eq!(grid.iter().filter(|p| p.kind == "stickers").count(), 8);
        assert_eq!(grid.iter().filter(|p| p.name == "leaf").count(), 1);
        // Mirror: the tiles either side of the centre join meet with matching u.
        let mut mirror = r.clone();
        mirror.run = EdgeMode::Mirror;
        let one = layout(&Plan { rect: [0.0, 0.0, 200.0, 100.0], cols: 1, rows: 1 }, &mirror, &sizes);
        let top: Vec<&PlacedPart> = one.iter().filter(|p| p.kind == "edges" && p.placed.rect[1] == 0.0).collect();
        let left_of_mid = top.iter().find(|p| (p.placed.rect[0] + p.placed.rect[2] - 100.0).abs() < 1e-3).unwrap();
        let right_of_mid = top.iter().find(|p| (p.placed.rect[0] - 100.0).abs() < 1e-3).unwrap();
        assert_eq!(left_of_mid.placed.uvs[1][0], right_of_mid.placed.uvs[0][0], "the join's two sides show the same column");
        // Every 40 pt along the top: the rule's spacing.
        let mut every = r.clone();
        every.ornaments = vec![OrnamentRule { image: "leaf".into(), sides: [true, false, false, false], place: Place::Every, spacing: 40.0, height: 10.0, outer: true, effects: Effects::default() }];
        let one = layout(&Plan { rect: [0.0, 0.0, 200.0, 100.0], cols: 1, rows: 1 }, &every, &PartSizes { ornaments: vec![Some([80.0, 40.0])], ..sizes.clone() });
        let leaves: Vec<f32> = one.iter().filter(|p| p.name == "leaf").map(|p| p.placed.rect[0] + p.placed.rect[2] / 2.0).collect();
        assert_eq!(leaves.len(), 9, "{leaves:?}");
        assert!(leaves.contains(&100.0));
    }

    #[test]
    fn the_bloom_lights_only_the_pixels_near_the_colour() {
        let mut img = RgbaImage::from_pixel(64, 16, image::Rgba([30, 30, 30, 255]));
        for x in 24..40 {
            img.put_pixel(x, 8, image::Rgba([255, 106, 32, 255]));
        }
        let bloom = emission_bloom(&img, [255, 106, 32], 0.3, 3.0);
        assert_eq!(bloom.dimensions(), (70, 22), "padded by the radius each side");
        assert!(bloom.pixels().all(|p| p.0[3] == 0), "alpha 0: additive");
        assert!(bloom.get_pixel(35, 11).0[0] > 60, "the lava line glows red");
        assert_eq!(bloom.get_pixel(5, 5).0, [0, 0, 0, 0], "the dark rock does not");
        assert_eq!(hex_rgb("#ff6a20"), Some([255, 106, 32]));
    }

    #[test]
    fn the_validator_wants_parts_that_exist_and_a_big_enough_square() {
        let mut r = Recipe::parse(EXAMPLE).unwrap();
        let lib = |kind: &str, name: &str| ((kind, name) != ("ornaments", "acorn")).then(|| RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 0])));
        let p = validate(&r, lib);
        assert!(p.iter().any(|p| !p.warning && p.text.contains("acorn")), "{p:?}");
        r.size = 12.0;
        let p = validate(&r, lib);
        assert!(p.iter().any(|p| !p.warning && p.text.contains("stickers.size")), "{p:?}");
        assert!(!exportable(&p));
    }
}
