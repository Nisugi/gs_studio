//! The golden-case runner (`vlight golden`, and the `golden` test): compares
//! this port against outputs of the Python reference, relight.py.

use std::time::Instant;

use crate::*;

pub fn parse_case(txt: &str) -> (usize, usize, LightEnv) {
    let mut w = 0;
    let mut h = 0;
    let mut env = LightEnv::from_key("#ffffff");
    for line in txt.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        let opt = |v: &str| (!v.is_empty() && v != "None").then(|| v.to_string());
        let f = |v: &str| v.parse::<f32>().unwrap();
        match k {
            "w" => w = v.parse().unwrap(),
            "h" => h = v.parse().unwrap(),
            "key" => env.key = v.to_string(),
            "ambient" => env.ambient = opt(v),
            "bounce" => env.bounce = opt(v),
            "fog" => env.fog = opt(v),
            "ambient_offset" => env.ambient_offset = f(v),
            "strength" => env.strength = f(v),
            "bounce_strength" => env.bounce_strength = f(v),
            "haze" => env.haze = f(v),
            "exposure" => env.exposure = opt(v).map(|s| f(&s)),
            "chroma" => env.chroma = opt(v).map(|s| f(&s)),
            "night" => env.night = f(v),
            "intensity" => env.intensity = f(v),
            _ => {}
        }
    }
    (w, h, env)
}

