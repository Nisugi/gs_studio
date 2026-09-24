//! The Preview stage's egui layer: VellumFE's Studio "Stage", drawn by the
//! renderer the clients use (`gs_field_egui`). It draws `stage::StageState`
//! and forwards input to it; every rule lives in `crate::stage`.
//!
//! One tool, two tabs. **Scenes** and **Preview** share this one
//! `StageTool` (one sandbox, one stage in the middle); the tab only picks
//! which side panel is showing -- scene authoring (scene, props, camera,
//! solver) or the cast (cast, roster, per-creature simulator).
//!
//! Ported from VellumFE (GPL-3.0-or-later) `src/frontend/gui/studio.rs` at
//! commit 60c84a79.

use gs_field::condition::INJURY_AREAS;
use gs_field::creature::CRTR_STATUS_FLAGS;
use gs_field::scenes::{self, SceneBinding};
use gs_field::solver::{camera_limits, SeparationBasis};
use gs_field::{Creature, CreatureFlags};
use gs_field_egui::{
    creature_field_stage_pos, render_creature_field_content, CreatureArtStore, CreatureFieldView, SharedCreatureArt,
    WantedCreature,
};

use crate::stage::color::{hex_rgb, parse_hex_rgb, NEUTRAL_SLATE};
use crate::stage::eval::eval_condition;
use crate::stage::{cast, ClassFlag, StageState, StudioRoomContext};

/// Which side panel the shared stage shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StagePanel {
    /// Scene, props, camera, solver.
    Scenes,
    /// Cast, roster, the per-creature simulator.
    Preview,
}

pub struct StageTool {
    stage: StageState,
    art: CreatureArtStore,
    /// `global/lighting.toml` (time-of-day defaults), read once.
    lighting: gs_field::lighting::GlobalLighting,
}

/// Art wanted for the current roster -- same recipe as the game's update
/// loop (family from the bestiary, wounds from flags; prone art always
/// loads so envelopes are known up front).
fn wanted_art(creatures: &[Creature]) -> Vec<WantedCreature> {
    creatures
        .iter()
        .map(|c| WantedCreature {
            name: c.name.clone(),
            noun: c.noun.clone(),
            family: c.noun.as_deref().and_then(gs_field::cards::family_for_noun),
            injuries: c.flags.as_ref().map(|f| f.injuries.clone()).unwrap_or_default(),
        })
        .collect()
}

impl StageTool {
    /// Scans the pool for the cast. Call after the pool directory is set.
    pub fn new(context: StudioRoomContext) -> StageTool {
        let mut art = CreatureArtStore::default();
        // What VellumFE's SkinState::apply_if_changed did on first apply.
        art.reset();
        let mut stage = StageState::new(context);
        // Smoke runs (no human at the window): GS_STUDIO_SMOKE=1 puts the
        // first few cast entries, an art-less creature (which draws as its
        // family puppet), a wounded/stunned/targeted creature and the first
        // saved scene on the stage, so the real art and render paths run.
        if std::env::var_os("GS_STUDIO_SMOKE").is_some() {
            let picks: Vec<_> = stage.cast.iter().take(3).map(|e| (e.display.clone(), e.noun.clone())).collect();
            for (display, noun) in picks {
                stage.spawn(&display, Some(noun));
            }
            stage.spawn_named("training dummy", Some("dummy".to_string()));
            // A real creature the pool almost certainly has no art for:
            // exercises the non-humanoid puppet path.
            stage.spawn_named("timber wolf", Some("wolf".to_string()));
            stage.set_status("studio-1", "stunned", true);
            stage.set_wound("studio-1", "head", 2);
            stage.set_health_bar("studio-1", true);
            stage.set_target("studio-1", true);
            if let Some(name) = scenes::list_scenes().first() {
                stage.load_scene_by_name(name);
            }
            eprintln!(
                "gs-studio: smoke stage: {} cast, {} spawned, scene '{}'",
                stage.cast.len(),
                stage.creatures().len(),
                stage.scene_name
            );
        }
        let lighting = gs_field::lighting::GlobalLighting::load();
        art.set_global_lighting(lighting.clone());
        StageTool { stage, art, lighting }
    }

    /// A line for the stage's own status log (the import report, say).
    pub fn push_status(&mut self, message: String) {
        self.stage.push_status(message);
    }

    /// "Show on stage" from the rig editor: put that family's puppet up.
    pub fn spawn_puppet(&mut self, family: &str) {
        self.stage.spawn_puppet(family);
    }

    /// The pool directory moved: re-root the art, re-scan the cast.
    pub fn pool_changed(&mut self) {
        let root = gs_field::data_dirs::global_images_dir().unwrap_or_default();
        self.art.reset_with_root(&root);
        self.stage.pool_changed();
    }

    /// A calibrator saved: art and sidecars changed on disk. Reloads the
    /// art store -- which also drops gs_field's geometry-calibration store,
    /// as VellumFE's `skin_state.force_reload()` did -- and, after a SCENERY
    /// save, the solver obstacles (exclusion spans read the prop sidecar).
    pub fn art_changed(&mut self, invalidate_obstacles: bool) {
        self.art.force_reload();
        if invalidate_obstacles {
            self.stage.invalidate_obstacles();
        }
        self.stage.push_status(
            if invalidate_obstacles { "Art and prop obstacles reloaded (scenery calibration saved)" } else { "Art reloaded (calibration saved)" }
                .to_string(),
        );
    }

    /// One line for the smoke-run log: what is on the stage right now.
    pub fn summary(&self) -> String {
        format!(
            "{} creatures in {} units, {} props, {} status lines (last: {:?})",
            self.stage.creatures().len(),
            self.stage.field.units().len(),
            self.stage.scene().props.len(),
            self.stage.status_lines().len(),
            self.stage.status_lines().last()
        )
    }

