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
//! creature draws as its family's creature doll (`gs_field::dolls`; today
//! only `humanoid` is published, so the wolf is a humanoid too -- tier 3 --
//! until a canine rig exists). If pool art exists it is used.
//!
//! The second bar edits the TARGETED creature: wound rank 0-3 per body part
//! and prone / kneeling / sitting / dead / boss.
//!
//! `STAGE_DEMO_EXIT_AFTER_SECS=<n>` closes the window by itself (smoke runs).
//! `STAGE_DEMO_SHOT=<path.png>` swaps in a showcase roster (healthy, arm
//! rank 1, chest rank 2, leg rank 3 = severed, prone, dead, boss), saves one
//! frame of the window to that PNG and exits.

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

/// `STAGE_DEMO_SHOT`: one doll per state worth looking at.
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
    vec![
        creature("#2001", "healthy bandit", hostile()),
        creature("#2002", "arm1 bandit", wounds(&[("rightArm", 1)])),
        creature("#2003", "chest2 bandit", wounds(&[("chest", 2)])),
        creature("#2004", "leg3 bandit", wounds(&[("leftLeg", 3)])),
        creature(
            "#2005",
            "prone bandit",
            CreatureFlags {
                statuses: vec!["prone".into()],
                ..wounds(&[("head", 2), ("nsys", 1)])
            },
        ),
        creature(
            "#2006",
            "dead bandit",
            CreatureFlags {
                dead: true,
                health: Some(0),
                ..wounds(&[("abdomen", 3)])
            },
        ),
        creature(
            "#2007",
            "boss bandit",
            CreatureFlags {
                ascension_boss: true,
                ..wounds(&[("leftEye", 2), ("rightArm", 3)])
            },
        ),
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
}

impl Demo {
    fn new() -> Self {
        let mut art = CreatureArtStore::default();
        // What SkinState::apply_if_changed did on first apply.
        art.reset();
        eprintln!("stage_demo: art root {}", art.root().display());
        let shot = std::env::var_os("STAGE_DEMO_SHOT").map(std::path::PathBuf::from);
        let creatures = if shot.is_some() {
            showcase_roster()
        } else {
            roster()
        };
        for c in &creatures {
            let doll = gs_field::dolls::doll_for(&c.name, c.noun.as_deref());
            eprintln!("stage_demo: {} -> doll {:?}", c.name, doll);
        }
        Self {
            field: CreatureField::default(),
            synced_gen: 0,
            synced_cal: 0,
            creatures,
            // Non-zero so the first sync_field sees a change.
            creatures_generation: 1,
            current_target: if shot.is_some() { "#2003" } else { "#1002" }.to_string(),
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
        if !self.shot_requested && self.frames >= 20 {
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
        // Art prep FIRST, roster sync second (VellumFE studio.rs, finding 8).
        let wanted = self.wanted_art();
        if !wanted.is_empty() {
            self.art.prepare_creature_art(&ctx, &wanted);
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
                scene: None,
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
