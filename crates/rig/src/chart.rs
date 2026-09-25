//! The silhouette source: `scale-chart.html` -> `outlines.json`.
//!
//! The chart embeds every family silhouette as SVG path data in one
//! `const SIL = {...}` line: `{key: {d: [path, ..], baseY, ref}}`, drawn at
//! 32 px per foot with `baseY` as the ground line. Each silhouette becomes
//! closed polylines in FEET: x centred on the bounding box, y up, ground at
//! 0 -- the placement the chart's own post-pass does.
//!
//! Tracing crumbs are dropped: islands under 1% of the filled area, holes
//! under 0.1%. Winding is preserved, because the chart fills with the
//! default nonzero rule and the labeller does the same.
//!
//! A port of the original `tools/extract_outlines.py`; same rules, same
//! output (coordinates rounded to 0.001 ft).

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use crate::model::RigError;

pub const PX_PER_FT: f64 = 32.0;
/// Line segments per cubic Bezier.
const CURVE_STEPS: usize = 6;
/// Smallest island kept, as a share of the total island area.
const ISLAND_MIN: f64 = 0.01;
/// Smallest hole kept, likewise.
const HOLE_MIN: f64 = 0.001;

type P = (f64, f64);

#[derive(Deserialize)]
struct Sil {
    d: Vec<String>,
    #[serde(rename = "baseY")]
    base_y: f64,
    /// Kept as written (the chart has both `2` and `2.5`).
    #[serde(rename = "ref")]
    ref_ft: Value,
}

/// One extracted silhouette, for the import report.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartRow {
    pub key: String,
    pub kept: usize,
    pub dropped: usize,
    pub width_ft: f64,
    pub height_ft: f64,
}

/// The result of reading a chart: `outlines.json` text plus what was found.
#[derive(Debug, Clone)]
pub struct ChartImport {
    pub json: String,
    pub rows: Vec<ChartRow>,
}

impl ChartImport {
    /// "44 outlines (3 tracing crumbs dropped)".
    pub fn summary(&self) -> String {
        let dropped: usize = self.rows.iter().map(|r| r.dropped).sum();
        format!("{} outlines ({dropped} tracing crumbs dropped)", self.rows.len())
    }
}