    pub fn status(&self, ui: &mut egui::Ui) {
        let lines = self.stage.status_lines();
        match lines.last() {
            Some(message) => ui.label(message),
            None => ui.weak("Ready"),
        }
        .on_hover_ui(|ui| {
            for message in lines {
                ui.label(message);
            }
        });
        ui.separator();
        ui.weak(format!("{} on stage", self.stage.creatures().len()));
    }

    /// Returns true when the Scenes panel asked for the scenery calibrator
    /// (the app switches to the Scenery tab).
    pub fn ui(&mut self, ui: &mut egui::Ui, panel: StagePanel) -> bool {
        let mut calibrate_scenery = false;
        let ctx = ui.ctx().clone();
        let stage = &mut self.stage;
        // Scene edits re-point the scene Arc; the solver's authored
        // exclusion metadata is rebuilt only then.
        stage.refresh_obstacles();
        // Art prep FIRST, roster sync second (finding 8): preparing the
        // roster's art feeds standing + prone calibration into the core
        // geometry store, so the sync's initial placements reserve true
        // envelopes instead of fallback guesses that recalibrate would
        // have to reconcile a frame later. Both are cached/generation-
        // gated, so a settled stage costs a few hash lookups.
        let wanted = wanted_art(stage.creatures());
        if !wanted.is_empty() {
            self.art.prepare_creature_art(&ctx, &wanted);
        }
        stage.sync_field();
        stage.refresh_mounts();
        // Lighting preview: the scrubbed hour, else the live Elanthian clock.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64());
        let game_hour = stage.light_hour.unwrap_or_else(|| crate::stage::elanthian_time::elanthian_hour_f(now));
        self.art.set_light_clock(gs_field_egui::creature_light::LightClock {
            game_hour: Some(game_hour),
            strength: stage.light_strength,
            outdoor_guess: None,
        });
        self.art.prepare_scene_art(&ctx, stage.scene());
        if self.art.lighting_busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        } else if stage.scene().light.is_some() && stage.light_hour.is_none() {
            // The live clock moves dawn and dusk along.
            ctx.request_repaint_after(std::time::Duration::from_secs(20));
        }
        let auto = stage.scene().background.as_deref().and_then(|bg| self.art.scene_auto_colours(bg));
        let lighting = &self.lighting;
        let art = self.art.creature_art();

        egui::Panel::right("stage_panel").default_size(300.0).min_size(240.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match panel {
                StagePanel::Scenes => {
                    calibrate_scenery = scene_ui(stage, ui);
                    light_ui(stage, ui, auto.as_ref(), lighting, game_hour);
                    ui.separator();
                    camera_ui(stage, ui);
                    ui.separator();
                    solver_ui(stage, ui);
                }
                StagePanel::Preview => {
                    if stage.scene().light.is_some() {
                        ui.heading("Light");
                        light_preview_ui(stage, ui, game_hour);
                        ui.separator();
                    }
                    cast_ui(stage, ui)
                }
            });
        });
        egui::CentralPanel::default().show(ui, |ui| field_ui(stage, ui, &art, panel));
        calibrate_scenery
    }
}

// ------------------------------------------------------------------ scene

/// The Scene section: backdrop + scenery props, persisted whole-file to
/// `global/scenes/<name>.toml`. Returns true when "Calibrate scenery..." was
/// pressed.
/// The scene's `[light]`: the scene light read from the backdrop (each value
/// overridable), time-of-day overrides, and the preview clock. Niffy's
/// "Stage editor: Light section".
fn light_ui(
    stage: &mut StageState,
    ui: &mut egui::Ui,
    auto: Option<&vellum_light::SceneColours>,
    global: &gs_field::lighting::GlobalLighting,
    game_hour: f32,
) {
    use gs_field::lighting::{LightChroma, SceneLight, SCENE_BOUNCE_STRENGTH, SCENE_STRENGTH};
    use vellum_light::Phase;

    ui.separator();
    ui.heading("Light");
    let mut on = stage.scene().light.is_some();
    if ui
        .checkbox(&mut on, "Light this scene")
        .on_hover_text("Light creatures and props to match the backdrop, and apply dawn, dusk and night outdoors")
        .changed()
    {
        stage.scene_mut().light = on.then(SceneLight::default);
    }
    let Some(light) = stage.scene().light.clone() else {
        ui.weak("Unlit: creatures and props draw exactly as painted.");
        return;
    };
    let mut edited = light.clone();
    if auto.is_none() {
        ui.weak("No backdrop to read: \"auto\" values use a neutral daylight.");
    }
    light_colour_row(ui, "Key", &mut edited.key, auto.map(|a| a.key.as_str()), "Main light: the faces turned toward it");
    light_colour_row(ui, "Ambient", &mut edited.ambient, auto.map(|a| a.ambient.as_str()), "Fill light: the shadowed side (sky or ceiling)");
    light_colour_row(ui, "Ground", &mut edited.bounce, auto.map(|a| a.bounce.as_str()), "Ground bounce: rises into the lower shadows");
    ui.horizontal(|ui| {
        ui.label("Saturation").on_hover_text("How colourful creatures stay: a misty backdrop mutes them to match");
        let auto_c = auto.map_or(0.9, |a| a.chroma);
        let mut v = match edited.chroma {
            LightChroma::Auto => auto_c,
            LightChroma::Value(v) => v,
        };
        if ui.add(egui::DragValue::new(&mut v).range(0.0..=1.2).speed(0.01)).changed() {
            edited.chroma = LightChroma::Value(v);
        }
        if edited.chroma.is_auto() {
            ui.weak("auto");
        } else if ui.small_button("Reset to auto").clicked() {
            edited.chroma = LightChroma::Auto;
        }
    });
    ui.add(egui::Slider::new(&mut edited.intensity, 0.0..=1.0).text("Intensity"));
    let mut outdoor = edited.outdoor.unwrap_or(true);
    if ui.checkbox(&mut outdoor, "Outdoor").on_hover_text("Dawn, dusk and night apply").changed() {
        edited.outdoor = Some(outdoor);
    }
    egui::CollapsingHeader::new("More").id_salt("light_more").show(ui, |ui| {
        light_opt_slider(ui, "Strength", &mut edited.strength, SCENE_STRENGTH, 0.0..=1.0);
        light_opt_slider(ui, "Ground bounce", &mut edited.bounce_strength, SCENE_BOUNCE_STRENGTH, 0.0..=1.0);
        light_opt_slider(ui, "Exposure", &mut edited.exposure, 1.0, 0.2..=2.0);
        light_opt_slider(ui, "Haze", &mut edited.haze, 0.0, 0.0..=0.5);
    });
    egui::CollapsingHeader::new("Time of day").id_salt("light_phases").show(ui, |ui| {
        for (phase, label) in [(Phase::Dawn, "Dawn"), (Phase::Day, "Day"), (Phase::Dusk, "Dusk"), (Phase::Night, "Night")] {
            let default = global.phase.get(phase).cloned().unwrap_or_default();
            let slot = match phase {
                Phase::Dawn => &mut edited.dawn,
                Phase::Day => &mut edited.day,
                Phase::Dusk => &mut edited.dusk,
                Phase::Night => &mut edited.night,
            };
            light_phase_row(ui, label, slot, &default);
        }
    });
    if edited != light {
        stage.scene_mut().light = Some(edited);
    }

    light_preview_ui(stage, ui, game_hour);
}

