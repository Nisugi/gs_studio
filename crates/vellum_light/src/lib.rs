//! Creature-field lighting for VellumFE (reference port of relight.py).
//!
//! Two lights reach the creature field:
//!   scene light  read from the backdrop (`auto_scene_light`) or authored in the
//!                Stage editor; applied to creatures and props only.
//!   phase light  dawn / day / dusk / night from the game clock
//!                (`phase_keyframes`, `default_phase_light`); applied to the
//!                backdrop (`apply_backdrop`), props and creatures.
//! Every light is used at its intensity x the player's Lighting strength
//! (`with_intensity`); gradual time of day cross-fades two pre-lit keyframes.
//!
//! Dependency-free so it can live in `core/` (Android-safe, no egui, no
//! `image` crate): everything works on straight (unmultiplied) RGBA8 byte
//! slices, which is what `image::RgbaImage::as_raw` and
//! `egui::ColorImage::from_rgba_unmultiplied` both use.
//!
//! Per creature, on a worker thread:
//! ```ignore
//! let lut = Srgb8Lut::new();
//! let (se, sd) = with_intensity(Some(&scene_light), player_strength);
//! let frame = LightFrame::measure(standing_rgba, standing_w, Some((active_rgba, active_w)), &lut);
//! apply_threads(&mut layer, layer_w, &se, &sd, &frame, YMap::canvas(layer_h, active_h), &lut, n);
//! // then each time-of-day keyframe on the scene-lit pixels, frame re-measured
//! // from the scene-lit standing base, and the keyframes cross-faded by weight.
//! ```
//! Golden outputs from relight.py must match within OKLab ΔE 0.01 (`vlight golden`).

pub mod golden;

pub const ALPHA_MIN: u8 = 32; // matches load_creature_art's bbox threshold
const LUM: [f32; 3] = [0.2126, 0.7152, 0.0722];