/// Read `scale-chart.html` and produce `outlines.json`.
pub fn outlines_from_chart(html: &str) -> Result<ChartImport, RigError> {
    let line = html
        .lines()
        .find(|l| l.contains("const SIL"))
        .ok_or_else(|| RigError::Parse("no `const SIL = {...}` line: is this scale-chart.html?".into()))?;
    let start = line.find('{').ok_or_else(|| RigError::Parse("`const SIL` has no object".into()))?;
    // The object is followed by `;` and possibly more script: read one value.
    let mut values = serde_json::Deserializer::from_str(&line[start..]).into_iter::<BTreeMap<String, Sil>>();
    let sil = values
        .next()
        .ok_or_else(|| RigError::Parse("`const SIL` is empty".into()))?
        .map_err(|e| RigError::Parse(format!("const SIL: {e}")))?;

    let mut out = serde_json::Map::new();
    let mut rows = Vec::new();
    for (key, v) in sil {
        let mut subs = Vec::new();
        for d in &v.d {
            subs.extend(flatten(d).map_err(|e| RigError::Parse(format!("{key}: {e}")))?);
        }
        let areas: Vec<f64> = subs.iter().map(|s| area(s).abs()).collect();
        // Nesting depth: even = island, odd = hole.
        let depth: Vec<usize> = subs
            .iter()
            .enumerate()
            .map(|(i, s)| {
                subs.iter()
                    .enumerate()
                    .filter(|(j, o)| *j != i && areas[*j] > areas[i] && inside(s[0], o))
                    .count()
            })
            .collect();
        let total: f64 = areas.iter().zip(&depth).filter(|(_, d)| *d % 2 == 0).map(|(a, _)| a).sum();
        let keep: Vec<&Vec<P>> = subs
            .iter()
            .zip(areas.iter().zip(&depth))
            .filter(|(_, (a, d))| total > 0.0 && *a / total >= if *d % 2 == 0 { ISLAND_MIN } else { HOLE_MIN })
            .map(|(s, _)| s)
            .collect();
        if keep.is_empty() {
            return Err(RigError::Parse(format!("{key}: no silhouette left after dropping crumbs")));
        }

        let (mut min_x, mut max_x) = (f64::INFINITY, f64::NEG_INFINITY);
        for p in keep.iter().flat_map(|s| s.iter()) {
            min_x = min_x.min(p.0);
            max_x = max_x.max(p.0);
        }
        let cx = (min_x + max_x) / 2.0;
        let feet: Vec<Vec<[f64; 2]>> = keep
            .iter()
            .map(|s| s.iter().map(|&(x, y)| [round3((x - cx) / PX_PER_FT), round3((v.base_y - y) / PX_PER_FT)]).collect())
            .collect();
        let (mut fx0, mut fx1, mut fy1) = (f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for p in feet.iter().flatten() {
            fx0 = fx0.min(p[0]);
            fx1 = fx1.max(p[0]);
            fy1 = fy1.max(p[1]);
        }
        rows.push(ChartRow { key: key.clone(), kept: keep.len(), dropped: subs.len() - keep.len(), width_ft: fx1 - fx0, height_ft: fy1 });
        out.insert(key, serde_json::json!({ "ref_ft": v.ref_ft, "subpaths": feet }));
    }
    let json = serde_json::to_string(&out).map_err(|e| RigError::Encode(e.to_string()))? + "\n";
    Ok(ChartImport { json, rows })
}

fn round3(v: f64) -> f64 {
    let r = (v * 1000.0).round() / 1000.0;
    if r == 0.0 {
        0.0
    } else {
        r
    }
}

/// SVG path data -> closed polylines (M L H V C Z, absolute and relative).
fn flatten(d: &str) -> Result<Vec<Vec<P>>, String> {
    let toks = tokens(d);
    let mut i = 0;
    let (mut subs, mut cur): (Vec<Vec<P>>, Vec<P>) = (Vec::new(), Vec::new());
    let (mut x, mut y, mut sx, mut sy) = (0.0, 0.0, 0.0, 0.0);
    let mut cmd: Option<char> = None;
    let num = |i: &mut usize| -> Result<f64, String> {
        let t = toks.get(*i).ok_or("path ends mid-command")?;
        *i += 1;
        match t {
            Tok::Num(v) => Ok(*v),
            Tok::Cmd(c) => Err(format!("expected a number, found '{c}'")),
        }
    };
    while i < toks.len() {
        if let Tok::Cmd(c) = toks[i] {
            cmd = Some(c);
            i += 1;
            if c == 'Z' || c == 'z' {
                if !cur.is_empty() {
                    subs.push(std::mem::take(&mut cur));
                }
                (x, y) = (sx, sy);
                continue;
            }
        }
        let c = cmd.ok_or("path data does not start with a command")?;
        match c {
            'M' | 'm' => {
                let (mut nx, mut ny) = (num(&mut i)?, num(&mut i)?);
                if c == 'm' {
                    (nx, ny) = (nx + x, ny + y);
                }
                if !cur.is_empty() {
                    subs.push(std::mem::take(&mut cur));
                }
                cur.push((nx, ny));
                (x, y, sx, sy) = (nx, ny, nx, ny);
                // Further pairs after a moveto are linetos.
                cmd = Some(if c == 'M' { 'L' } else { 'l' });
                continue;
            }
            _ => {}
        }
        if cur.is_empty() {
            cur.push((x, y));
        }
        match c {
            'L' | 'l' => {
                let (mut nx, mut ny) = (num(&mut i)?, num(&mut i)?);
                if c == 'l' {
                    (nx, ny) = (nx + x, ny + y);
                }
                (x, y) = (nx, ny);
                cur.push((x, y));
            }
            'H' | 'h' => {
                let nx = num(&mut i)?;
                x = if c == 'h' { nx + x } else { nx };
                cur.push((x, y));
            }
            'V' | 'v' => {
                let ny = num(&mut i)?;
                y = if c == 'v' { ny + y } else { ny };
                cur.push((x, y));
            }
            'C' | 'c' => {
                let mut p = [0.0; 6];
                for v in &mut p {
                    *v = num(&mut i)?;
                }
                if c == 'c' {
                    for (k, v) in p.iter_mut().enumerate() {
                        *v += if k % 2 == 0 { x } else { y };
                    }
                }
                for k in 1..=CURVE_STEPS {
                    let t = k as f64 / CURVE_STEPS as f64;
                    let u = 1.0 - t;
                    let bez = |a: f64, b: f64, c: f64, d: f64| u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d;
                    cur.push((bez(x, p[0], p[2], p[4]), bez(y, p[1], p[3], p[5])));
                }
                (x, y) = (p[4], p[5]);
            }
            other => return Err(format!("unsupported path command '{other}'")),
        }
    }
    if !cur.is_empty() {
        subs.push(cur);
    }
    Ok(subs.into_iter().filter(|s| s.len() >= 3).collect())
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tok {
    Cmd(char),
    Num(f64),
}

/// Commands and numbers; commas and whitespace separate, and a `-` starts a
/// new number (`1-2` is two numbers), as in the Python regex.
fn tokens(d: &str) -> Vec<Tok> {
    let b = d.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        if "MmLlHhVvCcZz".contains(c) {
            out.push(Tok::Cmd(c));
            i += 1;
        } else if c == '-' || c == '.' || c.is_ascii_digit() {
            let start = i;
            i += 1;
            let mut dot = c == '.';
            while i < b.len() {
                let n = b[i] as char;
                if n.is_ascii_digit() {
                    i += 1;
                } else if n == '.' && !dot {
                    dot = true;
                    i += 1;
                } else if (n == 'e' || n == 'E') && i + 1 < b.len() {
                    let mut j = i + 1;
                    if b[j] == b'-' || b[j] == b'+' {
                        j += 1;
                    }
                    if j < b.len() && b[j].is_ascii_digit() {
                        i = j;
                        while i < b.len() && b[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                    break;
                } else {
                    break;
                }
            }
            if let Ok(v) = d[start..i].parse() {
                out.push(Tok::Num(v));
            }
        } else {
            i += 1;
        }
    }
    out
}

fn area(p: &[P]) -> f64 {
    let n = p.len();
    (0..n).map(|i| p[i].0 * p[(i + 1) % n].1 - p[(i + 1) % n].0 * p[i].1).sum::<f64>() / 2.0
}

fn inside(pt: P, poly: &[P]) -> bool {
    let (x, y) = pt;
    let n = poly.len();
    let mut c = false;
    for i in 0..n {
        let ((x1, y1), (x2, y2)) = (poly[i], poly[(i + 1) % n]);
        if (y1 > y) != (y2 > y) && x < (x2 - x1) * (y - y1) / (y2 - y1) + x1 {
            c = !c;
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Outlines;

    fn chart(sil: &str) -> String {
        format!("<html><script>\nconst W = 1;\nconst SIL = {sil};\nfunction draw() {{}}\n</script></html>")
    }

    #[test]
    fn a_square_becomes_feet_centred_on_the_ground() {
        // 64 x 96 px square, ground at y = 200: 2 ft wide, 3 ft tall.
        let html = chart(r#"{"blob": {"d": ["M100,200 L164,200 L164,104 L100,104 Z"], "baseY": 200, "ref": 3}}"#);
        let got = outlines_from_chart(&html).unwrap();
        let o = Outlines::from_json(&got.json).unwrap();
        let e = &o.entries["blob"];
        assert_eq!(e.ref_ft, 3.0);
        assert_eq!(e.bounds(), Some([-1.0, 0.0, 1.0, 3.0]));
        assert_eq!(got.rows[0], ChartRow { key: "blob".into(), kept: 1, dropped: 0, width_ft: 2.0, height_ft: 3.0 });
        assert!(got.json.contains(r#""ref_ft":3"#), "ref kept as written: {}", got.json);
    }

    #[test]
    fn crumbs_go_holes_stay_and_relative_commands_work() {
        // Big square (relative commands), a real hole in it, and a speck.
        let html = chart(
            r#"{"b": {"d": ["m0,320 h320 v-320 h-320 z", "M100,220 L100,100 L220,100 L220,220 Z", "M400,320 l2,0 l0,-2 Z"], "baseY": 320, "ref": 10}}"#,
        );
        let got = outlines_from_chart(&html).unwrap();
        assert_eq!((got.rows[0].kept, got.rows[0].dropped), (2, 1));
        assert!(got.summary().starts_with("1 outlines (1 tracing crumbs"), "{}", got.summary());
    }

    #[test]
    fn curves_are_flattened_and_numbers_split_on_minus() {
        let subs = flatten("M0,0 C10,0 10,10 0,10 L-5,5 Z").unwrap();
        assert_eq!(subs[0].len(), 1 + CURVE_STEPS + 1);
        assert_eq!(tokens("1-2.5e1.5"), [Tok::Num(1.0), Tok::Num(-25.0), Tok::Num(0.5)]);
    }

    #[test]
    fn not_a_chart_is_an_error() {
        assert!(outlines_from_chart("<html></html>").is_err());
        assert!(outlines_from_chart(&chart(r#"{"x": {"d": ["M0,0 Q1,1 2,2 Z"], "baseY": 0, "ref": 1}}"#)).is_err());
    }

    /// The import reproduces the checked-in outlines, when the chart they
    /// came from is on this machine (image-gen's, made by sil_chart.py).
    /// The checked-in file may carry entries kept by hand beyond the
    /// chart's (the old single elemental beside the eight new ones).
    #[test]
    fn matches_the_checked_in_outlines_when_the_chart_is_here() {
        let chart = std::path::Path::new(r"C:\Gemstone\image-gen\silhouettes\scale-chart.html");
        let Ok(html) = std::fs::read_to_string(chart) else {
            return;
        };
        let got = Outlines::from_json(&outlines_from_chart(&html).unwrap().json).unwrap();
        let want = Outlines::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/outlines.json")).unwrap();
        assert!(got.entries.keys().all(|k| want.entries.contains_key(k)), "every chart entry is checked in");
        for (key, g) in &got.entries {
            let w = &want.entries[key];
            assert_eq!(g.ref_ft, w.ref_ft, "{key}");
            assert_eq!(g.subpaths.len(), w.subpaths.len(), "{key}");
            for (gs, ws) in g.subpaths.iter().zip(&w.subpaths) {
                assert_eq!(gs.len(), ws.len(), "{key}");
                for (a, b) in gs.iter().zip(ws) {
                    assert!((a[0] - b[0]).abs() <= 0.0011 && (a[1] - b[1]).abs() <= 0.0011, "{key}: {a:?} vs {b:?}");
                }
            }
        }
    }
}