/// The lighting preview: which game hour the stage shows, and the player's
/// Lighting strength. On the Scenes panel's Light section and on Preview.
fn light_preview_ui(stage: &mut StageState, ui: &mut egui::Ui, game_hour: f32) {
    ui.label("Time of day");
    ui.horizontal_wrapped(|ui| {
        let mut live = stage.light_hour.is_none();
        if ui.checkbox(&mut live, "Live clock").on_hover_text("The game's own time (Elanthian, US Eastern)").changed() {
            stage.light_hour = (!live).then_some(game_hour);
        }
        for (label, hour) in [("Dawn", 7.0), ("Day", 12.0), ("Dusk", 19.0), ("Night", 23.0)] {
            if ui.small_button(label).clicked() {
                stage.light_hour = Some(hour);
            }
        }
    });
    let mut hour = game_hour;
    let h = hour as u32;
    let text = format!("{:02}:{:02}", h, ((hour - h as f32) * 60.0) as u32);
    if ui.add(egui::Slider::new(&mut hour, 0.0..=24.0).step_by(0.05).text(text)).changed() {
        stage.light_hour = Some(hour);
    }
    ui.add(egui::Slider::new(&mut stage.light_strength, 0.0..=1.0).text("Lighting strength"))
        .on_hover_text("The player's setting: scales every light, scene and time of day alike");
}

fn light_colour_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut gs_field::lighting::LightColour,
    auto: Option<&str>,
    hint: &str,
) {
    use gs_field::lighting::LightColour;
    ui.horizontal(|ui| {
        ui.label(label).on_hover_text(hint);
        let shown = match value {
            LightColour::Hex(h) => Some(h.clone()),
            LightColour::Auto => auto.map(str::to_owned),
        };
        let mut rgb = shown.as_deref().and_then(parse_hex_rgb).unwrap_or(NEUTRAL_SLATE);
        if ui.color_edit_button_srgb(&mut rgb).changed() {
            *value = LightColour::Hex(hex_rgb(rgb));
        }
        match value {
            LightColour::Auto => {
                ui.weak("auto");
            }
            LightColour::Hex(_) => {
                if ui.small_button("Reset to auto").clicked() {
                    *value = LightColour::Auto;
                }
            }
        }
    });
}

/// An optional number: shows the default until touched; "x" drops it.
fn light_opt_slider(ui: &mut egui::Ui, label: &str, value: &mut Option<f32>, default: f32, range: std::ops::RangeInclusive<f32>) {
    ui.horizontal(|ui| {
        let mut v = value.unwrap_or(default);
        if ui.add(egui::Slider::new(&mut v, range).text(label)).changed() {
            *value = Some(v);
        }
        if value.is_some() && ui.small_button("x").on_hover_text("Back to the default").clicked() {
            *value = None;
        }
    });
}

/// One time-of-day row: the global default, or this scene's own key,
/// ambient and intensity laid over it.
fn light_phase_row(
    ui: &mut egui::Ui,
    label: &str,
    over: &mut Option<gs_field::lighting::LightFields>,
    default: &gs_field::lighting::LightFields,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        let mut use_default = over.is_none();
        if ui.checkbox(&mut use_default, "Use default").changed() {
            *over = (!use_default).then(Default::default);
        }
    });
    let enabled = over.is_some();
    let mut fields = over.clone().unwrap_or_default();
    ui.add_enabled_ui(enabled, |ui| {
        ui.horizontal(|ui| {
            for (name, value, fallback) in [
                ("Key", &mut fields.key, default.key.clone()),
                ("Ambient", &mut fields.ambient, default.ambient.clone()),
            ] {
                ui.label(name);
                let mut rgb = value.clone().or(fallback).as_deref().and_then(parse_hex_rgb).unwrap_or(NEUTRAL_SLATE);
                if ui.color_edit_button_srgb(&mut rgb).changed() {
                    *value = Some(hex_rgb(rgb));
                }
            }
        });
        let mut v = fields.intensity.or(default.intensity).unwrap_or(if default.key.is_some() { 1.0 } else { 0.0 });
        if ui.add(egui::Slider::new(&mut v, 0.0..=1.0).text("Intensity")).changed() {
            fields.intensity = Some(v);
        }
    });
    if enabled && over.as_ref() != Some(&fields) {
        *over = Some(fields);
    }
}


