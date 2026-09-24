//! stage_demo: the ported creature-field renderer in a bare eframe window.
//!
//!   cargo run -p gs_field_egui --example stage_demo
//!
//! Fabricates a small room (kobold, troll, wolf, a boss, one prone, one
//! dead, two wounded), and every frame runs the same order VellumFE's Studio
//! Stage does (`studio.rs::stage_ui`): prepare creature art -> `sync_field`
//! -> prepare scene art -> render. Clicking a creature makes it the target.
//!
//! No art is needed: with nothing in `<data>/global/images/creatures` every
//! creature draws as its family's baked 3D PUPPET (`gs_field::puppets` ->
//! `gs_puppet`) -- humanoid and canine are published, so the wolf is a real
//! quadruped. A family with no puppet falls back to the flat doll
//! (`gs_field::dolls`), and a family with neither to the placeholder
//! standee. If pool art exists it is used instead of all three.
//!
//! The second bar edits the TARGETED creature: wound rank 0-3 per body part
//! and prone / kneeling / sitting / dead / boss.
//!
//! `STAGE_DEMO_NO_ART=1` points the art root at an empty directory, so every
//! creature takes the no-art path (puppet, then doll, then standee) even on a
//! machine with a full image pool -- which is what the showcase shot wants.
//! `STAGE_DEMO_SHOT` turns it on by itself.
//!
//! `STAGE_DEMO_EXIT_AFTER_SECS=<n>` closes the window by itself (smoke runs).
//! `STAGE_DEMO_SHOT=<path.png>` swaps in a showcase roster (healthy, arm
//! rank 1, chest rank 2, leg rank 3 = severed, prone, dead, boss), saves one
//! frame of the window to that PNG and exits.
//! `STAGE_DEMO_STATUSES=1` (with `STAGE_DEMO_SHOT`) shows the status poses
//! and body effects instead: webbed, sleeping, disoriented, calmed, frozen.
//! `STAGE_DEMO_LIGHT=<backdrop pool path>` (e.g. `scenes/desert.jpg`) puts
//! the field in front of that backdrop with scene lighting on (every value
//! auto), at game hour `STAGE_DEMO_HOUR` (default 12) and Lighting strength
//! `STAGE_DEMO_STRENGTH` (default 1). The art stays the real pool unless
//! `STAGE_DEMO_NO_ART` is set, so sprites and puppets are lit together.
//! `STAGE_DEMO_WEATHER=rain|storm|snow|sleet|blood|fog` adds weather.
//! `STAGE_DEMO_FIRE=1` (with `STAGE_DEMO_LIGHT`) puts a campfire mid-field
//! and a torch to one side.
//! `STAGE_DEMO_FLAT=1` draws puppets as the flat shape list instead of the
//! smooth skin.

use gs_field::solver::CreatureField;
use gs_field::{Condition, Creature, CreatureFlags};
use gs_field_egui::{
    creature_field_stage_pos, render_creature_field_content, CreatureArtStore,
    CreatureFieldView, WantedCreature,
};

fn creature(id: &str, name: &str, flags: CreatureFlags) -> Creature {
    Creature {
        name: name.to_string(),
        noun: name.split(' ').next_back().map(str::to_string),
        id: id.to_string(),
        status: None,
        flags: Some(flags),
    }
}

fn roster() -> Vec<Creature> {
    let hostile = || CreatureFlags {
        hostile: true,
        health: Some(100),
        max_health: Some(100),
        ..Default::default()
    };
    vec![
        creature("#1001", "kobold", hostile()),
        creature(
            "#1002",
            "forest troll",
            CreatureFlags {
                health: Some(55),
                injuries: vec![("head".into(), 2), ("leftArm".into(), 1)],
                ..hostile()
            },
        ),
        creature(
            "#1003",
            "timber wolf",
            CreatureFlags {
                injuries: vec![("rightLeg".into(), 3), ("chest".into(), 1)],
                ..hostile()
            },
        ),
        creature(
            "#1004",
            "Ithzir champion",
            CreatureFlags {
                ascension_boss: true,
                challenging: true,
                ..hostile()
            },
        ),
        creature(
            "#1005",
            "hill troll",
            CreatureFlags {
                statuses: vec!["prone".into(), "stunned".into()],
                ..hostile()
            },
        ),
        creature(
            "#1006",
            "kobold",
            CreatureFlags {
                dead: true,
                health: Some(0),
                ..hostile()
            },
        ),
    ]
}

