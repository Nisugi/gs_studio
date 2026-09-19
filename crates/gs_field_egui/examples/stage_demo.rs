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
//! creature draws as a placeholder standee. If pool art exists it is used.
//!
//! `STAGE_DEMO_EXIT_AFTER_SECS=<n>` closes the window by itself (smoke runs).

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
}

impl Demo {
    fn new() -> Self {
        let mut art = CreatureArtStore::default();
        // What SkinState::apply_if_changed did on first apply.
        art.reset();
        eprintln!("stage_demo: art root {}", art.root().display());
        Self {
            field: CreatureField::default(),
            synced_gen: 0,
            synced_cal: 0,
            creatures: roster(),
            // Non-zero so the first sync_field sees a change.
            creatures_generation: 1,
            current_target: "#1002".to_string(),
            art,
            show_grid: true,
            show_order: true,
            frames: 0,
            started: std::time::Instant::now(),
            exit_after: std::env::var("STAGE_DEMO_EXIT_AFTER_SECS")
                .ok()
                .and_then(|s| s.parse().ok()),
            last_stage_pos: None,
        }
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