// ------------------------------------------------------------------ colour math
#[inline]
fn s2l(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}
#[inline]
fn l2s(c: f32) -> f32 {
    let c = c.max(0.0);
    if c <= 0.003_130_8 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// 256-entry sRGB8 -> linear table (every input pixel goes through it).
pub struct Srgb8Lut([f32; 256]);
impl Srgb8Lut {
    pub fn new() -> Self {
        let mut t = [0.0; 256];
        for (i, v) in t.iter_mut().enumerate() {
            *v = s2l(i as f32 / 255.0);
        }
        Srgb8Lut(t)
    }
}
impl Default for Srgb8Lut {
    fn default() -> Self { Self::new() }
}

#[inline]
fn lin_to_oklab(r: f32, g: f32, b: f32) -> [f32; 3] {
    let l = (0.412_221_47 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}
#[inline]
fn oklab_to_lin(lab: [f32; 3]) -> [f32; 3] {
    let [ll, a, b] = lab;
    let l = (ll + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m = (ll - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s = (ll - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    [
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    ]
}
#[inline]
fn in_gamut(c: [f32; 3]) -> bool {
    c.iter().all(|&v| (-1e-4..=1.0 + 1e-4).contains(&v))
}
/// Reduce chroma only (hold L and hue) until the colour fits sRGB — the
/// harmony generator's lch2hex rule. 14 bisection steps, like relight.py.
fn gamut_clip(lab: [f32; 3]) -> [f32; 3] {
    if in_gamut(oklab_to_lin(lab)) {
        return lab;
    }
    let c = (lab[1] * lab[1] + lab[2] * lab[2]).sqrt();
    if c < 1e-9 {
        return lab;
    }
    let (ua, ub) = (lab[1] / c, lab[2] / c);
    let (mut lo, mut hi) = (0.0f32, c);
    for _ in 0..14 {
        let mid = 0.5 * (lo + hi);
        if in_gamut(oklab_to_lin([lab[0], ua * mid, ub * mid])) { lo = mid } else { hi = mid }
    }
    [lab[0], ua * lo, ub * lo]
}

// f64 helpers for the once-per-env derivation (hex parsing, lch2hex)
fn hex_to_rgb(hex: &str) -> Option<[f64; 3]> {
    let h = hex.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok().map(|v| v as f64 / 255.0);
    Some([p(0)?, p(2)?, p(4)?])
}
fn hex_to_lch(hex: &str) -> Option<[f64; 3]> {
    let [r, g, b] = hex_to_rgb(hex)?;
    let lab = lin_to_oklab(s2l(r as f32), s2l(g as f32), s2l(b as f32));
    let (l, a, bb) = (lab[0] as f64, lab[1] as f64, lab[2] as f64);
    Some([l, a.hypot(bb), bb.atan2(a).to_degrees().rem_euclid(360.0)])
}
fn lch_to_hex(lch: [f64; 3]) -> String {
    let (l, c, h) = (lch[0], lch[1], lch[2].to_radians());
    let lab = gamut_clip([l as f32, (c * h.cos()) as f32, (c * h.sin()) as f32]);
    let lin = oklab_to_lin(lab);
    let to = |v: f32| (l2s(v).clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", to(lin[0]), to(lin[1]), to(lin[2]))
}
fn tint(hex: &str) -> [f32; 3] {
    let [r, g, b] = hex_to_rgb(hex).unwrap_or([1.0; 3]);
    let lin = [s2l(r as f32), s2l(g as f32), s2l(b as f32)];
    let y = (lin[0] * LUM[0] + lin[1] * LUM[1] + lin[2] * LUM[2]).max(1e-4);
    [lin[0] / y, lin[1] / y, lin[2] / y]
}

// ------------------------------------------------------------------ environment
/// One scene's light, as authored. Only `key` is required.
#[derive(Debug, Clone)]
pub struct LightEnv {
    pub key: String,
    pub ambient: Option<String>,
    pub ambient_offset: f32,
    pub bounce: Option<String>,
    pub fog: Option<String>,
    pub strength: f32,
    pub bounce_strength: f32,
    pub haze: f32,
    pub exposure: Option<f32>,
    pub chroma: Option<f32>,
    pub night: f32,
    /// 0 = no change .. 1 = the full look. The player's Lighting strength
    /// multiplies it (see `with_intensity`).
    pub intensity: f32,
}
impl LightEnv {
    pub fn from_key(key: &str) -> Self {
        LightEnv {
            key: key.to_string(), ambient: None, ambient_offset: 180.0, bounce: None,
            fog: None, strength: 0.85, bounce_strength: 0.35, haze: 0.0, exposure: None,
            chroma: None, night: 0.0, intensity: 1.0,
        }
    }
    /// The "no change" light: white, strength 0, exposure 1, saturation 1.
    pub fn identity() -> Self {
        LightEnv {
            ambient: Some("#ffffff".into()), bounce: Some("#ffffff".into()),
            fog: Some("#ffffff".into()), strength: 0.0, bounce_strength: 0.0,
            exposure: Some(1.0), chroma: Some(1.0), ..LightEnv::from_key("#ffffff")
        }
    }
}

/// Everything an env resolves to; build once per scene/phase change.
#[derive(Debug, Clone)]
pub struct Derived {
    pub key_tint: [f32; 3],
    pub ambient_tint: [f32; 3],
    pub bounce_tint: [f32; 3],
    pub fog_lab: [f32; 3],
    pub exposure: f32,
    pub white: f32,
    pub chroma: f32,
    pub ambient_hex: String,
    pub fog_hex: String,
    /// True = this pass changes nothing; `apply` returns the layer untouched.
    pub identity: bool,
}
impl Derived {
    pub fn new(env: &LightEnv) -> Self {
        let [l, c, h] = hex_to_lch(&env.key).unwrap_or([0.7, 0.0, 0.0]);
        let amb = env.ambient.clone().unwrap_or_else(|| {
            lch_to_hex([(l + 0.05).min(0.9), c * 0.55, (h + env.ambient_offset as f64).rem_euclid(360.0)])
        });
        let bnc = env.bounce.clone().unwrap_or_else(|| env.key.clone());
        let fog = env.fog.clone().unwrap_or_else(|| lch_to_hex([(l + 0.12).min(0.92), c * 0.35, h]));
        let exposure = env.exposure.unwrap_or(((l / 0.70).powi(3)).sqrt() as f32);
        let chroma = env.chroma.unwrap_or((0.45 + c * 6.0).clamp(0.5, 1.1) as f32);
        let [fr, fg, fb] = hex_to_rgb(&fog).unwrap_or([0.5; 3]);
        Derived {
            key_tint: tint(&env.key),
            ambient_tint: tint(&amb),
            bounce_tint: tint(&bnc),
            fog_lab: lin_to_oklab(s2l(fr as f32), s2l(fg as f32), s2l(fb as f32)),
            exposure,
            white: exposure.max(1.0),
            chroma,
            ambient_hex: amb,
            fog_hex: fog,
            identity: false,
        }
    }
    pub fn identity() -> Self {
        Derived { identity: true, ..Derived::new(&LightEnv::identity()) }
    }
}

/// Interpolate between two lights (None = no change) — used for INTENSITY
/// (`blend(None, Some(light), intensity)`) and for gradual time-of-day
/// keyframes. relight.py `blend`: strength / bounce_strength / haze / night /
/// saturation / fog lerp linearly, exposure geometrically, and a side with
/// strength 0 borrows the other side's tints so a light never fades through
/// white. Endpoints are exact.
pub fn blend(a: Option<&LightEnv>, b: Option<&LightEnv>, u: f32) -> (LightEnv, Derived) {
    let u = u.clamp(0.0, 1.0);
    let resolve = |e: Option<&LightEnv>| match e {
        None => (LightEnv::identity(), Derived::identity()),
        Some(e) => (e.clone(), Derived::new(e)),
    };
    if u <= 0.0 {
        return resolve(a);
    }
    if u >= 1.0 {
        return resolve(b);
    }
    let ea = a.cloned().unwrap_or_else(LightEnv::identity);
    let eb = b.cloned().unwrap_or_else(LightEnv::identity);
    let (da, db) = (Derived::new(&ea), Derived::new(&eb));
    let lerp = |x: f32, y: f32| (1.0 - u) * x + u * y;
    let mut env = eb.clone();
    env.strength = lerp(ea.strength, eb.strength);
    env.bounce_strength = lerp(ea.bounce_strength, eb.bounce_strength);
    env.haze = lerp(ea.haze, eb.haze);
    env.night = lerp(ea.night, eb.night);
    let mut d = db.clone();
    let pick = |ta: [f32; 3], tb: [f32; 3]| {
        let ta2 = if ea.strength > 0.0 { ta } else { tb };
        let tb2 = if eb.strength > 0.0 { tb } else { ta };
        [lerp(ta2[0], tb2[0]), lerp(ta2[1], tb2[1]), lerp(ta2[2], tb2[2])]
    };
    d.key_tint = pick(da.key_tint, db.key_tint);
    d.ambient_tint = pick(da.ambient_tint, db.ambient_tint);
    d.bounce_tint = pick(da.bounce_tint, db.bounce_tint);
    d.exposure = ((1.0 - u) * da.exposure.ln() + u * db.exposure.ln()).exp();
    d.white = d.exposure.max(1.0);
    d.chroma = lerp(da.chroma, db.chroma);
    for i in 0..3 {
        d.fog_lab[i] = lerp(da.fog_lab[i], db.fog_lab[i]);
    }
    d.identity = false;
    (env, d)
}

/// A light at its own intensity, scaled by the player's Lighting strength
/// (0..1, default 1). `None` = no change.
pub fn with_intensity(light: Option<&LightEnv>, player_strength: f32) -> (LightEnv, Derived) {
    match light {
        None => (LightEnv::identity(), Derived::identity()),
        Some(l) => blend(None, Some(l), l.intensity * player_strength),
    }
}

// ------------------------------------------------------------------ the frame
/// Per-creature constants shared by every layer drawn on that creature.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightFrame {
    /// 4th / 96th percentile OKLab L of the STANDING base (alpha >= 32).
    pub l_lo: f32,
    pub l_hi: f32,
    /// First / last content row of the ACTIVE art (standing, prone or
    /// variant image) — CreatureArt.bbox[1] * h and bbox[3] * h - 1.
    pub y_top: f32,
    pub y_bot: f32,
}

fn percentile_sorted(v: &[f32], p: f32) -> f32 {
    // numpy's default "linear" method
    let idx = p / 100.0 * (v.len() - 1) as f32;
    let (i, f) = (idx.floor() as usize, idx - idx.floor());
    if i + 1 < v.len() { v[i] + (v[i + 1] - v[i]) * f } else { v[i] }
}

impl LightFrame {
    /// Value range of the standing base. Cost: one pass + a sort of the
    /// opaque pixels' L; can ride inside load_creature_art's existing
    /// alpha-bbox pass.
    pub fn value_range(rgba: &[u8], lut: &Srgb8Lut) -> (f32, f32) {
        let mut ls: Vec<f32> = rgba
            .chunks_exact(4)
            .filter(|p| p[3] >= ALPHA_MIN)
            .map(|p| lin_to_oklab(lut.0[p[0] as usize], lut.0[p[1] as usize], lut.0[p[2] as usize])[0])
            .collect();
        if ls.is_empty() {
            return (0.0, 1.0);
        }
        ls.sort_by(f32::total_cmp);
        (percentile_sorted(&ls, 4.0), percentile_sorted(&ls, 96.0))
    }
    pub fn content_rows(rgba: &[u8], w: usize) -> (f32, f32) {
        let rows: Vec<bool> = rgba.chunks_exact(w * 4)
            .map(|row| row.chunks_exact(4).any(|p| p[3] >= ALPHA_MIN))
            .collect();
        let top = rows.iter().position(|&r| r).unwrap_or(0);
        let bot = rows.iter().rposition(|&r| r).unwrap_or(rows.len().saturating_sub(1));
        (top as f32, bot as f32)
    }
    pub fn measure(standing: &[u8], standing_w: usize, active: Option<(&[u8], usize)>,
                   lut: &Srgb8Lut) -> Self {
        let (l_lo, l_hi) = Self::value_range(standing, lut);
        let (y_top, y_bot) = match active {
            Some((a, aw)) => Self::content_rows(a, aw),
            None => Self::content_rows(standing, standing_w),
        };
        LightFrame { l_lo, l_hi, y_top, y_bot }
    }
}

/// Where a layer's rows land on the active art: row r -> offset + (r+0.5)*scale - 0.5.
#[derive(Debug, Clone, Copy)]
pub struct YMap {
    pub offset: f32,
    pub scale: f32,
}
impl YMap {
    pub const IDENTITY: YMap = YMap { offset: 0.0, scale: 1.0 };
    /// Canvas-matched wound art stretched over the drawn art.
    pub fn canvas(layer_h: usize, art_h: usize) -> Self {
        YMap { offset: 0.0, scale: art_h as f32 / layer_h as f32 }
    }
    /// Anchored wound / quad overlay: width = art_w * overlay_scale, centred
    /// on the anchor (fraction of the art).
    pub fn anchored(anchor_fy: f32, art_w: usize, art_h: usize, overlay_scale: f32,
                    layer_w: usize, layer_h: usize) -> Self {
        let s = art_w as f32 * overlay_scale / layer_w as f32;
        YMap { offset: anchor_fy * art_h as f32 - layer_h as f32 * s / 2.0, scale: s }
    }
    /// Body-wrap quad overlay stretched over the art's alpha bbox (fractions).
    pub fn body_wrap(bbox_y0: f32, bbox_y1: f32, art_h: usize, layer_h: usize) -> Self {
        YMap { offset: bbox_y0 * art_h as f32, scale: (bbox_y1 - bbox_y0) * art_h as f32 / layer_h as f32 }
    }
}

#[inline]
fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// The per-pixel function (relight.py `light_pixels`). Input and output are
/// LINEAR sRGB; `y` is the pixel's row in active-art space. Written without
/// trig: chroma scaling is a scale of (a, b), and the night pass's warmth is
/// cos(h - 40deg) = (a cos40 + b sin40) / C.
#[inline]
pub fn light_pixel(lin: [f32; 3], y: f32, env: &LightEnv, d: &Derived, fr: &LightFrame) -> [f32; 3] {
    let l_src = lin_to_oklab(lin[0], lin[1], lin[2])[0];
    // 1 litness
    let t = smoothstep((l_src - fr.l_lo) / (fr.l_hi - fr.l_lo).max(1e-3));
    // 2-3 light colour + bounce
    let yrel = ((y - fr.y_top) / (fr.y_bot - fr.y_top).max(1.0)).clamp(0.0, 1.0);
    let wb = env.bounce_strength * yrel * yrel * (1.0 - t);
    let mut out = [0.0f32; 3];
    for i in 0..3 {
        let light = d.ambient_tint[i] * (1.0 - t) + d.key_tint[i] * t;
        let light = light * (1.0 - wb) + d.bounce_tint[i] * wb;
        let light = 1.0 + env.strength * (light - 1.0);
        out[i] = lin[i] * light * d.exposure;
    }
    // 5 highlight shoulder (extended Reinhard, white = exposure)
    if d.white > 1.0 {
        let yl = (out[0] * LUM[0] + out[1] * LUM[1] + out[2] * LUM[2]).max(1e-6);
        let k = (1.0 + yl / (d.white * d.white)) / (1.0 + yl);
        out = [out[0] * k, out[1] * k, out[2] * k];
    }
    let [mut l, mut a, mut b] = lin_to_oklab(out[0].max(0.0), out[1].max(0.0), out[2].max(0.0));
    // 6 saturation
    a *= d.chroma;
    b *= d.chroma;
    // 7 night
    if env.night > 0.0 {
        let c = (a * a + b * b).sqrt();
        let (c40, s40) = (0.766_044_4f32, 0.642_787_6f32); // cos/sin 40deg
        let warm = if c > 1e-9 { ((a * c40 + b * s40) / c).max(0.0) } else { 0.0 } * smoothstep(c / 0.04);
        let k = 1.0 - 0.55 * env.night;
        a *= k;
        b *= k;
        l *= 1.0 - 0.18 * env.night * warm;
    }
    let mut lab = [l, a, b];
    // 8 haze
    if env.haze > 0.0 {
        for i in 0..3 {
            lab[i] = lab[i] * (1.0 - env.haze) + d.fog_lab[i] * env.haze;
        }
    }
    // 9 gamut clip (chroma only)
    let o = oklab_to_lin(gamut_clip(lab));
    [o[0].clamp(0.0, 1.0), o[1].clamp(0.0, 1.0), o[2].clamp(0.0, 1.0)]
}

/// Linear -> sRGB8 with correct rounding, by comparing against the 255
/// midpoints between sRGB levels (identical to round(l2s(v) * 255)).
pub struct Lin8Encoder([f32; 255]);
impl Lin8Encoder {
    pub fn new() -> Self {
        let mut t = [0.0; 255];
        for (i, v) in t.iter_mut().enumerate() {
            *v = s2l((i as f32 + 0.5) / 255.0);
        }
        Lin8Encoder(t)
    }
    #[inline]
    pub fn encode(&self, v: f32) -> u8 {
        self.0.partition_point(|&m| m < v) as u8
    }
}
impl Default for Lin8Encoder {
    fn default() -> Self { Self::new() }
}

fn apply_rows(rgba: &mut [u8], w: usize, row0: usize, env: &LightEnv, d: &Derived,
              fr: &LightFrame, ymap: YMap, lut: &Srgb8Lut, enc: &Lin8Encoder) {
    for (r, row) in rgba.chunks_exact_mut(w * 4).enumerate() {
        let y = ymap.offset + ((row0 + r) as f32 + 0.5) * ymap.scale - 0.5;
        for p in row.chunks_exact_mut(4) {
            if p[3] == 0 {
                continue;
            }
            let lin = [lut.0[p[0] as usize], lut.0[p[1] as usize], lut.0[p[2] as usize]];
            let o = light_pixel(lin, y, env, d, fr);
            p[0] = enc.encode(o[0]);
            p[1] = enc.encode(o[1]);
            p[2] = enc.encode(o[2]);
        }
    }
}

/// Light one straight-alpha RGBA8 layer in place, on `threads` threads
/// (std scoped threads, no dependency). Alpha untouched; alpha-0 pixels
/// skipped. Row r maps to active-art row  ymap.offset + (r + 0.5) * ymap.scale - 0.5.
pub fn apply_threads(rgba: &mut [u8], w: usize, env: &LightEnv, d: &Derived, fr: &LightFrame,
                     ymap: YMap, lut: &Srgb8Lut, threads: usize) {
    if d.identity {
        return;
    }
    let enc = Lin8Encoder::new();
    let h = rgba.len() / (w * 4);
    let threads = threads.clamp(1, h.max(1));
    if threads == 1 {
        return apply_rows(rgba, w, 0, env, d, fr, ymap, lut, &enc);
    }
    let rows_per = h.div_ceil(threads);
    std::thread::scope(|s| {
        for (k, chunk) in rgba.chunks_mut(rows_per * w * 4).enumerate() {
            let enc = &enc;
            s.spawn(move || apply_rows(chunk, w, k * rows_per, env, d, fr, ymap, lut, enc));
        }
    });
}

/// Single-threaded convenience.
pub fn apply(rgba: &mut [u8], w: usize, env: &LightEnv, d: &Derived, fr: &LightFrame,
             ymap: YMap, lut: &Srgb8Lut) {
    apply_threads(rgba, w, env, d, fr, ymap, lut, 1)
}

/// The scene light read from a backdrop image (relight.py `auto_scene_light`).
#[derive(Debug, Clone, PartialEq)]
pub struct SceneColours {
    pub key: String,
    pub ambient: String,
    pub bounce: String,
    /// Saturation for the creatures: 0.35 + 8 x the backdrop's mean OKLab
    /// chroma, kept to 0.55..1.0 (desert 0.98, grass 0.90, foggy swamp 0.58).
    pub chroma: f32,
}

/// Key / ambient / ground-bounce hexes from a backdrop (RGB or RGBA8,
/// `channels` = 3 or 4):
///   key     brightest 10% (OKLab L) of the ground — rows at or below 40% of the height
///   ambient the top 15% of rows (sky or ceiling)
///   bounce  the lower 40% of rows (the ground)
///   chroma  0.35 + 8 x the mean OKLab chroma of every pixel, kept to 0.55..1.0
/// Colour lightness fixed (0.72 / 0.72 / 0.66), colour chroma capped (0.09 / 0.06 / 0.09).
pub fn auto_scene_light(pixels: &[u8], w: usize, h: usize, channels: usize,
                        lut: &Srgb8Lut) -> SceneColours {
    let px = |x: usize, y: usize| {
        let i = (y * w + x) * channels;
        [lut.0[pixels[i] as usize], lut.0[pixels[i + 1] as usize], lut.0[pixels[i + 2] as usize]]
    };
    let hf = h as f64;
    // brightest 10% of the ground: 90th percentile of L (numpy "linear")
    let mut ground_l: Vec<f32> = Vec::new();
    for y in 0..h {
        if (y as f64) < 0.4 * hf { continue }
        for x in 0..w {
            let p = px(x, y);
            ground_l.push(lin_to_oklab(p[0], p[1], p[2])[0]);
        }
    }
    let mut sorted = ground_l.clone();
    sorted.sort_by(f32::total_cmp);
    let thr = percentile_sorted(&sorted, 90.0);
    let mut acc = [[0f64; 3]; 3];
    let mut n = [0f64; 3];
    let mut chroma_sum = 0f64;
    for y in 0..h {
        let yf = y as f64;
        for x in 0..w {
            let p = px(x, y);
            let lab = lin_to_oklab(p[0], p[1], p[2]);
            chroma_sum += (lab[1] as f64).hypot(lab[2] as f64);
            let mut add = |k: usize| {
                for c in 0..3 { acc[k][c] += p[c] as f64 }
                n[k] += 1.0;
            };
            if yf >= 0.4 * hf && lin_to_oklab(p[0], p[1], p[2])[0] >= thr { add(0) }
            if yf < 0.15 * hf { add(1) }
            if yf >= 0.6 * hf { add(2) }
        }
    }
    let pick = |k: usize, l_fix: f64, c_max: f64| {
        let m = |c: usize| (acc[k][c] / n[k].max(1.0)) as f32;
        let lab = lin_to_oklab(m(0), m(1), m(2));
        let (a, b) = (lab[1] as f64, lab[2] as f64);
        lch_to_hex([l_fix, a.hypot(b).min(c_max), b.atan2(a).to_degrees().rem_euclid(360.0)])
    };
    let chroma = (0.35 + 8.0 * chroma_sum / (w * h).max(1) as f64).clamp(0.55, 1.0) as f32;
    SceneColours { key: pick(0, 0.72, 0.09), ambient: pick(1, 0.72, 0.06), bounce: pick(2, 0.66, 0.09), chroma }
}

impl LightEnv {
    /// The scene light at its defaults (relight.py SCENE_DEFAULTS /
    /// resolve_scene_light with an empty [light] table): the backdrop's
    /// colours and saturation, strength 0.6, ground bounce 0.45, intensity 1.
    /// Overridden fields from the scene file replace these.
    pub fn scene_default(c: &SceneColours) -> Self {
        LightEnv {
            ambient: Some(c.ambient.clone()), bounce: Some(c.bounce.clone()),
            strength: 0.6, bounce_strength: 0.45, chroma: Some(c.chroma), intensity: 1.0,
            ..LightEnv::from_key(&c.key)
        }
    }
}

/// Phase pass on the scene backdrop (opaque RGBA8): its own value range as
/// the frame, no ground bounce (the backdrop IS the ground).
pub fn apply_backdrop(rgba: &mut [u8], w: usize, env: &LightEnv, d: &Derived,
                      lut: &Srgb8Lut, threads: usize) {
    let fr = LightFrame::measure(rgba, w, None, lut);
    let env = LightEnv { bounce_strength: 0.0, ..env.clone() };
    apply_threads(rgba, w, &env, d, &fr, YMap::IDENTITY, lut, threads);
}

// ------------------------------------------------------------------ time of day
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase { Dawn, Day, Dusk, Night }

/// Same meaning and defaults as core::elanthian_time::PhaseBoundaries.
#[derive(Debug, Clone, Copy)]
pub struct PhaseBoundaries { pub dawn: f32, pub day: f32, pub dusk: f32, pub night: f32 }
impl Default for PhaseBoundaries {
    fn default() -> Self { PhaseBoundaries { dawn: 6.0, day: 8.0, dusk: 18.0, night: 20.0 } }
}

/// Gradual schedule (relight.py `phase_keyframes`): the one or two phase
/// looks showing at a game hour (0..24, fractional) and their weights.
/// Dawn peaks mid-window (7:00 by default), dusk likewise (19:00); day and
/// night hold steady. Weights sum to 1.
pub fn phase_keyframes(hour: f32, b: PhaseBoundaries) -> Vec<(Phase, f32)> {
    let h = hour.rem_euclid(24.0);
    let dawn_pk = (b.dawn + b.day) / 2.0;
    let dusk_pk = (b.dusk + b.night) / 2.0;
    let ramp = |a: f32, z: f32| (h - a) / (z - a);
    if b.dawn <= h && h < dawn_pk { let u = ramp(b.dawn, dawn_pk); return vec![(Phase::Night, 1.0 - u), (Phase::Dawn, u)] }
    if dawn_pk <= h && h < b.day { let u = ramp(dawn_pk, b.day); return vec![(Phase::Dawn, 1.0 - u), (Phase::Day, u)] }
    if b.day <= h && h < b.dusk { return vec![(Phase::Day, 1.0)] }
    if b.dusk <= h && h < dusk_pk { let u = ramp(b.dusk, dusk_pk); return vec![(Phase::Day, 1.0 - u), (Phase::Dusk, u)] }
    if dusk_pk <= h && h < b.night { let u = ramp(dusk_pk, b.night); return vec![(Phase::Dusk, 1.0 - u), (Phase::Night, u)] }
    vec![(Phase::Night, 1.0)]
}

/// Shipped phase defaults (relight.py PHASE_DEFAULTS). Day = None = no change.
pub fn default_phase_light(p: Phase) -> Option<LightEnv> {
    match p {
        Phase::Day => None,
        Phase::Dawn => Some(LightEnv {
            ambient: Some("#9c9ed0".into()), fog: Some("#eadbe6".into()), haze: 0.10,
            strength: 0.5, bounce_strength: 0.0, exposure: Some(0.85), chroma: Some(0.9),
            intensity: 0.8, ..LightEnv::from_key("#f4b9c4") }),
        Phase::Dusk => Some(LightEnv {
            ambient: Some("#665fa6".into()), fog: Some("#e6c2c0".into()), haze: 0.06,
            strength: 0.6, bounce_strength: 0.0, exposure: Some(0.64), chroma: Some(0.9),
            intensity: 0.75, ..LightEnv::from_key("#ec9a86") }),
        Phase::Night => Some(LightEnv {
            ambient: Some("#2c3550".into()), strength: 0.9, bounce_strength: 0.0,
            exposure: Some(0.3), night: 0.85, intensity: 0.7, ..LightEnv::from_key("#56648c") }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_lit_alone_equals_layer_painted_in() {
        // A base with a value ramp, and an opaque wound layer over part of it.
        let (w, h) = (16usize, 32usize);
        let lut = Srgb8Lut::new();
        let mut base = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let v = (40 + y * 6) as u8;
                base[(y * w + x) * 4..][..4].copy_from_slice(&[v, v / 2 + 20, v / 3 + 10, 255]);
            }
        }
        let mut wound = vec![0u8; w * h * 4];
        for y in 20..28 {
            for x in 4..10 {
                wound[(y * w + x) * 4..][..4].copy_from_slice(&[150, 28, 24, 255]);
            }
        }
        let env = LightEnv { bounce: Some("#d4a070".into()), strength: 0.6, bounce_strength: 0.45,
                             chroma: Some(0.95), ..LightEnv::from_key("#e0ae7c") };
        let d = Derived::new(&env);
        let fr = LightFrame::measure(&base, w, None, &lut);
        // painted in, then lit
        let mut truth = base.clone();
        for (t, s) in truth.chunks_exact_mut(4).zip(wound.chunks_exact(4)) {
            if s[3] == 255 { t.copy_from_slice(s) }
        }
        apply(&mut truth, w, &env, &d, &fr, YMap::IDENTITY, &lut);
        // lit alone with the base's frame
        let mut lit = wound.clone();
        apply(&mut lit, w, &env, &d, &fr, YMap::canvas(h, h), &lut);
        for (t, s) in truth.chunks_exact(4).zip(lit.chunks_exact(4)) {
            if s[3] == 255 { assert_eq!(&t[..3], &s[..3]) }
        }
    }

    #[test]
    fn blend_endpoints_are_exact_and_zero_is_untouched() {
        let lut = Srgb8Lut::new();
        let night = default_phase_light(Phase::Night).unwrap();
        let (e1, d1) = blend(None, Some(&night), 1.0);
        let d_plain = Derived::new(&night);
        assert_eq!(d1.key_tint, d_plain.key_tint);
        assert_eq!(d1.exposure, d_plain.exposure);
        assert_eq!(e1.strength, night.strength);
        let (_, d0) = with_intensity(Some(&night), 0.0); // player strength 0
        assert!(d0.identity);
        let mut img: Vec<u8> = (0..64u32).flat_map(|i| [i as u8 * 3, 90, 200 - i as u8, 255]).collect();
        let before = img.clone();
        let fr = LightFrame::measure(&img, 8, None, &lut);
        apply(&mut img, 8, &night, &d0, &fr, YMap::IDENTITY, &lut);
        assert_eq!(img, before, "strength 0 must leave art byte-identical");
    }

    #[test]
    fn schedule_matches_the_spec() {
        let b = PhaseBoundaries::default();
        assert_eq!(phase_keyframes(3.0, b), vec![(Phase::Night, 1.0)]);
        assert_eq!(phase_keyframes(6.5, b), vec![(Phase::Night, 0.5), (Phase::Dawn, 0.5)]);
        assert_eq!(phase_keyframes(7.0, b), vec![(Phase::Dawn, 1.0), (Phase::Day, 0.0)]);
        assert_eq!(phase_keyframes(12.0, b), vec![(Phase::Day, 1.0)]);
        assert_eq!(phase_keyframes(18.5, b), vec![(Phase::Day, 0.5), (Phase::Dusk, 0.5)]);
        assert_eq!(phase_keyframes(19.75, b), vec![(Phase::Dusk, 0.25), (Phase::Night, 0.75)]);
        assert_eq!(phase_keyframes(21.0, b), vec![(Phase::Night, 1.0)]);
        for i in 0..480 {
            let s: f32 = phase_keyframes(i as f32 * 0.05, b).iter().map(|k| k.1).sum();
            assert!((s - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn scene_default_matches_the_reference() {
        let c = SceneColours { key: "#c69c67".into(), ambient: "#81abc6".into(), bounce: "#b68855".into(), chroma: 0.58 };
        let e = LightEnv::scene_default(&c);
        assert_eq!((e.strength, e.bounce_strength, e.chroma, e.intensity), (0.6, 0.45, Some(0.58), 1.0));
        // saturation follows the painting: flat grey hits the floor, vivid red the ceiling
        let lut = Srgb8Lut::new();
        let grey = vec![128u8; 40 * 30 * 3];
        let vivid: Vec<u8> = (0..40 * 30).flat_map(|_| [230u8, 80, 0]).collect();
        assert_eq!(auto_scene_light(&grey, 40, 30, 3, &lut).chroma, 0.55);
        assert_eq!(auto_scene_light(&vivid, 40, 30, 3, &lut).chroma, 1.0);
        assert_eq!(e.ambient.as_deref(), Some("#81abc6"));
    }

    #[test]
    fn exposure_at_or_below_one_is_not_shouldered() {
        let d = Derived::new(&LightEnv { exposure: Some(1.0), ..LightEnv::from_key("#b0b0b0") });
        assert_eq!(d.white, 1.0);
    }
}