/// `STAGE_DEMO_SHOT`: one puppet per state worth looking at -- the two
/// published body types, every pose the state machine can ask for, and the
/// wound ranks including a severance.
fn showcase_roster() -> Vec<Creature> {
    let hostile = || CreatureFlags {
        hostile: true,
        health: Some(100),
        max_health: Some(100),
        ..Default::default()
    };
    let wounds = |list: &[(&str, u8)]| CreatureFlags {
        injuries: list.iter().map(|(p, r)| (p.to_string(), *r)).collect(),
        ..hostile()
    };
    let posed = |id: &str, name: &str, status: &str, w: &[(&str, u8)]| {
        creature(
            id,
            name,
            CreatureFlags {
                statuses: vec![status.to_string()],
                ..wounds(w)
            },
        )
    };
    vec![
        creature("#2001", "healthy bandit", hostile()),
        creature("#2002", "chest2 bandit", wounds(&[("chest", 2)])),
        creature("#2003", "leg3 bandit", wounds(&[("leftLeg", 3)])),
        posed("#2004", "kneeling bandit", "kneeling", &[("rightArm", 1)]),
        posed("#2005", "stunned bandit", "stunned", &[("head", 2)]),
        posed("#2006", "prone bandit", "prone", &[("head", 2), ("nsys", 1)]),
        creature(
            "#2007",
            "dead bandit",
            CreatureFlags {
                dead: true,
                health: Some(0),
                ..wounds(&[("abdomen", 3)])
            },
        ),
        creature(
            "#2008",
            "boss bandit",
            CreatureFlags {
                ascension_boss: true,
                ..wounds(&[("leftEye", 2), ("rightArm", 3)])
            },
        ),
        // The other published body type: a quadruped puppet, standing,
        // lying and dead.
        creature("#2009", "timber wolf", wounds(&[("rightLeg", 1)])),
        posed("#2010", "sitting wolf", "sitting", &[]),
        posed("#2011", "prone wolf", "prone", &[("chest", 2)]),
        creature(
            "#2012",
            "dead wolf",
            CreatureFlags {
                dead: true,
                health: Some(0),
                ..hostile()
            },
        ),
    ]
}

/// `STAGE_DEMO_STATUSES`: each status pose and body effect, on both
/// published body types.
fn status_roster() -> Vec<Creature> {
    let with = |id: &str, name: &str, status: &str| {
        creature(
            id,
            name,
            CreatureFlags {
                hostile: true,
                health: Some(100),
                max_health: Some(100),
                statuses: vec![status.to_string()],
                ..Default::default()
            },
        )
    };
    vec![
        with("#3001", "webbed bandit", "webbed"),
        with("#3002", "sleeping bandit", "sleeping"),
        with("#3003", "disoriented bandit", "disoriented"),
        with("#3004", "calmed bandit", "calmed"),
        with("#3005", "frozen bandit", "frozen"),
        with("#3006", "webbed wolf", "webbed"),
        with("#3007", "sleeping wolf", "sleeping"),
        with("#3008", "disoriented wolf", "disoriented"),
        with("#3009", "frozen wolf", "frozen"),
    ]
}