fn scene_ui(stage: &mut StageState, ui: &mut egui::Ui) -> bool {
    ui.heading("Scene");
    // The scene FILENAME is the binding: leading room uid binds that room,
    // otherwise the stem matches room title -> location -> "default".
    // There is no separate bindings store to manage.
    ui.label(if stage.scene_name.trim().is_empty() {
        "Editing: (unsaved scene)".to_string()
    } else {
        format!("Editing: {}", scenes::display_name(stage.scene_name.trim()))
    });
    ui.horizontal_wrapped(|ui| {
        if !stage.scene_name.trim().is_empty() && ui.button("Save").clicked() {
            stage.save_current_scene();
        }
        if ui.button("Clear stage scene").clicked() {
            stage.clear_scene();
        }
    });
    new_scene_ui(stage, ui);

    let saved = scenes::list_scenes();
    if !saved.is_empty() {
        ui.horizontal(|ui| {
            ui.label("Load");
            ui.add(
                egui::TextEdit::singleline(&mut stage.scene_filter)
                    .hint_text("filter (number, title, location)")
                    .desired_width(160.0),
            );
        });
        let filter = stage.scene_filter.to_ascii_lowercase();
        let mut load_request: Option<String> = None;
        egui::ScrollArea::vertical().id_salt("scene_load_list").max_height(110.0).show(ui, |ui| {
            for name in &saved {
                // Menus render `_` as spaces; filter against the rendered
                // form so users type what they see.
                let display = scenes::display_name(name);
                if !filter.is_empty() && !display.to_ascii_lowercase().contains(&filter) {
                    continue;
                }
                if ui.selectable_label(stage.scene_name == *name, display).clicked() {
                    load_request = Some(name.clone());
                }
            }
        });
        if let Some(name) = load_request {
            stage.load_scene_by_name(&name);
        }
    }

    let backgrounds = gs_field::pool::list_category("scenes");
    ui.horizontal(|ui| {
        ui.label("Backdrop");
        let current = stage.scene().background.clone();
        let text = current
            .as_deref()
            .map(|p| p.rsplit('/').next().unwrap_or(p).to_string())
            .unwrap_or_else(|| "(none)".to_string());
        egui::ComboBox::from_id_salt("scene_bg").selected_text(text).show_ui(ui, |ui| {
            if ui.selectable_label(current.is_none(), "(none)").clicked() {
                stage.scene_mut().background = None;
            }
            for image in &backgrounds {
                let on = current.as_deref() == Some(image.pool_path.as_str());
                if ui.selectable_label(on, image.display_label()).clicked() {
                    stage.scene_mut().background = Some(image.pool_path.clone());
                }
            }
        });
    });
    ui.horizontal(|ui| {
        ui.label("Color");
        // The picker needs a concrete color; an unset scene edits a neutral
        // slate and only stores once touched (None keeps the default panel
        // fill).
        let mut rgb = stage.scene().background_color.as_deref().and_then(parse_hex_rgb).unwrap_or(NEUTRAL_SLATE);
        if ui.color_edit_button_srgb(&mut rgb).changed() {
            stage.scene_mut().background_color = Some(hex_rgb(rgb));
        }
        if stage.scene().background_color.is_some() && ui.small_button("x").on_hover_text("Clear color").clicked() {
            stage.scene_mut().background_color = None;
        }
    });

    ui.separator();
    ui.heading("Props");
    // VellumFE's Stage opened its scenery calibrator window from here; in
    // this app the calibrator is the Scenery tab, so the button goes there.
    // A save over there comes back through `StageTool::art_changed`.
    let calibrate_scenery = ui
        .button("Calibrate scenery...")
        .on_hover_text("Feet anchor, world size, exclusion edges -- per image. Opens the Scenery tab; the stage is kept.")
        .clicked();
    let pool = gs_field::pool::list_category("scenery");
    if backgrounds.is_empty() && pool.is_empty() {
        ui.weak(
            "Drop PNGs into global/images/scenes (backgrounds, author at 880x470) \
             or global/images/scenery (props)",
        );
    }
    ui.add(egui::TextEdit::singleline(&mut stage.prop_filter).hint_text("filter").desired_width(140.0));
    let filter = stage.prop_filter.to_ascii_lowercase();
    let mut add_request: Option<String> = None;
    egui::ScrollArea::vertical().id_salt("scene_pool").max_height(110.0).show(ui, |ui| {
        for image in &pool {
            if !filter.is_empty() && !image.pool_path.to_ascii_lowercase().contains(&filter) {
                continue;
            }
            ui.horizontal(|ui| {
                if ui.small_button("Add").clicked() {
                    add_request = Some(image.pool_path.clone());
                }
                ui.label(image.display_label());
            });
        }
    });
    if let Some(image) = add_request {
        stage.add_prop(image);
    }

    let placed: Vec<String> =
        stage.scene().props.iter().map(|p| p.image.rsplit('/').next().unwrap_or(&p.image).to_string()).collect();
    let mut remove_request: Option<usize> = None;
    for (k, name) in placed.iter().enumerate() {
        ui.horizontal(|ui| {
            if ui.small_button("x").on_hover_text("Remove").clicked() {
                remove_request = Some(k);
            }
            let selected = stage.selected_prop == Some(k);
            if ui.selectable_label(selected, name).on_hover_text("Click again to deselect").clicked() {
                stage.selected_prop = (!selected).then_some(k);
            }
        });
    }
    if let Some(k) = remove_request {
        stage.remove_prop(k);
    }
    if let Some(k) = stage.selected_prop {
        if let Some(prop) = stage.scene().props.get(k).cloned() {
            // Edit copies; only a real change re-points the scene Arc.
            let mut edited = prop.clone();
            ui.horizontal(|ui| {
                ui.label("x");
                ui.add(egui::DragValue::new(&mut edited.x).speed(2.0));
                ui.label("z");
                ui.add(egui::DragValue::new(&mut edited.z).speed(0.02));
            });
            ui.add(egui::Slider::new(&mut edited.scale, 0.1..=5.0).text("scale"));
            ui.checkbox(&mut edited.shadow, "Contact shadow")
                .on_hover_text("Off by default -- most scenery paints its own grounding");
            ui.weak("Drag on the stage to move the selected prop");
            if edited != prop {
                stage.scene_mut().props[k] = edited;
            }
            stage.clamp_prop(k);
        }
    }
    calibrate_scenery
}