fn oklab8(p: &[u8]) -> [f64; 3] {
    let s = |c: u8| {
        let c = c as f64 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let (r, g, b) = (s(p[0]), s(p[1]), s(p[2]));
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let q = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    [0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * q,
     1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * q,
     0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * q]
}

fn kv(txt: &str, key: &str) -> Option<String> {
    txt.lines().find_map(|l| l.split_once('=').filter(|(k, _)| k.trim() == key).map(|(_, v)| v.trim().to_string()))
}

/// Scene pass, then the clock's time-of-day keyframes cross-faded (route A),
/// each light at intensity x player strength — relight.py's CLI pipeline.
fn pipeline(input: &[u8], w: usize, scene: &LightEnv, hour: f32, strength: f32, lut: &Srgb8Lut) -> Vec<u8> {
    let (se, sd) = with_intensity(Some(scene), strength);
    let mut lit = input.to_vec();
    let fr1 = LightFrame::measure(input, w, None, lut);
    apply(&mut lit, w, &se, &sd, &fr1, YMap::IDENTITY, lut);
    let fr2 = LightFrame::measure(&lit, w, None, lut);
    let mut acc = vec![0f32; input.len()];
    for (phase, wgt) in phase_keyframes(hour, PhaseBoundaries::default()) {
        let light = default_phase_light(phase);
        let (pe, pd) = with_intensity(light.as_ref(), strength);
        let mut k = lit.clone();
        apply(&mut k, w, &pe, &pd, &fr2, YMap::IDENTITY, lut);
        for (a, v) in acc.iter_mut().zip(k.iter()) { *a += wgt * *v as f32 }
    }
    let mut out = input.to_vec();
    for (i, o) in out.iter_mut().enumerate() {
        if i % 4 != 3 { *o = acc[i].round() as u8 }
    }
    out
}

/// Run every golden case under `dir`; prints one line per case and
/// returns the worst max OKLab dE (f64::MAX for an auto-colour mismatch).
pub fn run(dir: &str) -> f64 {
    let lut = Srgb8Lut::new();
    let mut names: Vec<_> = std::fs::read_dir(dir).unwrap()
        .filter_map(|e| e.ok()).map(|e| e.path())
        .filter(|p| p.is_dir()).collect();
    names.sort();
    let mut worst = 0f64;
    for case in names {
        let txt = std::fs::read_to_string(case.join("case.txt")).unwrap();
        let label = case.file_name().unwrap().to_string_lossy().to_string();
        match kv(&txt, "mode").as_deref() {
            Some("auto") => {
                let (w, h): (usize, usize) = (kv(&txt, "w").unwrap().parse().unwrap(), kv(&txt, "h").unwrap().parse().unwrap());
                let px = std::fs::read(case.join("input.rgb")).unwrap();
                let got = auto_scene_light(&px, w, h, 3, &lut);
                let close = |a: &str, b: &str| (0..3).all(|i| {
                    let p = |s: &str| i32::from_str_radix(&s[1 + 2 * i..3 + 2 * i], 16).unwrap();
                    (p(a) - p(b)).abs() <= 1
                });
                let want_c: f32 = kv(&txt, "chroma").unwrap().parse().unwrap();
                let ok = close(&got.key, &kv(&txt, "key").unwrap()) && close(&got.ambient, &kv(&txt, "ambient").unwrap())
                    && close(&got.bounce, &kv(&txt, "bounce").unwrap()) && (got.chroma - want_c).abs() <= 0.005;
                println!("{label:28} auto scene light {} {} {} sat {:.4} vs python {} {} {} sat {} -> {}", got.key, got.ambient, got.bounce, got.chroma,
                         kv(&txt, "key").unwrap(), kv(&txt, "ambient").unwrap(), kv(&txt, "bounce").unwrap(), want_c,
                         if ok { "match" } else { "MISMATCH" });
                if !ok { worst = f64::MAX }
                continue;
            }
            Some("pipeline") => {
                let scene_txt: String = txt.lines().filter_map(|l| l.strip_prefix("scene.")).collect::<Vec<_>>().join("\n");
                let (_, _, scene) = parse_case(&scene_txt);
                let w: usize = kv(&txt, "w").unwrap().parse().unwrap();
                let hour: f32 = kv(&txt, "hour").unwrap().parse().unwrap();
                let st: f32 = kv(&txt, "strength").unwrap().parse().unwrap();
                let input = std::fs::read(case.join("input.rgba")).unwrap();
                let expected = std::fs::read(case.join("expected.rgba")).unwrap();
                let out = pipeline(&input, w, &scene, hour, st, &lut);
                let (mut sum, mut max, mut n) = (0f64, 0f64, 0usize);
                for (a, b) in out.chunks_exact(4).zip(expected.chunks_exact(4)) {
                    if a[3] == 0 { continue }
                    let (x, y) = (oklab8(a), oklab8(b));
                    let e = ((x[0] - y[0]).powi(2) + (x[1] - y[1]).powi(2) + (x[2] - y[2]).powi(2)).sqrt();
                    sum += e; max = max.max(e); n += 1;
                }
                worst = worst.max(max);
                println!("{label:28} mean ΔE {:.5}  max ΔE {:.5}", sum / n as f64, max);
                continue;
            }
            _ => {}
        }
        let (w, _h, env) = parse_case(&txt);
        let input = std::fs::read(case.join("input.rgba")).unwrap();
        let expected = std::fs::read(case.join("expected.rgba")).unwrap();
        let d = Derived::new(&env);
        let fr = LightFrame::measure(&input, w, None, &lut);
        let mut out = input.clone();
        let t = Instant::now();
        let reps = 20;
        for _ in 0..reps {
            out.copy_from_slice(&input);
            apply(&mut out, w, &env, &d, &fr, YMap::IDENTITY, &lut);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / reps as f64;
        let opaque = input.chunks_exact(4).filter(|p| p[3] > 0).count();
        eprintln!("  timing {:24} {:6.2} ms/layer, {:4.0} ns/opaque px (1 thread)",
                  case.file_name().unwrap().to_string_lossy(), ms, ms * 1e6 / opaque as f64);
        let (mut sum, mut max, mut n, mut exact) = (0f64, 0f64, 0usize, 0usize);
        for (a, b) in out.chunks_exact(4).zip(expected.chunks_exact(4)) {
            if a[3] == 0 { continue }
            let (x, y) = (oklab8(a), oklab8(b));
            let e = ((x[0] - y[0]).powi(2) + (x[1] - y[1]).powi(2) + (x[2] - y[2]).powi(2)).sqrt();
            sum += e; max = max.max(e); n += 1;
            if a[..3] == b[..3] { exact += 1 }
        }
        worst = worst.max(max);
        println!("{:28} mean ΔE {:.5}  max ΔE {:.5}  byte-identical {:5.1}%  (ambient {} fog {})",
                 case.file_name().unwrap().to_string_lossy(), sum / n as f64, max,
                 100.0 * exact as f64 / n as f64, d.ambient_hex, d.fog_hex);
    }
    println!("worst max ΔE {:.5} -> {}", worst, if worst <= 0.01 { "PASS (<= 0.01)" } else { "FAIL" });
    worst
}