/// The host owns condition evaluation. The demo has no game state, so only
/// creature-side leaves can hold (same shape as gs_field's test evaluator).
fn eval(cond: &Condition, flags: &CreatureFlags) -> bool {
    match cond {
        Condition::All { conditions } => conditions.iter().all(|c| eval(c, flags)),
        Condition::Any { conditions } => conditions.iter().any(|c| eval(c, flags)),
        Condition::CrtrStatus { id, active } => flags.has_flag(id) == *active,
        _ => false,
    }
}

struct Demo {
    field: CreatureField,
    synced_gen: u64,
    synced_cal: u64,
    creatures: Vec<Creature>,
    creatures_generation: u64,
    current_target: String,
    art: CreatureArtStore,
    show_grid: bool,
    show_order: bool,
    frames: u64,
    started: std::time::Instant,
    exit_after: Option<f32>,
    last_stage_pos: Option<(f32, f32)>,
    /// `STAGE_DEMO_SHOT`: where to write the frame, and whether it was asked for.
    shot: Option<std::path::PathBuf>,
    shot_requested: bool,
    /// `STAGE_DEMO_LIGHT`: a lit scene in front of that backdrop.
    scene: Option<gs_field::scenes::StageScene>,
    /// Frames since the lighting bake settled.
    light_settled: u64,
}

impl Demo {
    fn new() -> Self {
        let mut art = CreatureArtStore::default();
        // What SkinState::apply_if_changed did on first apply.
        art.reset();
        let shot = std::env::var_os("STAGE_DEMO_SHOT").map(std::path::PathBuf::from);
        // The showcase shot is about the no-art path, so it must not find a
        // sprite: re-point the resolve root at an empty directory.
        let light_bg = std::env::var("STAGE_DEMO_LIGHT").ok();
        if (shot.is_some() && light_bg.is_none()) || std::env::var_os("STAGE_DEMO_NO_ART").is_some() {
            // The whole data dir, not just the resolve root: the pool
            // cascade and its caches key on the base dir.
            let empty = std::env::temp_dir().join("stage_demo_no_art");
            let _ = std::fs::create_dir_all(empty.join("global").join("images"));
            std::env::set_var("VELLUM_FE_DIR", &empty);
            gs_field::data_dirs::clear_base_dir();
            gs_field::pool::invalidate_cache();
            art.reset_with_root(&empty.join("global").join("images"));
            eprintln!("stage_demo: art forced empty ({})", empty.display());
        }
        eprintln!("stage_demo: art root {}", art.root().display());
        let creatures = if shot.is_some() {
            if std::env::var_os("STAGE_DEMO_STATUSES").is_some() { status_roster() } else { showcase_roster() }
        } else {
            roster()
        };
        for c in &creatures {
            let puppet = gs_field::puppets::puppet_for(&c.name, c.noun.as_deref());
            let doll = gs_field::dolls::doll_for(&c.name, c.noun.as_deref());
            eprintln!(
                "stage_demo: {} -> puppet {:?}, doll {:?}",
                c.name, puppet, doll
            );
        }
        Self {
            field: CreatureField::default(),
            synced_gen: 0,
            synced_cal: 0,
            creatures,
            // Non-zero so the first sync_field sees a change.
            creatures_generation: 1,
            current_target: if shot.is_some() { "#2002" } else { "#1002" }.to_string(),
            art,
            show_grid: true,
            show_order: true,
            frames: 0,
            started: std::time::Instant::now(),
            exit_after: std::env::var("STAGE_DEMO_EXIT_AFTER_SECS")
                .ok()
                .and_then(|s| s.parse().ok()),
            last_stage_pos: None,
            shot,
            shot_requested: false,
            scene: light_bg.map(|bg| {
                use gs_field::lighting::{LampKind, PointLight};
                let lights = if std::env::var_os("STAGE_DEMO_FIRE").is_some() {
                    vec![PointLight::new(LampKind::Campfire, 470.0, 3.6), PointLight::new(LampKind::Torch, 250.0, 3.0)]
                } else {
                    Vec::new()
                };
                gs_field::scenes::StageScene {
                    background: Some(bg),
                    light: Some(gs_field::lighting::SceneLight::default()),
                    lights,
                    ..Default::default()
                }
            }),
            light_settled: 0,
        }
    }

