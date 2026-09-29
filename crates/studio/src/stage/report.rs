//! A scene's report, without the drawing: what its lamps light and what
//! they cost, for whoever lights the scene next -- a person deciding where
//! a fill light goes, or Claude answering a note that says "too dark by the
//! door". `gs-studio --scene-report <scene>` writes it with a screenshot.
//!
//! Distances are metres on the ground, from the v2 pass's own camera (the
//! numbers the lamps' reach is in); costs are the pass timed offscreen on
//! this machine's GPU, the frame whole and per lamp what leaving it out
//! would save and what its shadow alone costs.

use std::fmt::Write;

use crate::stage::scene_notes::lamp_label;
use crate::stage::StageState;

/// One lamp as the pass saw it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LampFacts {
    /// How far its light reaches, metres.
    pub reach_m: f32,
    /// It got into the pass (past its lights limit it is drawn the old way).
    pub in_pass: bool,
    /// It asks for shadows and got one of the pass's shadow slots.
    pub shadow_given: bool,
    /// Milliseconds a frame leaving it out would save, and its shadow alone.
    pub cost_ms: Option<f32>,
    pub shadow_ms: Option<f32>,
}

/// The v2 pass's frame as drawn: its camera (to put things in metres) and
/// what it cost.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PassFacts {
    /// The GPU it was timed on.
    pub gpu: String,
    /// The stage's size on screen, pixels (what the pass shaded).
    pub size: [u32; 2],
    /// The frame whole, milliseconds.
    pub frame_ms: Option<f32>,
    /// Metres per camera unit, and the focal length in stage pixels.
    pub k: f32,
    pub f: f32,
    /// The moons' lights ahead of the lamps.
    pub moons: usize,
    pub shadow_slots: usize,
    pub light_slots: usize,
    /// Per scene lamp, in the scene's order.
    pub lamps: Vec<LampFacts>,
}

impl PassFacts {
    /// A ground point (stage x, solver depth z) in metres: across from the
    /// view's centre line, and away.
    pub fn metres(&self, x: f32, z: f32) -> [f32; 2] {
        let away = z.max(0.4) * self.k;
        [(x - gs_field::solver::STAGE_W * 0.5) * away / self.f.max(1e-3), away]
    }

    fn apart(&self, a: (f32, f32), b: (f32, f32)) -> f32 {
        let (p, q) = (self.metres(a.0, a.1), self.metres(b.0, b.1));
        ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt()
    }
}

/// Something on the ground a lamp may reach.
struct Thing {
    name: String,
    at: (f32, f32),
    scenery: bool,
}

fn ms(v: Option<f32>) -> String {
    v.map_or("-".to_owned(), |v| format!("{v:.2}"))
}