/// New-scene naming: explicit binding + typed name, previewed and validated
/// before it becomes the save target. Room facts (from the command line)
/// only PREFILL the inputs.
fn new_scene_ui(stage: &mut StageState, ui: &mut egui::Ui) {
    egui::CollapsingHeader::new("New scene...").default_open(false).show(ui, |ui| {
        let ns = &mut stage.new_scene;
        ui.horizontal_wrapped(|ui| {
            ui.label("Bind by");
            for (binding, label) in [
                (SceneBinding::RoomUid, "Room uid"),
                (SceneBinding::RoomTitle, "Room title"),
                (SceneBinding::Location, "Location"),
                (SceneBinding::Default, "Default"),
            ] {
                if ui.selectable_label(ns.binding == binding, label).clicked() {
                    ns.set_binding(binding);
                }
            }
        });
        match ns.binding {
            SceneBinding::RoomUid => {
                ui.horizontal(|ui| {
                    ui.label("Room uid");
                    ui.add(egui::TextEdit::singleline(&mut ns.uid_text).hint_text("e.g. 47009").desired_width(80.0));
                    ui.label("garnish");
                    ui.add(
                        egui::TextEdit::singleline(&mut ns.text).hint_text("optional room name").desired_width(160.0),
                    );
                });
            }
            SceneBinding::RoomTitle | SceneBinding::Location => {
                ui.horizontal(|ui| {
                    ui.label(if ns.binding == SceneBinding::RoomTitle { "Room title" } else { "Location" });
                    ui.add(
                        egui::TextEdit::singleline(&mut ns.text).hint_text("exact in-game text").desired_width(220.0),
                    );
                });
            }
            SceneBinding::Default => {
                ui.weak("The fallback scene every unbound room uses.");
            }
        }
        match ns.preview() {
            Ok(stem) => {
                ui.label(format!("File: global/scenes/{stem}.toml"));
                let exists = scenes::scene_exists(&stem);
                let mut adopt = false;
                let mut load_existing = false;
                if exists {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        format!("Scene '{}' already exists -- Save will REPLACE it.", scenes::display_name(&stem)),
                    );
                    ui.horizontal(|ui| {
                        adopt = ui
                            .button("Use name (replace on save)")
                            .on_hover_text("The stage carries over; the next Save overwrites the existing scene file")
                            .clicked();
                        load_existing = ui.button("Load existing instead").clicked();
                    });
                } else {
                    adopt = ui
                        .button("Use name (save-as)")
                        .on_hover_text("The stage carries over; Save writes a new scene file")
                        .clicked();
                }
                if adopt {
                    stage.adopt_scene_name(stem);
                } else if load_existing {
                    stage.load_scene_by_name(&stem);
                }
            }
            Err(err) => {
                ui.colored_label(ui.visuals().error_fg_color, err);
            }
        }
    });
}

// ----------------------------------------------------------------- camera

fn camera_ui(stage: &mut StageState, ui: &mut egui::Ui) {
    use camera_limits::*;
    ui.heading("Camera");
    // Studio-only tuning; the game never mutates live params. Edits share
    // the loader's bounds (`camera_limits`): the drags are ranged AND the
    // whole camera re-sanitizes after every frame's edits, so a value that
    // would reload differently (or a typed NaN) never reaches the renderer
    // or a saved scene.
    {
        let params = &mut stage.field.params;
        let before = params.clone();
        egui::Grid::new("stage_camera").num_columns(2).show(ui, |ui| {
            let row = |ui: &mut egui::Ui, label: &str, value: &mut f32, speed: f64, range: (f32, f32)| {
                ui.label(label);
                ui.add(egui::DragValue::new(value).speed(speed).range(range.0..=range.1));
                ui.end_row();
            };
            row(ui, "focal", &mut params.focal, 2.0, FOCAL);
            row(ui, "cam_h", &mut params.cam_h, 0.02, EYE_HEIGHT);
            row(ui, "z0", &mut params.z0, 0.02, NEAR_DEPTH);
            row(ui, "dz", &mut params.dz, 0.02, ROW_DEPTH);
            row(ui, "horizon", &mut params.horizon, 1.0, HORIZON);
            row(ui, "cell_w", &mut params.cell_w, 0.01, CELL_WIDTH);
        });
        params.sanitize_camera(&before);
    }
    ui.horizontal_wrapped(|ui| {
        // Reset semantics are explicit: built-in solver defaults, or the
        // values this scene would INHERIT from the default scene layer
        // (only offered for a non-default scene).
        if ui.button("Reset camera (built-in)").on_hover_text("The solver's built-in default camera").clicked() {
            stage.reset_camera_builtin();
        }
        if stage.has_inherited_camera()
            && ui
                .button("Reset camera (inherited)")
                .on_hover_text(
                    "The camera this scene inherits from the default scene layer -- what unpinned keys resolve to",
                )
                .clicked()
        {
            stage.reset_camera_inherited();
        }
    });
}

