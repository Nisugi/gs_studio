//! relight_golden: the GPU relight (`relight_gpu`) against vellum_light's
//! golden cases -- relight.py's own outputs -- by the same measure its CPU
//! port is held to: the worst OKLab dE over every pixel shown <= 0.01.
//!
//!   cargo run --release -p gs_field_egui --example relight_golden [-- <golden dir>]
//!
//! Single-light cases test the kernel alone; pipeline cases the scene pass,
//! the frame measured again on its bytes, and the time-of-day keyframes
//! cross-faded. The auto-colour cases are CPU-only and skipped.

use std::path::PathBuf;

use egui_wgpu::wgpu;
use gs_field_egui::relight_gpu::{relight_pixels, Pass, Plan};
use vellum_light::golden::parse_case;
use vellum_light::{default_phase_light, phase_keyframes, with_intensity, Derived, PhaseBoundaries};

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    let mut f = std::pin::pin!(f);
    loop {
        if let std::task::Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}

fn kv(txt: &str, key: &str) -> Option<String> {
    txt.lines().find_map(|l| l.split_once('=').filter(|(k, _)| k.trim() == key).map(|(_, v)| v.trim().to_string()))
}

fn oklab8(p: &[u8]) -> [f64; 3] {
    let s = |c: u8| {
        let c = c as f64 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = (s(p[0]), s(p[1]), s(p[2]));
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let q = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    [
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * q,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * q,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * q,
    ]
}

fn main() {
    let dir = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../vellum_light/golden")));
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() })).expect("an adapter");
    println!("adapter: {}", adapter.get_info().name);
    let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor { label: Some("relight_golden"), ..Default::default() })).expect("a device");
    let mut names: Vec<PathBuf> = std::fs::read_dir(&dir).expect("golden dir").filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect();
    names.sort();
    let mut worst = 0f64;
    for case in names {
        let txt = std::fs::read_to_string(case.join("case.txt")).unwrap();
        let label = case.file_name().unwrap().to_string_lossy().to_string();
        let (w, h, plan) = match kv(&txt, "mode").as_deref() {
            Some("auto") => continue,
            Some("pipeline") => {
                let scene_txt: String = txt.lines().filter_map(|l| l.strip_prefix("scene.")).collect::<Vec<_>>().join("\n");
                let (_, _, scene) = parse_case(&scene_txt);
                let (w, h): (u32, u32) = (kv(&txt, "w").unwrap().parse().unwrap(), kv(&txt, "h").unwrap().parse().unwrap());
                let hour: f32 = kv(&txt, "hour").unwrap().parse().unwrap();
                let st: f32 = kv(&txt, "strength").unwrap().parse().unwrap();
                let (se, sd) = with_intensity(Some(&scene), st);
                let keyframes = phase_keyframes(hour, PhaseBoundaries::default())
                    .into_iter()
                    .map(|(phase, wgt)| {
                        let (pe, pd) = with_intensity(default_phase_light(phase).as_ref(), st);
                        (Pass::of(&pe, &pd), wgt)
                    })
                    .collect();
                (w, h, Plan { scene: Pass::of(&se, &sd), keyframes })
            }
            _ => {
                let (w, h, env) = parse_case(&txt);
                (w as u32, h as u32, Plan { scene: Pass::of(&env, &Derived::new(&env)), keyframes: Vec::new() })
            }
        };
        let input = std::fs::read(case.join("input.rgba")).unwrap();
        let expected = std::fs::read(case.join("expected.rgba")).unwrap();
        let out = relight_pixels(&device, &queue, &input, w, h, &plan);
        let (mut sum, mut max, mut n) = (0f64, 0f64, 0usize);
        for (a, b) in out.chunks_exact(4).zip(expected.chunks_exact(4)) {
            if a[3] == 0 {
                continue;
            }
            let (p, q) = (oklab8(a), oklab8(b));
            let d = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt();
            sum += d;
            max = max.max(d);
            n += 1;
        }
        println!("{label:28} mean dE {:.5}  max {:.5}{}", sum / n.max(1) as f64, max, if max > 0.01 { "  OVER" } else { "" });
        worst = worst.max(max);
    }
    println!("worst max dE {worst:.5} (limit 0.01): {}", if worst <= 0.01 { "PASS" } else { "FAIL" });
}