/// The report, Markdown: the stage at a glance, the pass, a row per lamp
/// (where, reach, brightness, shadows, cost, what it reaches), what no lamp
/// reaches, and the scene's open notes. Without `pass` (the v2 pass is not
/// drawing) there are no metres or costs, and it says so.
pub fn scene_report(stage: &StageState, game_hour: f32, pass: Option<&PassFacts>) -> String {
    let scene = stage.scene();
    let name = stage.scene_name.trim();
    let mut out = String::new();
    let _ = writeln!(out, "# Scene report: {}\n", if name.is_empty() { "(unsaved scene)" } else { name });
    let _ = writeln!(out, "{}\n", stage.outline(game_hour));
    if let Some(bg) = &scene.background {
        let _ = writeln!(out, "Backdrop: `{bg}`\n");
    }
    let Some(pass) = pass else {
        let _ = writeln!(out, "The v2 GPU pass was not drawing this scene: no distances in metres, no costs.\n");
        return out;
    };
    let admitted = pass.lamps.iter().filter(|l| l.in_pass).count();
    let shadowed = pass.lamps.iter().filter(|l| l.shadow_given).count();
    let _ = writeln!(
        out,
        "GPU pass: {}, {}x{} px, {} ms a frame. Lights {} of {} ({} moon{}), lamp shadows {} of {}.\n",
        pass.gpu,
        pass.size[0],
        pass.size[1],
        ms(pass.frame_ms),
        admitted + pass.moons,
        pass.light_slots,
        pass.moons,
        if pass.moons == 1 { "" } else { "s" },
        shadowed,
        pass.shadow_slots,
    );

    // what stands on the ground: props, then creatures
    let mut things: Vec<Thing> = scene
        .props
        .iter()
        .enumerate()
        .map(|(k, p)| {
            let image = p.image.rsplit('/').next().unwrap_or(&p.image).trim_end_matches(".png");
            Thing { name: format!("{image} (prop {})", k + 1), at: (p.x, p.z), scenery: true }
        })
        .collect();
    for u in stage.field.units() {
        let name = u.members.first().and_then(|id| stage.creature(id)).map_or("?".to_owned(), |c| c.name.clone());
        things.push(Thing { name, at: (stage.field.foot(u).0, stage.field.ground_z(u)), scenery: false });
    }

    let _ = writeln!(out, "## Lamps\n");
    if scene.lights.is_empty() {
        let _ = writeln!(out, "None.\n");
    } else {
        let _ = writeln!(out, "| # | lamp | at (across, away) m | reach m | brightness | shadows | saves ms / its shadow ms | reaches |");
        let _ = writeln!(out, "|---|---|---|---|---|---|---|---|");
    }
    let mut reached = vec![false; things.len()];
    for (k, lamp) in scene.lights.iter().enumerate() {
        let facts = pass.lamps.get(k).cloned().unwrap_or_default();
        let at = pass.metres(lamp.x, lamp.z);
        let shadows = match (lamp.casts_shadows(), facts.shadow_given) {
            (true, true) => "yes",
            (true, false) => "asks, none left",
            (false, _) => "no (fill)",
        };
        let mut near: Vec<(f32, String)> = Vec::new();
        for (i, t) in things.iter().enumerate() {
            let lit = if t.scenery { lamp.lights(gs_field::lighting::LightTarget::Scenery) } else { lamp.lights(gs_field::lighting::LightTarget::Creatures) };
            let d = pass.apart((lamp.x, lamp.z), t.at);
            if lit && d <= facts.reach_m {
                reached[i] = true;
                near.push((d, format!("{} {d:.1} m", t.name)));
            }
        }
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut kind = format!("{} {}", lamp_label(lamp.kind), k + 1);
        if !lamp.all_round() {
            kind.push_str(&format!(", beam {:.0} deg at {:.0}", lamp.spread, lamp.aim));
        }
        if !facts.in_pass {
            kind.push_str(" (NOT in the pass: drawn the old way)");
        }
        let _ = writeln!(
            out,
            "| {} | {} | {:.1}, {:.1} | {:.1} | {:.2} | {} | {} / {} | {} |",
            k + 1,
            kind,
            at[0],
            at[1],
            facts.reach_m,
            lamp.intensity,
            shadows,
            ms(facts.cost_ms),
            ms(facts.shadow_ms),
            if near.is_empty() { "nothing".to_owned() } else { near.into_iter().map(|(_, s)| s).collect::<Vec<_>>().join(", ") },
        );
    }
    if !scene.lights.is_empty() {
        out.push('\n');
    }

    // what no lamp reaches, with the nearest lamp and how far
    let dark: Vec<String> = things
        .iter()
        .zip(&reached)
        .filter(|(_, r)| !**r)
        .map(|(t, _)| {
            let nearest = scene
                .lights
                .iter()
                .enumerate()
                .map(|(k, l)| (pass.apart((l.x, l.z), t.at), k, l))
                .min_by(|a, b| a.0.total_cmp(&b.0));
            match nearest {
                Some((d, k, l)) => format!("- {}: nearest {} {} at {d:.1} m", t.name, lamp_label(l.kind), k + 1),
                None => format!("- {}", t.name),
            }
        })
        .collect();
    if !dark.is_empty() {
        let _ = writeln!(out, "## Out of every lamp's reach\n\n{}\n", dark.join("\n"));
    }

    if let Ok(notes) = stage.notes.notes() {
        let open: Vec<String> = notes
            .open()
            .map(|n| {
                let about = if n.about.is_empty() { String::new() } else { format!(" (on {})", n.about) };
                format!("- #{}{about} at [{:.0}, {:.0}]: {}", n.id, n.at[0], n.at[1], n.text)
            })
            .collect();
        if !open.is_empty() {
            let _ = writeln!(out, "## Open notes\n\n{}\n", open.join("\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::StudioRoomContext;

    fn facts(lamps: Vec<LampFacts>) -> PassFacts {
        PassFacts { gpu: "Test GPU".into(), size: [1320, 705], frame_ms: Some(2.5), k: 1.0, f: 420.0, moons: 1, shadow_slots: 8, light_slots: 32, lamps }
    }

    /// A lamp's row names what it reaches, in metres, nearest first; what
    /// no lamp reaches is listed with the nearest lamp; without the pass
    /// the report says why it has no numbers.
    #[test]
    fn the_report_says_what_each_lamp_reaches_and_costs() {
        let mut s = StageState::with_cast(StudioRoomContext::default(), Vec::new());
        s.scene_mut().light = Some(Default::default());
        s.add_light(gs_field::lighting::LampKind::Campfire);
        {
            let l = &mut s.scene_mut().lights[0];
            l.x = 440.0;
            l.z = 6.0;
        }
        s.add_prop("scenery/ojandhaart_bench.png".into());
        s.add_prop("scenery/ojandhaart_statue.png".into());
        {
            let props = &mut s.scene_mut().props;
            props[0].x = 480.0;
            props[0].z = 7.0;
            props[1].x = 440.0;
            props[1].z = 30.0;
        }
        let pass = facts(vec![LampFacts { reach_m: 5.0, in_pass: true, shadow_given: true, cost_ms: Some(0.61), shadow_ms: Some(0.4) }]);
        let text = scene_report(&s, 23.0, Some(&pass));
        assert!(text.contains("GPU pass: Test GPU, 1320x705 px, 2.50 ms a frame. Lights 2 of 32 (1 moon), lamp shadows 1 of 8."), "{text}");
        let row = text.lines().find(|l| l.starts_with("| 1 | Campfire 1")).expect("the lamp's row");
        assert!(row.contains("| yes | 0.61 / 0.40 |"), "{row}");
        assert!(row.contains("ojandhaart_bench (prop 1) 1.2 m"), "{row}");
        assert!(!row.contains("statue"), "the statue is 24 m away: {row}");
        assert!(text.contains("- ojandhaart_statue (prop 2): nearest Campfire 1 at 24.0 m"), "{text}");
        assert!(scene_report(&s, 23.0, None).contains("was not drawing this scene"));
    }

    #[test]
    fn metres_follow_the_pass_camera() {
        let pass = facts(Vec::new());
        assert_eq!(pass.metres(440.0, 5.0), [0.0, 5.0]);
        let m = pass.metres(440.0 + 42.0, 10.0);
        assert!((m[0] - 1.0).abs() < 1e-4 && (m[1] - 10.0).abs() < 1e-4, "{m:?}");
    }
}
