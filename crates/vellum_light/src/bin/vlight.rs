//! vlight golden <dir>   compare against relight.py golden cases (ΔE report)
//! vlight bench          timings at creature-art sizes
use std::time::Instant;
use vellum_light::*;
use vellum_light::golden;

fn bench() {
    let lut = Srgb8Lut::new();
    let env = LightEnv { bounce: Some("#d4a070".into()), strength: 0.6, bounce_strength: 0.45,
                         chroma: Some(0.95), ..LightEnv::from_key("#e0ae7c") };
    let night = LightEnv { ambient: Some("#2c3550".into()), strength: 0.9, night: 0.85,
                           exposure: Some(0.3), bounce_strength: 0.15, ..LightEnv::from_key("#56648c") };
    let swamp = LightEnv { ambient: Some("#6f7564".into()), bounce: Some("#6d6a52".into()),
                           strength: 0.9, haze: 0.14, chroma: Some(0.62), ..LightEnv::from_key("#9c9d86") };
    // Sizes: this demo's cut sprite; the 32 px/ft template canvas for a 6 ft
    // and a 20 ft creature; a large painted sprite.
    for &(w, h, label) in &[(159usize, 281usize, "159x281 (demo sprite)"),
                            (128, 224, "128x224 (6 ft @ 32 px/ft)"),
                            (448, 704, "448x704 (20 ft @ 32 px/ft)"),
                            (1024, 1024, "1024x1024 (large painted)")] {
        let mut img = vec![0u8; w * h * 4];
        let mut seed = 12345u32;
        for p in img.chunks_exact_mut(4) {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            p.copy_from_slice(&[(seed >> 24) as u8, (seed >> 16) as u8, (seed >> 8) as u8, 255]);
        }
        // real sprites are ~45-50% transparent: clear every other 2x2 block
        for (i, p) in img.chunks_exact_mut(4).enumerate() {
            let (x, y) = (i % w, i / w);
            if ((x / 2) + (y / 2)) % 2 == 0 { p[3] = 0 }
        }
        let t0 = Instant::now();
        let fr = LightFrame::measure(&img, w, None, &lut);
        let t_measure = t0.elapsed();
        for (name, e) in [("desert", &env), ("swamp", &swamp), ("night", &night)] {
            let d = Derived::new(e);
            let mut buf = img.clone();
            let reps = 5;
            let mut per = [0f64; 2];
            for (j, th) in [1usize, 4].iter().enumerate() {
                let t = Instant::now();
                for _ in 0..reps {
                    buf.copy_from_slice(&img);
                    apply_threads(&mut buf, w, e, &d, &fr, YMap::IDENTITY, &lut, *th);
                }
                per[j] = t.elapsed().as_secs_f64() * 1000.0 / reps as f64;
            }
            println!("{label:28} {name:7} apply 1 thread {:7.2} ms ({:.0} ns/opaque px) | 4 threads {:6.2} ms | measure {:.2} ms",
                     per[0], per[0] * 1e6 / (w * h / 2) as f64, per[1], t_measure.as_secs_f64() * 1000.0);
        }
    }
}

/// Phase pass on an opaque 880x470 backdrop (the size the Stage editor asks
/// artists to author), using the desert screenshot's pixels resized by rows.
fn bench_backdrop() {
    let lut = Srgb8Lut::new();
    let (w, h) = (880usize, 470usize);
    let mut img = vec![0u8; w * h * 4];
    let mut seed = 7u32;
    for (i, p) in img.chunks_exact_mut(4).enumerate() {
        // smooth painted-looking gradients plus mild noise (not worst-case random)
        let (x, y) = ((i % w) as f32, (i / w) as f32);
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let n = ((seed >> 24) as f32 / 255.0 - 0.5) * 20.0;
        p[0] = (150.0 + 70.0 * (x / 300.0).sin() + n).clamp(0.0, 255.0) as u8;
        p[1] = (120.0 + 50.0 * (y / 90.0).cos() + n).clamp(0.0, 255.0) as u8;
        p[2] = (90.0 + 60.0 * ((x + y) / 200.0).sin() + n).clamp(0.0, 255.0) as u8;
        p[3] = 255;
    }
    for p in [Phase::Dawn, Phase::Dusk, Phase::Night] {
        let light = default_phase_light(p);
        let (e, d) = with_intensity(light.as_ref(), 1.0);
        for th in [1usize, 4] {
            let reps = 5;
            let t = Instant::now();
            for _ in 0..reps {
                let mut buf = img.clone();
                apply_backdrop(&mut buf, w, &e, &d, &lut, th);
            }
            println!("backdrop 880x470 {:?} {} thread(s): {:.1} ms", p, th, t.elapsed().as_secs_f64() * 1000.0 / reps as f64);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("golden") => {
            golden::run(args.get(2).map(String::as_str).unwrap_or("golden"));
        }
        Some("bench") => bench(),
        Some("bench-backdrop") => bench_backdrop(),
        _ => eprintln!("usage: vlight golden <dir> | vlight bench | vlight bench-backdrop"),
    }
}