fn solver_ui(stage: &mut StageState, ui: &mut egui::Ui) {
    ui.checkbox(&mut stage.show_solver_tuning, "Show solver tuning (testing)");
    if !stage.show_solver_tuning {
        return;
    }
    ui.heading("Solver");
    ui.weak("Placement tunables. Only future arrivals move -- permanence.");
    let solver = &mut stage.field.params.solver;
    ui.checkbox(&mut solver.zone_on, "Spawn ellipse (off = grid)");
    egui::Grid::new("stage_solver").num_columns(2).show(ui, |ui| {
        let float = |ui: &mut egui::Ui, label: &str, value: &mut f32, speed: f64, lo: f32, hi: f32| {
            ui.label(label);
            ui.add(egui::DragValue::new(value).speed(speed).range(lo..=hi));
            ui.end_row();
        };
        float(ui, "zone_inset", &mut solver.zone_inset, 0.005, 0.0, 0.45);
        float(ui, "centre_pull", &mut solver.centre_pull, 0.01, 0.0, 5.0);
        ui.label("depth_samples");
        ui.add(egui::DragValue::new(&mut solver.depth_samples).range(1..=41));
        ui.end_row();
        float(ui, "depth_jitter", &mut solver.depth_jitter, 0.01, 0.0, 1.0);
        float(ui, "lateral_jitter", &mut solver.lateral_jitter, 0.01, 0.0, 1.0);
        float(ui, "depth_spread", &mut solver.depth_spread, 0.01, 0.0, 5.0);
        float(ui, "row_band_push", &mut solver.row_band_push, 0.02, 0.0, 10.0);
        float(ui, "row_band_px", &mut solver.row_band_px, 0.5, 1.0, 200.0);
        float(ui, "occlusion_cap", &mut solver.occlusion_cap, 0.005, 0.0, 0.95);
        float(ui, "variation", &mut solver.variation, 0.01, 0.0, 2.0);
        float(ui, "fall_reserve", &mut solver.fall_reserve, 0.01, 0.0, 5.0);
        ui.label("relax_steps");
        ui.add(egui::DragValue::new(&mut solver.relax_steps).range(0..=10));
        ui.end_row();
    });
    ui.checkbox(&mut solver.seed_front, "First arrival front + centre");
    ui.checkbox(&mut solver.fall_reserve_hard, "Fall envelope is a hard bound");
    ui.checkbox(&mut solver.shuffle_ties, "Shuffle tie candidates");
    ui.horizontal(|ui| {
        ui.label("separation");
        ui.selectable_value(&mut solver.separation_basis, SeparationBasis::Contact, "contact");
        ui.selectable_value(&mut solver.separation_basis, SeparationBasis::Card, "card");
    });
    if ui.button("Reset solver").clicked() {
        *solver = Default::default();
    }
}

// ------------------------------------------------------------------- cast

/// Every published puppet, one Spawn button each. The Cast below is built
/// from the art pool and sprite art beats the puppet, so this is the direct
/// way to look at one: it spawns a real creature of that family that has no
/// art.
fn puppets_ui(stage: &mut StageState, ui: &mut egui::Ui) {
    egui::CollapsingHeader::new("Puppets").default_open(true).show(ui, |ui| {
        let set = gs_field::puppets::puppets();
        let mut spawn: Option<String> = None;
        for family in set.families() {
            let Some(rig) = set.get(family) else { continue };
            ui.horizontal(|ui| {
                if ui.small_button("Spawn").on_hover_text("Spawn a creature of this family, drawn as this puppet whatever art the pool has").clicked() {
                    spawn = Some(family.to_string());
                }
                ui.label(family);
                let body = if rig.body_type.is_empty() { "?" } else { rig.body_type.as_str() };
                match rig.source {
                    gs_field::puppets::PuppetSource::Embedded => ui.weak(format!("{body}, built in")),
                    gs_field::puppets::PuppetSource::Override => ui
                        .weak(format!("{body}, pool"))
                        .on_hover_text("From the pool's global/puppets/ (a rig save puts it there); it replaces any built-in one"),
                };
            });
        }
        ui.weak("Save a rig in the Rigs tab to build its puppet.");
        if let Some(family) = spawn {
            stage.spawn_puppet(&family);
        }
    });
}

