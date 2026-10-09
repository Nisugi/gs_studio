//! arm_hang: how far out each raised arm of a baked puppet hangs
//! (`Puppet::arm_hangs`), degrees from straight down, and as drawn, and
//! how far from the body's middle its hand ends; `<puppet.toml>@<deg>`
//! hangs it at that (`Puppet::arm_hangs_at`, a body's `arms_hang`).
//!
//!   cargo run --release -p gs_puppet --example arm_hang -- <puppet.toml>[@deg] ...

use gs_puppet::math::{mul_mv, sub};
use gs_puppet::Puppet;

fn main() {
    for arg in std::env::args().skip(1) {
        let (path, at) = match arg.rsplit_once('@') {
            Some((path, deg)) => (path.to_owned(), deg.parse::<f32>().ok()),
            None => (arg.clone(), None),
        };
        let text = std::fs::read_to_string(&path).expect("read the puppet");
        let mut puppet = Puppet::from_baked_toml(&text).expect("a baked puppet");
        let out = |v: [f32; 3]| v[0].abs().atan2(-v[1]).to_degrees();
        if let Some(deg) = at {
            puppet.arm_hang = puppet.arm_hangs_at(deg);
        }
        let hangs = puppet.arm_hang.clone();
        println!("{path}");
        for (i, s) in puppet.segments.iter().enumerate() {
            if s.chain_index != 0 || !s.part.as_deref().is_some_and(|p| p.ends_with("Arm")) {
                continue;
            }
            let v = sub(s.tail, s.head);
            let hung = hangs.iter().find(|(k, _)| *k == i).map(|(_, m)| out(mul_mv(m, v)));
            println!("  {:8} drawn {:5.1} deg out, hangs {}", s.name, out(v), hung.map_or("as drawn".to_owned(), |h| format!("{h:5.1} deg out")));
        }
        // where the hands end, standing as at rest
        let posed = puppet.pose(&gs_puppet::Pose::default());
        for (i, s) in puppet.segments.iter().enumerate() {
            if s.part.as_deref().is_some_and(|p| p.ends_with("Hand")) {
                let tip = posed.xf[i].point(s.tail);
                println!("  {:8} its tip {:.2} ft from the middle, {:.2} ft up", s.name, tip[0].abs(), tip[1]);
            }
        }
    }
}