    /// The second bar: wound ranks and pose / class flags of the targeted
    /// creature. Any edit bumps the roster generation, exactly as a
    /// crtrStatus update does in the game (poses re-derive the card box).
    fn simulator_bar(&mut self, ui: &mut egui::Ui) {
        let target = self.current_target.trim_start_matches('#').to_string();
        let Some(c) = self
            .creatures
            .iter_mut()
            .find(|c| c.id.trim_start_matches('#') == target)
        else {
            ui.label("click a creature to edit it");
            return;
        };
        let name = c.name.clone();
        let flags = c.flags.get_or_insert_with(Default::default);
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.strong(name);
            ui.separator();
            for status in ["prone", "kneeling", "sitting"] {
                let mut on = flags.statuses.iter().any(|s| s == status);
                if ui.checkbox(&mut on, status).changed() {
                    flags.statuses.retain(|s| s != status);
                    if on {
                        flags.statuses.push(status.to_string());
                    }
                    changed = true;
                }
            }
            changed |= ui.checkbox(&mut flags.dead, "dead").changed();
            changed |= ui.checkbox(&mut flags.ascension_boss, "boss").changed();
        });
        // Compact grid: 14 parts, two rows of seven, each a 0-3 rank picker.
        egui::Grid::new("demo_wounds")
            .num_columns(7)
            .spacing([10.0, 2.0])
            .show(ui, |ui| {
                for (i, (part, _, _)) in gs_field::skins::DOLL_PARTS.iter().enumerate() {
                    let mut rank = flags
                        .injuries
                        .iter()
                        .find(|(p, _)| p == part)
                        .map(|(_, r)| *r)
                        .unwrap_or(0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        ui.label(format!("{part}:"));
                        for r in 0..=3u8 {
                            if ui.selectable_label(rank == r, r.to_string()).clicked() && rank != r
                            {
                                rank = r;
                                flags.injuries.retain(|(p, _)| p != part);
                                if r > 0 {
                                    flags.injuries.push((part.to_string(), r));
                                }
                                changed = true;
                            }
                        }
                    });
                    if i % 7 == 6 {
                        ui.end_row();
                    }
                }
            });
        if changed {
            self.creatures_generation += 1;
        }
    }

    /// `STAGE_DEMO_SHOT`: ask for a screenshot once things have settled,
    /// write it when it arrives, then close.
    fn screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.shot.clone() else {
            return;
        };
        if !self.shot_requested && self.frames >= 20 && (self.scene.is_none() || self.light_settled >= 10) {
            self.shot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            let [w, h] = image.size;
            let rgba: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
            match image::save_buffer(
                &path,
                &rgba,
                w as u32,
                h as u32,
                image::ExtendedColorType::Rgba8,
            ) {
                Ok(()) => eprintln!("stage_demo: wrote {} ({w}x{h})", path.display()),
                Err(e) => eprintln!("stage_demo: could not write {}: {e}", path.display()),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }

    /// Same recipe as Studio's `StageState::wanted_art`.
    fn wanted_art(&self) -> Vec<WantedCreature> {
        self.creatures
            .iter()
            .map(|c| WantedCreature {
                name: c.name.clone(),
                noun: c.noun.clone(),
                family: c.noun.as_deref().and_then(gs_field::cards::family_for_noun),
                injuries: c
                    .flags
                    .as_ref()
                    .map(|f| f.injuries.clone())
                    .unwrap_or_default(),
            })
            .collect()
    }
}