fn cast_ui(stage: &mut StageState, ui: &mut egui::Ui) {
    puppets_ui(stage, ui);
    ui.separator();
    ui.heading("Cast");
    ui.weak("Built from the ART POOL, so everything here draws as its sprite. Use Puppets above, or a creature's \"Draw as puppet\", to see the 3D puppet.");
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut stage.filter).hint_text("filter").desired_width(140.0));
        if ui
            .button("Add generic")
            .on_hover_text(
                "Spawn an anonymous \"training dummy\". It has no art, so it draws as the humanoid puppet.",
            )
            .clicked()
        {
            stage.spawn_named("training dummy", Some("dummy".to_string()));
        }
    });
    if stage.cast.is_empty() {
        ui.weak(
            "No creature art in the pool (global/images/creatures). \"Add generic\" still works: \
             with no art it draws as its family's 3D puppet.",
        );
    }
    spawn_by_name_ui(stage, ui);
    let filter = stage.filter.to_ascii_lowercase();
    let mut spawn_request: Option<(String, String)> = None;
    let mut cast_row = |ui: &mut egui::Ui, entry: &cast::CastEntry| {
        ui.horizontal(|ui| {
            if ui.small_button("Spawn").clicked() {
                spawn_request = Some((entry.display.clone(), entry.noun.clone()));
            }
            ui.label(&entry.display);
        });
    };
    egui::ScrollArea::vertical().id_salt("stage_cast").max_height(160.0).show(ui, |ui| {
        if filter.is_empty() {
            // Grouped by bestiary area (multi-area creatures list under
            // each; unknowns under "Other").
            for (area, entries) in cast::by_area(&stage.cast) {
                egui::CollapsingHeader::new(format!("{area} ({})", entries.len())).id_salt(("cast_area", area)).show(
                    ui,
                    |ui| {
                        for entry in entries {
                            cast_row(ui, entry);
                        }
                    },
                );
            }
        } else {
            // Filter flattens the groups to matches.
            for entry in &stage.cast {
                if entry.display.to_ascii_lowercase().contains(&filter) {
                    cast_row(ui, entry);
                }
            }
        }
    });
    if let Some((display, noun)) = spawn_request {
        stage.spawn(&display, Some(noun));
    }

    ui.separator();
    ui.heading("Roster");
    ui.horizontal(|ui| {
        if ui.button("Clear stage").clicked() {
            stage.clear();
        }
    });
    let roster: Vec<(String, String)> = stage.creatures().iter().map(|c| (c.id.clone(), c.name.clone())).collect();
    let mut remove_request: Option<String> = None;
    for (id, name) in &roster {
        ui.horizontal(|ui| {
            if ui.small_button("x").on_hover_text("Remove").clicked() {
                remove_request = Some(id.clone());
            }
            if ui.selectable_label(stage.selected.as_deref() == Some(id), name).clicked() {
                stage.selected = Some(id.clone());
            }
        });
    }
    if let Some(id) = remove_request {
        stage.remove(&id);
    }

    if let Some(id) = stage.selected.clone() {
        ui.separator();
        simulator_ui(stage, ui, &id);
    }
}

/// Spawn by bestiary NAME, art or no art. The Cast list above is built from
/// the art pool, so every entry in it has art by definition, and sprite art
/// beats the family puppet -- without this box there is no way to put a
/// canine / elephantid / suine puppet on the stage short of emptying the
/// pool. Suggestions come from the bestiary so the names are real creatures,
/// but anything typed can be spawned.
fn spawn_by_name_ui(stage: &mut StageState, ui: &mut egui::Ui) {
    egui::CollapsingHeader::new("Spawn by name (art or no art)").default_open(false).show(ui, |ui| {
        ui.weak("Any creature from the bestiary. With no art in the pool it draws as its family's 3D puppet.");
        let mut submit = false;
        ui.horizontal(|ui| {
            let box_id = ui.make_persistent_id("spawn_by_name");
            let response = ui.add(
                egui::TextEdit::singleline(&mut stage.spawn_name)
                    .id(box_id)
                    .hint_text("creature name, e.g. timber wolf")
                    .desired_width(180.0),
            );
            submit = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            submit |= ui
                .add_enabled(!stage.spawn_name.trim().is_empty(), egui::Button::new("Spawn"))
                .on_disabled_hover_text("Type a creature name first")
                .clicked();
        });

        // What the typed name would get right now -- the same resolution
        // the renderer runs, so this never disagrees with the card.
        let typed = stage.spawn_name.trim().to_string();
        if !typed.is_empty() {
            let hints = stage.spawn_hints.get(typed.clone(), || cast::spawn_hints(&typed, 12)).clone();
            if hints.has_art {
                ui.weak(format!("'{typed}': has art in the pool -- draws as its sprite."));
            } else {
                let text = match &hints.puppet {
                    Some(family) => format!("'{typed}': NO art in the pool -- draws as the {family} puppet."),
                    None => format!("'{typed}': NO art and no puppet fits -- draws as a placeholder."),
                };
                ui.colored_label(ui.visuals().warn_fg_color, text);
            }

            // Bestiary suggestions: click one to fill the box.
            if !hints.picks.is_empty() {
                egui::ScrollArea::vertical().id_salt("spawn_suggestions").max_height(120.0).show(ui, |ui| {
                    for (pick, art) in &hints.picks {
                        let mark = if *art { "art" } else { "puppet" };
                        let family = pick.family.as_deref().unwrap_or("?");
                        let row = ui
                            .selectable_label(false, format!("{}   [{mark}]", pick.name))
                            .on_hover_text(format!("noun '{}', family {family}", pick.noun));
                        if row.clicked() {
                            stage.spawn_named(&pick.name, Some(pick.noun.clone()));
                        }
                    }
                });
                ui.weak("[art] = a sprite in the pool wins   [puppet] = no sprite, so the 3D puppet draws");
            }
        }

        if submit {
            let name = stage.spawn_name.trim().to_string();
            if stage.spawn_named(&name, None) {
                stage.spawn_name.clear();
            }
        }
    });
}

/// State editor for one spawned creature: crtrStatus flags, class bools,
/// wounds, health, targeting, mounting.
fn simulator_ui(stage: &mut StageState, ui: &mut egui::Ui, id: &str) {
    let Some(creature) = stage.creature(id) else {
        return;
    };
    ui.heading(&creature.name);
    // A snapshot to draw from; every edit goes through StageState.
    let flags = creature.flags.clone().unwrap_or_else(|| CreatureFlags { hostile: true, ..Default::default() });

    let mut want_target = stage.is_target(id);
    if ui.checkbox(&mut want_target, "Target").changed() {
        stage.set_target(id, want_target);
    }
    let mut puppet = stage.is_forced_puppet(id);
    if ui
        .checkbox(&mut puppet, "Draw as puppet")
        .on_hover_text("Draw this creature as its family's 3D puppet even though sprite art resolves for it")
        .changed()
    {
        stage.set_forced_puppet(id, puppet);
    }
    ui.horizontal(|ui| {
        let custom = stage.facing(id);
        let mut deg = custom.unwrap_or(gs_field_egui::creature_puppet::FIELD_YAW_DEG);
        let slider = ui
            .add(egui::Slider::new(&mut deg, -180.0..=180.0).step_by(5.0).suffix(" deg").text("Facing"))
            .on_hover_text("Turns this creature's 3D puppet (0 = facing you, 90 = its front to the right). Sprites are flat and do not turn.");
        if slider.changed() {
            stage.set_facing(id, Some(deg));
        }
        if ui.add_enabled(custom.is_some(), egui::Button::new("Default")).on_hover_text("Back to the renderer's own quarter-turn").clicked() {
            stage.set_facing(id, None);
        }
    });

    ui.label("Status");
    ui.horizontal_wrapped(|ui| {
        for (_, canonical) in CRTR_STATUS_FLAGS.iter() {
            let mut on = flags.statuses.iter().any(|s| s == canonical);
            if ui.checkbox(&mut on, *canonical).changed() {
                stage.set_status(id, canonical, on);
            }
        }
    });

    ui.label("Class");
    ui.horizontal_wrapped(|ui| {
        for class in ClassFlag::ALL {
            let mut on = class.get(&flags);
            if ui.checkbox(&mut on, class.label()).changed() {
                stage.set_class(id, class, on);
            }
        }
    });
    if let Some((rider, mount)) = stage.pair() {
        if rider == id || mount == id {
            ui.weak(if rider == id { "Riding the first flagged mount" } else { "Carrying the first flagged rider" });
        }
    }

    let mut has_health = flags.health.is_some();
    if ui.checkbox(&mut has_health, "Health bar").changed() {
        stage.set_health_bar(id, has_health);
    }
    if let Some(mut hp) = flags.health.filter(|_| has_health) {
        if ui.add(egui::Slider::new(&mut hp, 0..=100).text("health")).changed() {
            stage.set_health(id, hp);
        }
    }

    ui.label("Wounds (rank 0-3)");
    egui::Grid::new("stage_wounds").num_columns(2).show(ui, |ui| {
        for part in INJURY_AREAS.iter() {
            let rank = stage.wound_rank(id, part);
            ui.label(*part);
            ui.horizontal(|ui| {
                for r in 0u8..=3 {
                    if ui.selectable_label(rank == r, format!("{r}")).clicked() {
                        stage.set_wound(id, part, r);
                    }
                }
            });
            ui.end_row();
        }
    });
}

// ------------------------------------------------------------------ stage

fn field_ui(stage: &mut StageState, ui: &mut egui::Ui, art: &SharedCreatureArt, panel: StagePanel) {
    // On the Scenes tab a selected prop owns the drag (as in VellumFE); on
    // the Preview tab drags always move creatures.
    let dragging_prop = panel == StagePanel::Scenes && stage.selected_prop.is_some();
    ui.horizontal(|ui| {
        ui.checkbox(&mut stage.show_grid, "Grid");
        ui.checkbox(&mut stage.show_order, "Draw order");
        ui.separator();
        ui.weak(if dragging_prop {
            "drag: move the selected prop   (click it in the list again to drag creatures)"
        } else {
            "click a card: target it   drag a card: move it"
        });
    });
    // The renderer allocates the remaining region; remember it for the drag
    // interact below (same origin + size).
    let rect = egui::Rect::from_min_size(ui.next_widget_position(), ui.available_size());
    // The Stage feeds the renderer its own toggles and scene directly;
    // creature state comes from the fabricated roster. There is no game
    // state: conditions go through the sandbox evaluator and no creature
    // has a derived-effect severity.
    gs_field_egui::creature_puppet::set_forced_puppets(ui.ctx(), stage.forced_puppets().cloned());
    gs_field_egui::creature_puppet::set_puppet_facings(ui.ctx(), stage.facings().map(|(id, d)| (id.clone(), *d)));
    let click = {
        let view = CreatureFieldView {
            field: &stage.field,
            creatures: stage.creatures(),
            current_target: stage.current_target(),
            eval_condition: Box::new(eval_condition),
            effect_severity: Box::new(|_, _| None),
            show_grid: stage.show_grid,
            show_order: stage.show_order,
            creature_art: Some(art),
            scene: Some(stage.scene().as_ref()),
        };
        render_creature_field_content(&view, ui)
    };
    let stage_pos = |pos: egui::Pos2| creature_field_stage_pos(rect, pos);
    if dragging_prop {
        // Drag-to-place: while a prop is selected, dragging the field view
        // moves it on the ground plane (screen -> stage -> ground, through
        // the renderer's own mapping and the solver's inverse projection).
        let response = ui.interact(rect, ui.id().with("scene_prop_drag"), egui::Sense::drag());
        if response.dragged() {
            if let Some(pos) = response.interact_pointer_pos() {
                let (sx, sy) = stage_pos(pos);
                stage.drag_selected_prop_to(sx, sy);
            }
        }
    } else {
        // Creature drag-to-place: grab a card (topmost = nearest under the
        // pointer) and drag its ground point around the floor.
        let response = ui.interact(rect, ui.id().with("stage_creature_drag"), egui::Sense::drag());
        if response.drag_started() {
            match response.interact_pointer_pos() {
                Some(pos) => {
                    let (sx, sy) = stage_pos(pos);
                    stage.begin_creature_drag(sx, sy);
                }
                None => stage.end_creature_drag(),
            }
        }
        if response.dragged() {
            if let Some(pos) = response.interact_pointer_pos() {
                let (sx, sy) = stage_pos(pos);
                stage.drag_creature_to(sx, sy);
            }
        }
        if response.drag_stopped() {
            stage.end_creature_drag();
        }
    }
    // The renderer reports which card was clicked; the Stage targets it
    // locally instead of sending a command anywhere.
    if let Some(click) = click {
        stage.target_clicked(click.exist_id);
    }
}