impl eframe::App for Demo {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if std::env::var_os("STAGE_DEMO_FLAT").is_some() {
            gs_field_egui::creature_puppet::set_puppet_skin(&ctx, false);
        }
        if let Ok(w) = std::env::var("STAGE_DEMO_WEATHER") {
            use gs_field_egui::field_weather::{set_weather, Weather, WeatherKind};
            let kind = match w.as_str() {
                "storm" => WeatherKind::Storm,
                "snow" => WeatherKind::Snow,
                "sleet" => WeatherKind::Sleet,
                "blood" => WeatherKind::BloodRain,
                "fog" => WeatherKind::Fog,
                _ => WeatherKind::Rain,
            };
            set_weather(&ctx, Some(Weather { kind, intensity: 0.8, wind: 0.25 }));
        }
        // Art prep FIRST, roster sync second (VellumFE studio.rs, finding 8).
        let wanted = self.wanted_art();
        if !wanted.is_empty() {
            self.art.prepare_creature_art(&ctx, &wanted);
        }
        if let Some(scene) = &self.scene {
            let env = |k: &str, d: f32| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
            self.art.set_light_clock(gs_field_egui::creature_light::LightClock {
                game_hour: Some(env("STAGE_DEMO_HOUR", 12.0)),
                strength: env("STAGE_DEMO_STRENGTH", 1.0),
                outdoor_guess: None,
            });
            self.art.prepare_scene_art(&ctx, scene);
            if self.art.lighting_busy() {
                self.light_settled = 0;
                ctx.request_repaint();
            } else {
                self.light_settled += 1;
            }
        }
        gs_field::cards::sync_field(
            &mut self.field,
            &mut self.synced_gen,
            &mut self.synced_cal,
            &self.creatures,
            self.creatures_generation,
            &[],
        );
        // No scene in the demo, so no prepare_scene_art call (the game does
        // not call it either).
        let art = self.art.creature_art();

        self.frames += 1;
        if self.frames == 1 || self.frames == 60 {
            eprintln!(
                "stage_demo: frame {} ok, {} creatures -> {} units",
                self.frames,
                self.creatures.len(),
                self.field.units().len()
            );
        }

        egui::Panel::top("demo_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.show_grid, "Grid");
                ui.checkbox(&mut self.show_order, "Draw order");
                if ui.button("Reload art").clicked() {
                    self.art.force_reload();
                }
                ui.separator();
                ui.label(format!("target: {}", self.current_target));
                ui.separator();
                let puppets = gs_field::puppets::puppets();
                ui.label(format!(
                    "puppets: {}",
                    puppets.families().collect::<Vec<_>>().join(", ")
                ));
                if let Some((x, y)) = self.last_stage_pos {
                    ui.label(format!("last click, stage: ({x:.0}, {y:.0})"));
                }
            });
            self.simulator_bar(ui);
        });
        egui::CentralPanel::default().show(ui, |ui| {
            let rect = egui::Rect::from_min_size(ui.next_widget_position(), ui.available_size());
            let view = CreatureFieldView {
                field: &self.field,
                creatures: &self.creatures,
                current_target: &self.current_target,
                eval_condition: Box::new(eval),
                effect_severity: Box::new(|_, _| None),
                show_grid: self.show_grid,
                show_order: self.show_order,
                creature_art: Some(&art),
                scene: self.scene.as_ref(),
            };
            let click = render_creature_field_content(&view, ui);
            drop(view);
            if let Some(click) = click {
                self.last_stage_pos = Some(creature_field_stage_pos(rect, click.pointer_pos));
                eprintln!("stage_demo: clicked {}", click.exist_id);
                self.current_target = format!("#{}", click.exist_id);
            }
        });

        self.screenshot(&ctx);
        if let Some(secs) = self.exit_after {
            if self.started.elapsed().as_secs_f32() >= secs {
                eprintln!("stage_demo: {} frames, exiting cleanly", self.frames);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ctx.request_repaint();
        }
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("gs_field_egui stage demo")
            .with_inner_size([1100.0, 700.0]),
        ..Default::default()
    };
    eframe::run_native(
        "gs_field_egui_stage_demo",
        options,
        Box::new(|_cc| Ok(Box::new(Demo::new()))),
    )
}
