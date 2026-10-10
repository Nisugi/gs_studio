//! The Preview stage's egui layer: VellumFE's Studio "Stage", drawn by the
//! renderer the clients use (`creature_field_egui`). It draws `stage::StageState`
//! and forwards input to it; every rule lives in `crate::stage`.
//!
//! One tool, two tabs. **Scenes** and **Preview** share this one
//! `StageTool` (one sandbox, one stage in the middle); the tab only picks
//! which side panel is showing -- scene authoring (scene, props, camera,
//! solver) or the cast (cast, roster, per-creature simulator).
//!
//! Ported from VellumFE (GPL-3.0-or-later) `src/frontend/gui/studio.rs` at
//! commit 60c84a79.

use creature_field::condition::INJURY_AREAS;
use creature_field::creature::CRTR_STATUS_FLAGS;
use creature_field::scenes::{self, SceneBinding};
use creature_field::solver::{camera_limits, SeparationBasis};
use creature_field::{Creature, CreatureFlags};
use creature_field_egui::{
    creature_field_stage_pos, render_creature_field_content, CreatureArtStore, CreatureFieldView, SharedCreatureArt,
    WantedCreature,
};

use crate::stage::color::{hex_rgb, parse_hex_rgb, NEUTRAL_SLATE};
use crate::stage::eval::eval_condition;
use crate::stage::{cast, ClassFlag, SceneAction, StageState, StudioRoomContext};

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
    lighting: creature_field::lighting::GlobalLighting,
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
            family: c.noun.as_deref().and_then(creature_field::cards::family_for_noun),
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
            // Creatures in outfits (data/outfits/outfits.toml): the dressed
            // puppet path.
            stage.spawn_named("grim gigas skald", Some("skald".to_string()));
            stage.spawn_named("squamous reptilian mutant", Some("mutant".to_string()));
            stage.set_status("studio-1", "stunned", true);
            stage.set_wound("studio-1", "head", 2);
            stage.set_health_bar("studio-1", true);
            stage.set_target("studio-1", true);
            if let Some(name) = scenes::list_scenes().first() {
                stage.load_scene_by_name(name);
            }
            // GS_STUDIO_SMOKE_LIGHTS=1: the scene lit, with a torch at the
            // front left and a wisp to the right (in memory only), for
            // checking lamps and the v2 pass with no one at the window;
            // =all adds every other kind: a campfire, a glow, and a lantern
            // at the back right throwing its beam left, and makes the wisp
            // an Arcane one.
            if let Some(which) = std::env::var_os("GS_STUDIO_SMOKE_LIGHTS") {
                use creature_field::lighting::{LampKind, SceneLight};
                // GS_STUDIO_SUN=<azimuth>,<elevation>: where its sun is
                let sun = std::env::var("GS_STUDIO_SUN").ok().and_then(|v| {
                    let (a, e) = v.split_once(',')?;
                    Some(creature_field::lighting::SunDir { azimuth: a.trim().parse().ok()?, elevation: e.trim().parse().ok()? })
                });
                stage.scene_mut().light = Some(SceneLight { sun, ..SceneLight::default() });
                let mut lamps = vec![(LampKind::Torch, 170.0, 430.0), (LampKind::Wisp, 640.0, 330.0)];
                let all = which == "all";
                if all {
                    lamps.extend([(LampKind::Campfire, 300.0, 300.0), (LampKind::Glow, 80.0, 280.0), (LampKind::Lantern, 820.0, 290.0)]);
                }
                for (kind, sx, sy) in lamps {
                    stage.add_light(kind);
                    stage.drag_selected_light_to(sx, sy);
                    let lamp = stage.selected_light.and_then(|k| stage.scene_mut().lights.get_mut(k));
                    match (all, kind, lamp) {
                        (true, LampKind::Wisp, Some(l)) => l.color = creature_field_egui::field_gpu::MOTE_PRESETS[1].1[1].into(),
                        (true, LampKind::Lantern, Some(l)) => l.aim = 195.0,
                        _ => {}
                    }
                }
            }
            eprintln!(
                "gs-studio: smoke stage: {} cast, {} spawned, scene '{}'",
                stage.cast.len(),
                stage.creatures().len(),
                stage.scene_name
            );
        }
        // Puppets with no one at the Spawn buttons: GS_STUDIO_PUPPETS=
        // <form>[/<creature name>][,...] puts each form's puppet on the
        // stage (forced, as the Puppets list's Spawn does; with a creature
        // name, that creature in its own skin, as the tree's texture rows
        // do), since a pool with default art otherwise draws every
        // creature as a sprite.
        // GS_STUDIO_SCENE=<name>: that saved scene on the stage.
        if let Ok(name) = std::env::var("GS_STUDIO_SCENE") {
            // (a scene it cannot load says so in the status log)
            stage.load_scene_by_name(name.trim());
        }
        if let Ok(forms) = std::env::var("GS_STUDIO_PUPPETS") {
            for form in forms.split(',').map(str::trim).filter(|f| !f.is_empty()) {
                match form.split_once('/') {
                    Some((form, name)) => {
                        let noun = name.rsplit(' ').next().unwrap_or(name);
                        stage.spawn_puppet_wearing(form, Some((name, noun)));
                    }
                    None => {
                        stage.spawn_puppet(form);
                    }
                }
            }
        }
        // A splat check with no one at the checkbox: GS_STUDIO_SPLATS=1
        // draws every puppet as a splat cloud (the Splats checkbox).
        if std::env::var_os("GS_STUDIO_SPLATS").is_some() {
            stage.show_splats = true;
        }
        // GS_STUDIO_GPU_SKINS=0: the puppets' skins on the CPU (GPU skins off)
        if let Ok(v) = std::env::var("GS_STUDIO_GPU_SKINS") {
            stage.gpu_skins = v.trim() != "0";
        }
        // A lighting check with no one at the slider: GS_STUDIO_LIGHT_HOUR
        // holds the stage at that game hour (23 = night).
        if let Some(hour) = std::env::var("GS_STUDIO_LIGHT_HOUR").ok().and_then(|h| h.trim().parse::<f32>().ok()) {
            stage.light_hour = Some(hour.rem_euclid(24.0));
        }
        // GS_STUDIO_WEATHER=<kind>[:<intensity>] sets the weather (Rain,
        // Storm, Snow, Sleet, Blood rain, Fog, Sand; any case).
        if let Ok(w) = std::env::var("GS_STUDIO_WEATHER") {
            let (kind, amount) = w.split_once(':').map_or((w.as_str(), None), |(k, a)| (k, a.trim().parse::<f32>().ok()));
            let kinds = creature_field_egui::field_weather::WeatherKind::ALL;
            if let Some(k) = kinds.iter().position(|k| k.label().eq_ignore_ascii_case(kind.trim())) {
                stage.weather_kind = k + 1;
                if let Some(a) = amount {
                    stage.weather_intensity = a.clamp(0.0, 1.0);
                }
            }
        }
        // GS_STUDIO_SUN_LIT=1 shades the puppets by the scene (Lit by the scene).
        if std::env::var_os("GS_STUDIO_SUN_LIT").is_some() {
            stage.sun_lit_skins = true;
        }
        // ... and GS_STUDIO_LIGHT_CLOUDS / GS_STUDIO_LIGHT_MOON set its sky (0..1).
        let sky = |k: &str| std::env::var(k).ok().and_then(|v| v.trim().parse::<f32>().ok()).map(|v| v.clamp(0.0, 1.0));
        if let Some(v) = sky("GS_STUDIO_LIGHT_CLOUDS") {
            stage.light_clouds = v;
        }
        if let Some(v) = sky("GS_STUDIO_LIGHT_MOON") {
            stage.light_moon = v;
        }
        // GS_STUDIO_GAME_SKY=1 takes the moons from the game's calendar,
        // =<unix seconds> at that instant; GS_STUDIO_LORNONS_EVE=1 on Eorgaen 31.
        if let Ok(v) = std::env::var("GS_STUDIO_GAME_SKY") {
            stage.game_sky = true;
            stage.sky_at = v.trim().parse::<f64>().ok().filter(|t| *t > 1.0);
        }
        if std::env::var_os("GS_STUDIO_LORNONS_EVE").is_some() {
            stage.lornons_eve = true;
        }
        // GS_STUDIO_FACING=<n|ne|e|...|w|nw>: the way the view faces
        if let Some(b) = std::env::var("GS_STUDIO_FACING").ok().and_then(|f| creature_field::sky::bearing_of(&f)) {
            stage.facing = Some(b);
        }
        let lighting = creature_field::lighting::GlobalLighting::load();
        art.set_global_lighting(lighting.clone());
        StageTool { stage, art, lighting }
    }

    /// A line for the stage's own status log (the import report, say).
    /// The facts a scene report needs of the v2 pass's last frame: its
    /// camera, which lamps got in and cast shadows, each lamp's reach, and
    /// (when timed) what the frame and each lamp cost.
    pub fn pass_facts(
        &self,
        snap: &creature_field_egui::field_gpu_stage::PassSnapshot,
        timings: Option<&creature_field_egui::field_gpu_stage::PassTimings>,
        gpu: String,
    ) -> crate::stage::report::PassFacts {
        use crate::stage::report::{LampFacts, PassFacts};
        let lamps = self
            .stage
            .scene()
            .lights
            .iter()
            .enumerate()
            .map(|(k, lamp)| {
                let cost = timings.and_then(|t| t.lamps.get(k)).copied().unwrap_or((None, None));
                LampFacts {
                    reach_m: creature_field_egui::field_gpu_stage::lamp_range(lamp),
                    in_pass: snap.in_pass.get(k).copied().unwrap_or(false),
                    shadow_given: snap.shadow_given(k),
                    cost_ms: cost.0,
                    shadow_ms: cost.1,
                }
            })
            .collect();
        PassFacts {
            gpu,
            size: snap.size,
            frame_ms: timings.map(|t| t.whole_ms),
            k: snap.k,
            f: snap.f,
            moons: snap.moons(),
            shadow_slots: creature_field_egui::field_gpu::MAX_SHADOW_LIGHTS,
            light_slots: creature_field_egui::field_gpu::MAX_LIGHTS,
            lamps,
        }
    }

    /// The scene report (`stage::report`) at the hour on view.
    pub fn report(&self, pass: Option<&crate::stage::report::PassFacts>) -> String {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
        let hour = self.stage.light_hour.unwrap_or_else(|| crate::stage::elanthian_time::elanthian_hour_f(now));
        crate::stage::report::scene_report(&self.stage, hour, pass)
    }

    pub fn push_status(&mut self, message: String) {
        self.stage.push_status(message);
    }

    /// "Show on stage" from the rig editor: put that family's puppet up.
    pub fn spawn_puppet(&mut self, family: &str) {
        self.stage.spawn_puppet(family);
    }

    /// The pool directory moved: re-root the art, re-scan the cast.
    pub fn pool_changed(&mut self) {
        let root = creature_field::pool_dirs::global_images_dir().unwrap_or_default();
        self.art.reset_with_root(&root);
        self.stage.pool_changed();
    }

    /// A calibrator saved: art and sidecars changed on disk. Reloads the
    /// art store -- which also drops creature_field's geometry-calibration store,
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

    /// The unsaved-scene question, drawn by the app over whichever tab is
    /// showing (a window close asks it from anywhere).
    pub fn modal_prompts(&mut self, ctx: &egui::Context) {
        let stage = &mut self.stage;
        let Some(action) = stage.scene_pending.clone() else {
            return;
        };
        let name = stage.scene_name.trim().to_owned();
        let modal = egui::Modal::new(egui::Id::new("scene_discard")).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.heading("Unsaved scene");
            let what = if name.is_empty() {
                "The stage scene (not named yet)".to_owned()
            } else {
                format!("The scene '{}'", scenes::display_name(&name))
            };
            ui.label(format!("{what} has unsaved changes. Discard them and {}?", action.describe()));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let save = ui
                    .add_enabled(!name.is_empty(), egui::Button::new("Save first"))
                    .on_disabled_hover_text("Name it under New scene (Scenes tab) to save it");
                if save.clicked() {
                    stage.save_scene_then_confirm();
                }
                if ui.button("Discard").clicked() {
                    stage.confirm_scene_pending();
                }
                if ui.button("Cancel").clicked() {
                    stage.cancel_scene_pending();
                }
            });
        });
        if modal.should_close() {
            stage.cancel_scene_pending();
        }
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
        use creature_field_egui::field_weather::{set_weather, Weather, WeatherKind};
        let weather = stage.weather_kind.checked_sub(1).and_then(|k| WeatherKind::ALL.get(k)).map(|&kind| Weather {
            kind,
            intensity: stage.weather_intensity,
            wind: stage.weather_wind,
        });
        // the sky's own cloud, or the weather's if it brings more
        let clouds = stage.light_clouds.max(weather.map_or(0.0, |w| w.cloud_cover()));
        self.art.set_light_clock(creature_field_egui::creature_light::LightClock {
            game_hour: Some(game_hour),
            strength: stage.light_strength,
            outdoor_guess: None,
            sky: stage.sky(now, game_hour, clouds),
            sun_lit: stage.sun_lit_skins,
        });
        self.art.prepare_scene_art(&ctx, stage.scene());
        set_weather(&ctx, weather);
        creature_field_egui::field_occluders::set_show_occluders(&ctx, stage.show_occluders && panel == StagePanel::Scenes);
        creature_field_egui::creature_puppet::set_puppet_props(&ctx, stage.show_props);
        creature_field_egui::creature_puppet::set_puppet_breath(&ctx, stage.show_breath);
        creature_field_egui::creature_puppet::set_puppet_splats(&ctx, stage.show_splats);
        creature_field_egui::creature_puppet::set_puppet_gpu_skin(&ctx, stage.gpu_skins);
        creature_field_egui::creature_puppet::set_puppet_splat_params(&ctx, stage.splat_params);
        if self.art.lighting_busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        } else if stage.scene().light.is_some() && stage.light_hour.is_none() {
            // The live clock moves dawn and dusk along.
            ctx.request_repaint_after(std::time::Duration::from_secs(20));
        }
        let auto = stage.scene().background.as_deref().and_then(|bg| self.art.scene_auto_colours(bg));
        let lighting = &mut self.lighting;
        let mut lighting_changed = false;
        let art = self.art.creature_art();

        egui::Panel::right("stage_panel").default_size(300.0).min_size(240.0).show(ui, |ui| {
            // Scenes holds what the scene file keeps; Preview the moment it
            // is seen in (the hour, the sky, the weather) and who is there.
            // "Time and weather" is the same section on both, open or shut
            // as one.
            outline_ui(stage, ui, game_hour);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match panel {
                StagePanel::Scenes => {
                    super::section(ui, "stage_scene", "Scene", |ui| scene_ui(stage, ui));
                    calibrate_scenery = super::section(ui, "stage_props", "Props", |ui| props_ui(stage, ui)).unwrap_or(false);
                    super::section(ui, "stage_lamps", "Lamps", |ui| lights_ui(stage, ui));
                    if stage.scene().background.is_some() {
                        super::section(ui, "stage_occluders", "Masks", |ui| {
                            super::group(ui, "In front");
                            occluders_ui(stage, ui, &art);
                            super::group(ui, "Walkable");
                            walk_ui(stage, ui, &art);
                        });
                    }
                    super::section(ui, "stage_light", "Light", |ui| light_ui(stage, ui, auto.as_ref(), lighting));
                    super::section(ui, "stage_time", "Time and weather", |ui| time_ui(stage, ui, game_hour));
                    notes_section(stage, ui);
                    super::section(ui, "stage_camera", "Camera", |ui| camera_ui(stage, ui));
                    super::section(ui, "stage_solver", "Solver (testing)", |ui| solver_ui(stage, ui));
                    let every = super::section_title(ui, "Every scene");
                    super::section_rich(ui, "stage_every_scene", every, false, |ui| {
                        lighting_changed = global_light_ui(ui, lighting, stage);
                    });
                }
                StagePanel::Preview => {
                    super::section(ui, "stage_time", "Time and weather", |ui| time_ui(stage, ui, game_hour));
                    notes_section(stage, ui);
                    super::section(ui, "stage_display", "Display", |ui| display_ui(stage, ui));
                    cast_ui(stage, ui)
                }
            });
        });
        egui::CentralPanel::default().show(ui, |ui| field_ui(stage, ui, &art, panel, game_hour));
        if lighting_changed {
            self.art.set_global_lighting(self.lighting.clone());
        }
        calibrate_scenery
    }
}

impl crate::quit::QuitGuard for StageTool {
    fn approve_quit(&mut self) -> bool {
        self.stage.approve_quit()
    }
    fn quit_asking(&self) -> bool {
        self.stage.quit_asking()
    }
    fn quit_approved(&self) -> bool {
        self.stage.quit_approved()
    }
    fn cancel_quit(&mut self) {
        self.stage.cancel_quit()
    }
}

// ------------------------------------------------------------------ scene

/// The backdrop's occluder mask (`creature_field_egui::field_occluders`): what
/// the painting holds in front of creatures.
fn occluders_ui(stage: &mut StageState, ui: &mut egui::Ui, art: &creature_field_egui::creature_art::SharedCreatureArt) {
    let Some(bg) = stage.scene().background.clone() else { return };
    let mask = creature_field_egui::field_occluders::mask_path(&bg);
    let (count, size) = {
        let cache = art.lock().unwrap_or_else(|e| e.into_inner());
        (cache.occluders(&bg).map(|o| o.blobs.len()), cache.scene_background(&bg).map(|t| t.size()))
    };
    let abs = creature_field::pool_dirs::global_images_dir().ok().map(|d| d.join(&mask));
    let exists = abs.as_ref().is_some_and(|p| p.is_file());
    match (count, exists) {
        (Some(n), _) => ui.label(format!("{mask}: {n} occluder(s)")),
        (None, true) => ui.colored_label(ui.visuals().warn_fg_color, format!("{mask} holds no occluders (see the log)")),
        (None, false) => ui.weak(format!("No {mask}: nothing painted stands in front of creatures.")),
    };
    ui.weak("Paint white over what stands IN FRONT of creatures (a rock, a cactus); each blob hides the creatures behind where its bottom meets the floor.");
    ui.horizontal_wrapped(|ui| {
        if !exists {
            let create = ui
                .add_enabled(size.is_some() && abs.is_some(), egui::Button::new("Create mask"))
                .on_hover_text("Write a transparent PNG the backdrop's size to paint on (as a layer over the backdrop)");
            if create.clicked() {
                if let (Some([w, h]), Some(path)) = (size, abs.as_ref()) {
                    let written = crate::editor::texture::write_png(path, w, h, &vec![0; w * h * 4]);
                    stage.push_status(match written {
                        Ok(()) => format!("Wrote {}", path.display()),
                        Err(e) => format!("Mask not written: {e}"),
                    });
                }
            }
        }
        if ui.button("Reload mask").on_hover_text("Read the mask again after painting it").clicked() {
            art.lock().unwrap_or_else(|e| e.into_inner()).occluders.remove(&bg);
        }
        ui.checkbox(&mut stage.show_occluders, "Show").on_hover_text("Tint the occluders red on the stage");
    });
}

/// The backdrop's walk mask (`creature_field::walk`): where on the painting a
/// creature may stand.
fn walk_ui(stage: &mut StageState, ui: &mut egui::Ui, art: &creature_field_egui::creature_art::SharedCreatureArt) {
    let Some(bg) = stage.scene().background.clone() else { return };
    let mask = creature_field::walk::walk_path(&bg);
    let abs = creature_field::pool_dirs::global_images_dir().ok().map(|d| d.join(&mask));
    let exists = abs.as_ref().is_some_and(|p| p.is_file());
    match (stage.field.walk(), &stage.walk_error) {
        (_, Some(e)) => ui.colored_label(ui.visuals().warn_fg_color, e.clone()),
        (Some(m), None) => ui.label(format!("{mask}: {:.0}% walkable", m.share() * 100.0)),
        (None, None) => ui.weak(format!("No {mask}: creatures stand anywhere on the floor.")),
    };
    ui.weak("Paint black over what no creature may stand on (a wall, the river, a roof); white is ground. Props and lamps are not held to it.");
    ui.horizontal_wrapped(|ui| {
        if !exists {
            let size = art.lock().unwrap_or_else(|e| e.into_inner()).scene_background(&bg).map(|t| t.size());
            let create = ui
                .add_enabled(size.is_some() && abs.is_some(), egui::Button::new("Create mask"))
                .on_hover_text("Write a white PNG the backdrop's size (all walkable) to paint the walls and water black on");
            if create.clicked() {
                if let (Some([w, h]), Some(path)) = (size, abs.as_ref()) {
                    let written = crate::editor::texture::write_png(path, w, h, &vec![255; w * h * 4]);
                    stage.push_status(match written {
                        Ok(()) => format!("Wrote {}", path.display()),
                        Err(e) => format!("Mask not written: {e}"),
                    });
                    stage.invalidate_obstacles();
                }
            }
        }
        if ui.button("Reload mask").on_hover_text("Read the mask again after painting it; creatures arriving from now keep to it").clicked() {
            stage.invalidate_obstacles();
        }
        ui.checkbox(&mut stage.show_walk, "Show").on_hover_text("Tint red where creatures may not stand");
    });
}

/// The stage-to-screen mapping of the field drawn into `rect`, read off
/// the renderer's own inverse: two points give the scale and the origin,
/// so what is drawn over the stage can never drift from it.
fn stage_to_screen(rect: egui::Rect) -> impl Fn(f32, f32) -> egui::Pos2 {
    let s0 = creature_field_stage_pos(rect, rect.min);
    let s1 = creature_field_stage_pos(rect, rect.min + egui::vec2(100.0, 100.0));
    let scale = 100.0 / (s1.0 - s0.0).max(1e-3);
    let origin = rect.min - egui::vec2(s0.0, s0.1) * scale;
    move |x, y| origin + egui::vec2(x, y) * scale
}

/// Where the walk mask keeps creatures off, tinted red over the stage.
fn walk_overlay(stage: &StageState, ui: &mut egui::Ui, rect: egui::Rect) {
    let Some(mask) = stage.field.walk() else { return };
    let key = (mask.v.as_ptr() as usize, mask.w, mask.h);
    let id = egui::Id::new("stage_walk_overlay");
    let held: Option<((usize, usize, usize), egui::TextureHandle)> = ui.ctx().data(|d| d.get_temp(id));
    let texture = match held {
        Some((k, t)) if k == key => t,
        _ => {
            let rgba: Vec<u8> = mask.v.iter().flat_map(|&p| if p >= 128 { [0, 0, 0, 0] } else { [220, 40, 40, 110] }).collect();
            let image = egui::ColorImage::from_rgba_unmultiplied([mask.w, mask.h], &rgba);
            let t = ui.ctx().load_texture("walk mask", image, egui::TextureOptions::NEAREST);
            ui.ctx().data_mut(|d| d.insert_temp(id, (key, t.clone())));
            t
        }
    };
    let at = stage_to_screen(rect);
    let [x0, y0, x1, y1] = mask.rect;
    let place = egui::Rect::from_min_max(at(x0, y0), at(x1, y1));
    ui.painter_at(rect).image(texture.id(), place, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
}

/// The top of the stage panel, above the sections: the scene, the stage at
/// a glance ([`StageState::outline`]), and the one switch that changes how
/// all of it is drawn.
fn outline_ui(stage: &mut StageState, ui: &mut egui::Ui, game_hour: f32) {
    ui.horizontal_wrapped(|ui| {
        let name = stage.scene_name.trim();
        ui.strong(if name.is_empty() { "(unsaved scene)".to_owned() } else { scenes::display_name(name) });
        if stage.scene_dirty() {
            ui.colored_label(ui.visuals().warn_fg_color, "* unsaved");
        }
    });
    ui.weak(stage.outline(game_hour));
    // The v2 pass: lamps light the backdrop and every body on the GPU, with
    // real falloff and shadows, and it draws the sky (creature_field_egui::field_gpu_stage).
    let ctx = ui.ctx().clone();
    if creature_field_egui::field_gpu::available(&ctx) {
        let mut v2 = creature_field_egui::field_gpu_stage::enabled(&ctx);
        if ui
            .checkbox(&mut v2, "Light on the GPU (v2)")
            .on_hover_text(
                "Lamps light the backdrop and every creature from where they are, with shadows, and the sky shows its moons, \
                 stars, cloud and aurora. Scenes still need Light this scene for their night look.",
            )
            .changed()
        {
            creature_field_egui::field_gpu_stage::set_enabled(&ctx, v2);
            // remembered for the next run (settings.toml)
            if let Some(e) = crate::settings::remember(|s| s.gpu_light = Some(v2)) {
                eprintln!("gs-studio: {e}");
            }
        }
    }
    ui.separator();
}

/// Dawn, day, dusk and night, as the tables' columns.
const PHASES: [(vellum_light::Phase, &str); 4] = [
    (vellum_light::Phase::Dawn, "Dawn"),
    (vellum_light::Phase::Day, "Day"),
    (vellum_light::Phase::Dusk, "Dusk"),
    (vellum_light::Phase::Night, "Night"),
];

/// The scene's `[light]`: its colours read from the backdrop (each
/// overridable), levels, sun, sky, and its own time of day over Every
/// scene's. Niffy's "Stage editor: Light section".
fn light_ui(
    stage: &mut StageState,
    ui: &mut egui::Ui,
    auto: Option<&vellum_light::SceneColours>,
    global: &creature_field::lighting::GlobalLighting,
) {
    use creature_field::lighting::SceneLight;

    let light = stage.scene().light.clone();
    let mut edited = light.clone();
    ui.horizontal_wrapped(|ui| {
        let mut on = edited.is_some();
        if ui
            .checkbox(&mut on, "Light this scene")
            .on_hover_text("Light creatures and props to match the backdrop, and apply dawn, dusk and night outdoors")
            .changed()
        {
            edited = on.then(SceneLight::default);
        }
        if let Some(e) = edited.as_mut() {
            let mut outdoor = e.outdoor.unwrap_or(true);
            if ui.checkbox(&mut outdoor, "Outdoor").on_hover_text("Dawn, dusk and night apply, and the sky: moons, stars, cloud, aurora").changed() {
                e.outdoor = Some(outdoor);
            }
        }
    });
    match edited.as_mut() {
        Some(e) => light_body(ui, e, auto, global),
        None => {
            ui.weak("Unlit: creatures and props draw exactly as painted.");
        }
    }
    if edited != light {
        stage.scene_mut().light = edited;
    }
}

fn light_body(
    ui: &mut egui::Ui,
    e: &mut creature_field::lighting::SceneLight,
    auto: Option<&vellum_light::SceneColours>,
    global: &creature_field::lighting::GlobalLighting,
) {
    use creature_field::lighting::{LightChroma, SCENE_BOUNCE_STRENGTH, SCENE_STRENGTH};

    if auto.is_none() {
        ui.weak("No backdrop to read: \"auto\" values use a neutral daylight.");
    }
    super::group(ui, "Colour");
    egui::Grid::new("light_colour").num_columns(3).show(ui, |ui| {
        light_colour_row(ui, "Key", &mut e.key, auto.map(|a| a.key.as_str()), "Main light: the faces turned toward it");
        light_colour_row(ui, "Ambient", &mut e.ambient, auto.map(|a| a.ambient.as_str()), "Fill light: the shadowed side (sky or ceiling)");
        light_colour_row(ui, "Ground", &mut e.bounce, auto.map(|a| a.bounce.as_str()), "Ground bounce: rises into the lower shadows");
        ui.label("Saturation").on_hover_text("How colourful creatures stay: a misty backdrop mutes them to match");
        let auto_c = auto.map_or(0.9, |a| a.chroma);
        let mut v = match e.chroma {
            LightChroma::Auto => auto_c,
            LightChroma::Value(v) => v,
        };
        if ui.add(egui::DragValue::new(&mut v).range(0.0..=1.2).speed(0.01).fixed_decimals(2)).changed() {
            e.chroma = LightChroma::Value(v);
        }
        if e.chroma.is_auto() {
            ui.weak("auto");
        } else if ui.small_button("x").on_hover_text("Back to auto: read from the backdrop").clicked() {
            e.chroma = LightChroma::Auto;
        }
        ui.end_row();
    });

    super::group(ui, "Levels");
    let outdoor = e.outdoor.unwrap_or(true);
    egui::Grid::new("light_levels").num_columns(3).show(ui, |ui| {
        ui.label("Intensity");
        ui.add(egui::Slider::new(&mut e.intensity, 0.0..=1.0));
        ui.end_row();
        opt_slider_row(
            ui,
            "Darkness",
            &mut e.darkness,
            global.darkness.at(outdoor),
            0.0..=1.0,
            "How black the dark is beyond the lamps: outdoors at the darkest of night (half at dusk and dawn), indoors at every hour. \
             1 = pitch black but for the lamps. Unset: Every scene's",
        );
        opt_slider_row(ui, "Strength", &mut e.strength, SCENE_STRENGTH, 0.0..=1.0, "");
        opt_slider_row(ui, "Ground bounce", &mut e.bounce_strength, SCENE_BOUNCE_STRENGTH, 0.0..=1.0, "");
        opt_slider_row(ui, "Exposure", &mut e.exposure, 1.0, 0.2..=2.0, "");
        opt_slider_row(ui, "Haze", &mut e.haze, 0.0, 0.0..=0.5, "");
    });

    // the side the painting is lit from: puppets lit by the scene take it
    super::group(ui, "Sun");
    let mut sun = e.sun.unwrap_or_default();
    let (mut moved, mut reset) = (false, false);
    egui::Grid::new("light_sun").num_columns(3).show(ui, |ui| {
        ui.label("From");
        moved |= ui
            .add(egui::Slider::new(&mut sun.azimuth, -180.0..=180.0).step_by(1.0).suffix(" deg"))
            .on_hover_text("Where the painting's light comes from, round the scene: 0 = from behind you (lit from the front), -90 = the left, 90 = the right, 180 = from behind the scene (backlit). Puppets lit by the scene (Preview) take it")
            .changed();
        if e.sun.is_none() {
            ui.weak("default");
        } else {
            reset = ui.small_button("x").on_hover_text("Back to the default: from the front left, 44 deg up").clicked();
        }
        ui.end_row();
        ui.label("Height");
        moved |= ui
            .add(egui::Slider::new(&mut sun.elevation, 0.0..=90.0).step_by(1.0).suffix(" deg"))
            .on_hover_text("How high the sun is: 0 = at the horizon, 90 = overhead")
            .changed();
        ui.end_row();
    });
    if reset {
        e.sun = None;
    } else if moved {
        e.sun = Some(sun);
    }

    super::group(ui, "Sky");
    egui::Grid::new("light_sky").num_columns(3).show(ui, |ui| {
        ui.label("Aurora").on_hover_text(
            "Curtains of light over the scene by night, behind the cloud: the Hinterwilds' sky. Drawn by the GPU lighting pass (v2), outdoors only",
        );
        let mut aurora = e.aurora.unwrap_or(0.0);
        if ui.add(egui::Slider::new(&mut aurora, 0.0..=1.0)).changed() {
            e.aurora = (aurora > 0.0).then_some(aurora);
        }
        if e.aurora.is_none() {
            ui.weak("none");
        } else if ui.small_button("x").on_hover_text("No aurora").clicked() {
            e.aurora = None;
        }
        ui.end_row();
    });

    super::group(ui, "Time of day");
    phase_table(ui, e, global);
}

/// The scene's own dawn, day, dusk and night: a column each, ticked "own"
/// to lay its key, ambient and intensity over Every scene's (shown greyed
/// until then).
fn phase_table(ui: &mut egui::Ui, e: &mut creature_field::lighting::SceneLight, global: &creature_field::lighting::GlobalLighting) {
    let defaults: Vec<creature_field::lighting::LightFields> = PHASES.iter().map(|(p, _)| global.phase.get(*p).cloned().unwrap_or_default()).collect();
    let mut slots = [&mut e.dawn, &mut e.day, &mut e.dusk, &mut e.night];
    egui::Grid::new("light_phases").num_columns(5).show(ui, |ui| {
        ui.label("");
        for (_, name) in PHASES {
            ui.weak(name);
        }
        ui.end_row();
        ui.label("Own").on_hover_text("This scene's own key, ambient and intensity for that part of the day; unticked, Every scene's");
        for slot in slots.iter_mut() {
            let mut own = slot.is_some();
            if ui.checkbox(&mut own, "").changed() {
                **slot = own.then(Default::default);
            }
        }
        ui.end_row();
        for (name, key) in [("Key", true), ("Ambient", false)] {
            ui.label(name);
            for (slot, d) in slots.iter_mut().zip(&defaults) {
                let fallback = if key { d.key.clone() } else { d.ambient.clone() };
                match (**slot).as_mut() {
                    Some(f) => {
                        let value = if key { &mut f.key } else { &mut f.ambient };
                        let mut rgb = value.clone().or(fallback).as_deref().and_then(parse_hex_rgb).unwrap_or(NEUTRAL_SLATE);
                        if ui.color_edit_button_srgb(&mut rgb).changed() {
                            *value = Some(hex_rgb(rgb));
                        }
                    }
                    None => {
                        let mut rgb = fallback.as_deref().and_then(parse_hex_rgb).unwrap_or(NEUTRAL_SLATE);
                        ui.add_enabled_ui(false, |ui| ui.color_edit_button_srgb(&mut rgb));
                    }
                }
            }
            ui.end_row();
        }
        ui.label("Intensity");
        for (slot, d) in slots.iter_mut().zip(&defaults) {
            let fallback = d.intensity.unwrap_or(if d.key.is_some() { 1.0 } else { 0.0 });
            match (**slot).as_mut() {
                Some(f) => {
                    let mut v = f.intensity.unwrap_or(fallback);
                    if ui.add(egui::DragValue::new(&mut v).range(0.0..=1.0).speed(0.01).fixed_decimals(2)).changed() {
                        f.intensity = Some(v);
                    }
                }
                None => {
                    let mut v = fallback;
                    ui.add_enabled(false, egui::DragValue::new(&mut v).fixed_decimals(2));
                }
            }
        }
        ui.end_row();
    });
}

/// The Facing picker's ways: none, then the compass.
const FACINGS: [(&str, Option<f32>); 9] = [
    ("None", None),
    ("N", Some(0.0)),
    ("NE", Some(45.0)),
    ("E", Some(90.0)),
    ("SE", Some(135.0)),
    ("S", Some(180.0)),
    ("SW", Some(225.0)),
    ("W", Some(270.0)),
    ("NW", Some(315.0)),
];

/// The moment the stage is seen in, on Scenes and Preview alike: the hour
/// and the player's Lighting strength, the sky (the game's moons or a Moon
/// slider, facing, cloud) and the weather. None of it is saved with the
/// scene: the game sets it from the clock and the room.
fn time_ui(stage: &mut StageState, ui: &mut egui::Ui, game_hour: f32) {
    use creature_field_egui::field_weather::WeatherKind;

    super::group(ui, "Clock");
    if stage.scene().light.is_none() {
        ui.weak("This scene is unlit: the hour and the sky show once Light this scene is ticked (Scenes, Light).");
    }
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
    egui::Grid::new("time_clock").num_columns(2).show(ui, |ui| {
        let mut hour = game_hour;
        let h = hour as u32;
        ui.label(format!("{:02}:{:02}", h, ((hour - h as f32) * 60.0) as u32));
        if ui.add(egui::Slider::new(&mut hour, 0.0..=24.0).step_by(0.05).show_value(false)).changed() {
            stage.light_hour = Some(hour);
        }
        ui.end_row();
        ui.label("Strength").on_hover_text("The player's Lighting strength: scales every light, scene and time of day alike");
        ui.add(egui::Slider::new(&mut stage.light_strength, 0.0..=1.0));
        ui.end_row();
    });

    super::group(ui, "Sky");
    ui.checkbox(&mut stage.game_sky, "The game's moons").on_hover_text(
        "Liabo and Lornon as the game has them at the hour shown, today: the game's own sky math (Saga's almanac). Liabo's phase comes round every 7.449 days, Lornon's every 35.3; each is highest at 01:00 when full and runs later as it wanes. By day Liabo shows pale gold unless new; outdoors only",
    );
    if stage.game_sky {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
        ui.weak(stage.moons_line(now, game_hour));
        ui.checkbox(&mut stage.lornons_eve, "Lornon's Eve").on_hover_text("Show them on Eorgaen 31: Lornon crimson and glowing");
    }
    egui::Grid::new("time_sky").num_columns(2).show(ui, |ui| {
        if !stage.game_sky {
            ui.label("Moon").on_hover_text(
                "How full the moon is: 0 = new, 1 = full. It rises at 18:00 on the left, is highest at midnight and sets at 06:00 on the right; outdoors only",
            );
            ui.add(egui::Slider::new(&mut stage.light_moon, 0.0..=1.0));
            ui.end_row();
        }
        ui.label("Facing").on_hover_text(
            "The way the view faces -- in the game, the way you last walked: the moons rise in the east, are highest in the south and set in the west, and are drawn in the sky when they are ahead of you. None: they keep to your side of the scene",
        );
        let label = |f: Option<f32>| FACINGS.iter().find(|(_, b)| *b == f).map_or("?", |(n, _)| *n);
        egui::ComboBox::from_id_salt("light_facing").selected_text(label(stage.facing)).show_ui(ui, |ui| {
            for (name, bearing) in FACINGS {
                ui.selectable_value(&mut stage.facing, bearing, name);
            }
        });
        ui.end_row();
        ui.label("Clouds").on_hover_text(
            "How much cloud covers the sky, 0 clear .. 1 overcast: on the GPU it is drawn on the sky and its shadows drift over the moonlit ground; rain, snow and fog bring their own",
        );
        ui.add(egui::Slider::new(&mut stage.light_clouds, 0.0..=1.0));
        ui.end_row();
    });

    // a preview: the game sets it from the room
    super::group(ui, "Weather");
    egui::Grid::new("time_weather").num_columns(2).show(ui, |ui| {
        ui.label("Kind");
        let name = |k: usize| k.checked_sub(1).and_then(|i| WeatherKind::ALL.get(i)).map_or("Clear", |w| w.label());
        egui::ComboBox::from_id_salt("stage_weather").selected_text(name(stage.weather_kind)).show_ui(ui, |ui| {
            for k in 0..=WeatherKind::ALL.len() {
                ui.selectable_value(&mut stage.weather_kind, k, name(k));
            }
        });
        ui.end_row();
        if stage.weather_kind > 0 {
            ui.label("Amount");
            ui.add(egui::Slider::new(&mut stage.weather_intensity, 0.0..=1.0));
            ui.end_row();
            ui.label("Wind");
            ui.add(egui::Slider::new(&mut stage.weather_wind, -1.0..=1.0));
            ui.end_row();
        }
    });
}

/// One colour row of a three-column grid: the label, the swatch (the
/// backdrop's value until set), and "auto" or the reset.
fn light_colour_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut creature_field::lighting::LightColour,
    auto: Option<&str>,
    hint: &str,
) {
    use creature_field::lighting::LightColour;
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
            if ui.small_button("x").on_hover_text("Back to auto: read from the backdrop").clicked() {
                *value = LightColour::Auto;
            }
        }
    }
    ui.end_row();
}

/// One optional number as a row of a three-column grid: the label, a slider
/// showing the default until touched, and "default" or the reset.
fn opt_slider_row(ui: &mut egui::Ui, label: &str, value: &mut Option<f32>, default: f32, range: std::ops::RangeInclusive<f32>, hint: &str) {
    let label = ui.label(label);
    if !hint.is_empty() {
        label.on_hover_text(hint);
    }
    let mut v = value.unwrap_or(default);
    if ui.add(egui::Slider::new(&mut v, range)).changed() {
        *value = Some(v);
    }
    if value.is_none() {
        ui.weak("default");
    } else if ui.small_button("x").on_hover_text(format!("Back to the default, {default:.2}")).clicked() {
        *value = None;
    }
    ui.end_row();
}

/// `global/lighting.toml`: the darkness, moon and time of day every scene
/// takes unless it sets its own (Niffy's "Settings > Lighting"). Edits apply
/// live; Save writes the file. Returns true when anything changed.
fn global_light_ui(ui: &mut egui::Ui, global: &mut creature_field::lighting::GlobalLighting, stage: &mut StageState) -> bool {
    use creature_field::lighting::LightFields;
    let mut changed = false;
    ui.weak("What every scene takes unless it sets its own: global/lighting.toml. Edits show at once; Save keeps them.");

    super::group(ui, "Darkness");
    egui::Grid::new("every_darkness").num_columns(2).show(ui, |ui| {
        let d = &mut global.darkness;
        ui.label("Outdoors").on_hover_text("At the darkest of night; half of it at dusk and dawn");
        changed |= ui.add(egui::Slider::new(&mut d.outdoor, 0.0..=1.0)).changed();
        ui.end_row();
        ui.label("Indoors").on_hover_text("At every hour: a room needs a lamp");
        changed |= ui.add(egui::Slider::new(&mut d.indoor, 0.0..=1.0)).changed();
        ui.end_row();
    });

    super::group(ui, "Moon");
    egui::Grid::new("every_moon").num_columns(2).show(ui, |ui| {
        let m = &mut global.moon;
        ui.label("Colour");
        let mut rgb = parse_hex_rgb(&m.color).unwrap_or(NEUTRAL_SLATE);
        if ui.color_edit_button_srgb(&mut rgb).changed() {
            m.color = hex_rgb(rgb);
            changed = true;
        }
        ui.end_row();
        ui.label("Strength");
        changed |= ui.add(egui::Slider::new(&mut m.strength, 0.0..=2.0)).changed();
        ui.end_row();
        ui.label("Lift").on_hover_text("Without the GPU pass: how much of the darkness a full, clear, high moon takes away");
        changed |= ui.add(egui::Slider::new(&mut m.lift, 0.0..=1.0)).changed();
        ui.end_row();
    });

    // Rows are the settings, columns the parts of the day (day is "no
    // change" unless the file gives one); a greyed number is the default,
    // right-click puts a set one back.
    super::group(ui, "Time of day");
    type Field = fn(&mut LightFields) -> &mut Option<f32>;
    let numbers: [(&str, Field, f32, f32, f32, &str); 6] = [
        ("Intensity", |f| &mut f.intensity, 1.0, 0.0, 1.0, "How much of that part of the day's light shows"),
        ("Exposure", |f| &mut f.exposure, 1.0, 0.1, 1.5, ""),
        ("Strength", |f| &mut f.strength, 0.85, 0.0, 1.0, ""),
        ("Haze", |f| &mut f.haze, 0.0, 0.0, 0.5, ""),
        ("Saturation", |f| &mut f.chroma, 1.0, 0.0, 1.2, ""),
        ("Night vision", |f| &mut f.night, 0.0, 0.0, 1.0, ""),
    ];
    let phases = &mut global.phase;
    let mut slots = [("Dawn", &mut phases.dawn), ("Dusk", &mut phases.dusk), ("Night", &mut phases.night)];
    egui::Grid::new("every_phases").num_columns(4).show(ui, |ui| {
        ui.label("");
        for (name, _) in &slots {
            ui.weak(*name);
        }
        ui.end_row();
        for (name, key) in [("Key", true), ("Ambient", false)] {
            ui.label(name);
            for (_, slot) in slots.iter_mut() {
                match slot.as_mut() {
                    Some(f) => {
                        let value = if key { &mut f.key } else { &mut f.ambient };
                        let mut rgb = value.as_deref().and_then(parse_hex_rgb).unwrap_or(NEUTRAL_SLATE);
                        if ui.color_edit_button_srgb(&mut rgb).changed() {
                            *value = Some(hex_rgb(rgb));
                            changed = true;
                        }
                    }
                    None => {
                        ui.weak("-");
                    }
                }
            }
            ui.end_row();
        }
        for (name, field, default, lo, hi, hint) in numbers {
            let label = ui.label(name);
            if !hint.is_empty() {
                label.on_hover_text(hint);
            }
            for (_, slot) in slots.iter_mut() {
                match slot.as_mut() {
                    Some(f) => changed |= super::opt_drag(ui, field(f), default, lo..=hi, 0.01),
                    None => {
                        ui.weak("-");
                    }
                }
            }
            ui.end_row();
        }
    });

    ui.horizontal(|ui| {
        if ui.button("Save").on_hover_text("Write global/lighting.toml").clicked() {
            let msg = match global.save() {
                Ok(()) => "Saved global/lighting.toml".to_string(),
                Err(e) => format!("Could not save global/lighting.toml: {e}"),
            };
            stage.push_status(msg);
        }
        if ui.button("Restore shipped defaults").clicked() {
            *global = creature_field::lighting::GlobalLighting::default();
            changed = true;
        }
    });
    changed
}

/// The Scene section: which scene is edited, its backdrop and colour,
/// persisted whole-file to `global/scenes/<name>.toml`.
fn scene_ui(stage: &mut StageState, ui: &mut egui::Ui) {
    // The scene FILENAME is the binding: leading room uid binds that room,
    // otherwise the stem matches room title -> location -> "default".
    // There is no separate bindings store to manage.
    // (its name and "* unsaved" head the panel: outline_ui)
    ui.horizontal_wrapped(|ui| {
        if !stage.scene_name.trim().is_empty() && ui.button("Save").clicked() {
            stage.save_current_scene();
        }
        if ui.button("Clear stage scene").clicked() {
            stage.request_scene(SceneAction::Clear);
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
            stage.request_scene(SceneAction::Load(name));
        }
    }

    let backgrounds = creature_field::pool::list_category("scenes");
    super::group(ui, "Backdrop");
    egui::Grid::new("scene_backdrop").num_columns(2).show(ui, |ui| {
        ui.label("Image");
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
        ui.end_row();
        ui.label("Colour");
        // The picker needs a concrete color; an unset scene edits a neutral
        // slate and only stores once touched (None keeps the default panel
        // fill).
        let mut rgb = stage.scene().background_color.as_deref().and_then(parse_hex_rgb).unwrap_or(NEUTRAL_SLATE);
        ui.horizontal(|ui| {
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                stage.scene_mut().background_color = Some(hex_rgb(rgb));
            }
            if stage.scene().background_color.is_some() && ui.small_button("x").on_hover_text("Clear colour").clicked() {
                stage.scene_mut().background_color = None;
            }
        });
        ui.end_row();
    });
}

/// The scene's scenery props. True when "Calibrate scenery..." was pressed.
fn props_ui(stage: &mut StageState, ui: &mut egui::Ui) -> bool {
    // VellumFE's Stage opened its scenery calibrator window from here; in
    // this app the calibrator is the Scenery tab, so the button goes there.
    // A save over there comes back through `StageTool::art_changed`.
    let calibrate_scenery = ui
        .button("Calibrate scenery...")
        .on_hover_text("Feet anchor, world size, exclusion edges -- per image. Opens the Scenery tab; the stage is kept.")
        .clicked();
    let pool = creature_field::pool::list_category("scenery");
    if pool.is_empty() && creature_field::pool::list_category("scenes").is_empty() {
        ui.weak(
            "Drop PNGs into global/images/scenes (backgrounds, author at 880x470) \
             or global/images/scenery (props)",
        );
    }
    super::group(ui, "Add");
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

    if stage.scene().props.is_empty() {
        return calibrate_scenery;
    }
    // What stands on the stage: one row each, the selected one's place below.
    super::group(ui, "Placed");
    let props = stage.scene().props.clone();
    let mut edited = props.clone();
    let (mut remove, mut select) = (None, None);
    egui::Grid::new("prop_table").num_columns(4).striped(true).show(ui, |ui| {
        ui.weak("prop");
        ui.weak("scale");
        ui.weak("shadow").on_hover_text("A contact shadow under it: off by default -- most scenery paints its own grounding");
        ui.label("");
        ui.end_row();
        for (k, prop) in edited.iter_mut().enumerate() {
            let name = prop.image.rsplit('/').next().unwrap_or(&prop.image).to_string();
            let selected = stage.selected_prop == Some(k);
            if ui.selectable_label(selected, name).on_hover_text("Select it to move it: drag it on the stage; click again to deselect").clicked() {
                select = Some(k);
            }
            ui.add(egui::DragValue::new(&mut prop.scale).range(0.1..=5.0).speed(0.01).fixed_decimals(2));
            ui.checkbox(&mut prop.shadow, "");
            if ui.small_button("x").on_hover_text("Remove").clicked() {
                remove = Some(k);
            }
            ui.end_row();
        }
    });
    if edited != props {
        stage.scene_mut().props = edited;
    }
    if let Some(k) = select {
        let was = stage.selected_prop == Some(k);
        stage.selected_prop = (!was).then_some(k);
        if !was {
            stage.selected_light = None;
        }
    }
    if let Some(k) = remove {
        stage.remove_prop(k);
    }
    if let Some(k) = stage.selected_prop {
        if let Some(prop) = stage.scene().props.get(k).cloned() {
            // Edit copies; only a real change re-points the scene Arc.
            let mut edited = prop.clone();
            ui.horizontal(|ui| {
                ui.label("Place: x");
                ui.add(egui::DragValue::new(&mut edited.x).speed(2.0));
                ui.label("z");
                ui.add(egui::DragValue::new(&mut edited.z).speed(0.02));
            });
            ui.weak("Drag on the stage to move the selected prop");
            if edited != prop {
                stage.scene_mut().props[k] = edited;
            }
            stage.clamp_prop(k);
        }
    }
    calibrate_scenery
}

/// Point lights on the floor: campfires, torches, glows (`[[lights]]`).
fn lights_ui(stage: &mut StageState, ui: &mut egui::Ui) {
    use creature_field::lighting::LampKind;
    ui.horizontal_wrapped(|ui| {
        for (label, kind) in [
            ("+ Campfire", LampKind::Campfire),
            ("+ Torch", LampKind::Torch),
            ("+ Glow", LampKind::Glow),
            ("+ Lantern", LampKind::Lantern),
            ("+ Wisp", LampKind::Wisp),
        ] {
            if ui.button(label).clicked() {
                stage.add_light(kind);
            }
        }
    });
    if stage.scene().lights.is_empty() {
        ui.weak("Firelight shows best at night: tick Light this scene and pick Night.");
        return;
    }
    // Every lamp one row: what it is, its colour, whether it is a main light
    // (shadows, most of its cost on the GPU) or a fill.
    let lights = stage.scene().lights.clone();
    let mut edited = lights.clone();
    let (mut remove, mut select) = (None, None);
    let shadow_hint = "A main light (a fire, a torch) casts shadows. Unticked, it is a fill or an accent: it lights without them, \
                       far cheaper on the GPU (v2), which shadows only the first few lights that ask";
    egui::Grid::new("lamp_table").num_columns(4).striped(true).show(ui, |ui| {
        ui.weak("lamp");
        ui.weak("colour");
        ui.weak("shadows").on_hover_text(shadow_hint);
        ui.label("");
        ui.end_row();
        for (k, lamp) in edited.iter_mut().enumerate() {
            let selected = stage.selected_light == Some(k);
            if ui
                .selectable_label(selected, format!("{} {}", kind_name(lamp.kind), k + 1))
                .on_hover_text("Select it: its settings below, and drag it on the stage; click again to deselect")
                .clicked()
            {
                select = Some(k);
            }
            let mut rgb = parse_hex_rgb(&lamp.color).unwrap_or([255, 160, 74]);
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                lamp.color = hex_rgb(rgb);
            }
            let mut shadows = lamp.casts_shadows();
            if ui.checkbox(&mut shadows, "").on_hover_text(shadow_hint).changed() {
                lamp.set_shadows(shadows);
            }
            if ui.small_button("x").on_hover_text("Remove").clicked() {
                remove = Some(k);
            }
            ui.end_row();
        }
    });
    let shadowed = edited.iter().filter(|l| l.casts_shadows()).count();
    let cap = creature_field_egui::field_gpu::MAX_SHADOW_LIGHTS;
    if shadowed > cap {
        ui.colored_label(ui.visuals().warn_fg_color, format!("{shadowed} lamps ask for shadows; the GPU shadows only the first {cap}."));
    }
    if edited != lights {
        stage.scene_mut().lights = edited;
    }
    if let Some(k) = select {
        let was = stage.selected_light == Some(k);
        stage.selected_light = (!was).then_some(k);
        if !was {
            stage.selected_prop = None;
        }
    }
    if let Some(k) = remove {
        stage.remove_light(k);
    }
    let Some(k) = stage.selected_light else { return };
    let Some(light) = stage.scene().lights.get(k).cloned() else { return };
    let mut edited = light.clone();
    super::group(ui, &format!("{} {}", kind_name(edited.kind), k + 1));
    egui::Grid::new("lamp_detail").num_columns(2).show(ui, |ui| {
        ui.label("Kind");
        egui::ComboBox::from_id_salt("light_kind").selected_text(kind_name(edited.kind)).show_ui(ui, |ui| {
            ui.selectable_value(&mut edited.kind, LampKind::Campfire, "Campfire");
            ui.selectable_value(&mut edited.kind, LampKind::Torch, "Torch");
            ui.selectable_value(&mut edited.kind, LampKind::Glow, "Glow (no flame)");
            ui.selectable_value(&mut edited.kind, LampKind::Lantern, "Lantern (shines one way)");
            ui.selectable_value(&mut edited.kind, LampKind::Wisp, "Wisp (floats and drifts)");
        });
        ui.end_row();
        if matches!(edited.kind, LampKind::Wisp | LampKind::Glow) {
            // Niffy's presets: on the GPU (v2) a preset's light swells and fades
            // toward its own brighter and dimmer shades as it pulses.
            ui.label("Preset");
            ui.horizontal_wrapped(|ui| {
                for (name, [_, base, _]) in creature_field_egui::field_gpu::MOTE_PRESETS {
                    if ui.small_button(name).on_hover_text("Its colour; on the GPU (v2) it pulses toward the preset's own shades").clicked() {
                        edited.color = base.to_string();
                    }
                }
            });
            ui.end_row();
        }
        ui.label("Place");
        ui.horizontal(|ui| {
            ui.label("x");
            ui.add(egui::DragValue::new(&mut edited.x).speed(2.0));
            ui.label("z");
            ui.add(egui::DragValue::new(&mut edited.z).speed(0.02));
        });
        ui.end_row();
        ui.label("Reach");
        ui.add(egui::Slider::new(&mut edited.radius, 0.3..=6.0));
        ui.end_row();
        ui.label("Brightness");
        ui.add(egui::Slider::new(&mut edited.intensity, 0.0..=1.0));
        ui.end_row();
        ui.label("Flicker");
        ui.add(egui::Slider::new(&mut edited.flicker, 0.0..=1.0));
        ui.end_row();
        ui.label("Beam").on_hover_text("Shine one way (a lantern, a lit window) instead of all round");
        let mut beam = !edited.all_round();
        if ui.checkbox(&mut beam, "").changed() {
            edited.spread = if beam { 90.0 } else { 360.0 };
        }
        ui.end_row();
        if beam {
            ui.label("Width");
            ui.add(egui::Slider::new(&mut edited.spread, 10.0..=350.0).suffix(" deg"));
            ui.end_row();
            ui.label("Aim").on_hover_text("Which way across the floor: 0 = right, 90 = away from you, 180 = left, 270 = toward you");
            ui.add(egui::Slider::new(&mut edited.aim, 0.0..=359.0).suffix(" deg"));
            ui.end_row();
        }
        ui.label("Lights");
        ui.horizontal_wrapped(|ui| {
            use creature_field::lighting::LightTarget;
            for (target, name, tip) in [
                (LightTarget::Backdrop, "Backdrop", "The painted ground and walls"),
                (LightTarget::Scenery, "Scenery", "The scenery props placed on the stage"),
                (LightTarget::Creatures, "Creatures", "The creatures"),
                (LightTarget::Air, "Air", "Its glow in mist, rain, snow and blowing sand (on the GPU, v2)"),
            ] {
                let mut on = edited.lights(target);
                if ui.checkbox(&mut on, name).on_hover_text(tip).changed() {
                    edited.set_lights(target, on);
                }
            }
        });
        ui.end_row();
    });
    ui.weak("Drag on the stage to move the selected light");
    if edited != light {
        stage.scene_mut().lights[k] = edited;
    }
}

fn kind_name(kind: creature_field::lighting::LampKind) -> &'static str {
    crate::stage::scene_notes::lamp_label(kind)
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
                    stage.request_scene(SceneAction::Load(stem));
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

/// How the stage draws its puppets (Preview): what they hold, whether they
/// breathe, their shading, and the splat look test.
fn display_ui(stage: &mut StageState, ui: &mut egui::Ui) {
    ui.checkbox(&mut stage.show_props, "Held weapons").on_hover_text(
        "A puppet holds the first weapon its bestiary entry attacks with (a forest troll its cudgel), \
         shaped by props/weapons.toml",
    );
    ui.checkbox(&mut stage.gpu_skins, "GPU skins").on_hover_text(
        "Draw the puppets' skins on the GPU, relit there by the scene and the time of day -- with Light on the GPU (v2) \
         and a lit scene; the CPU skin otherwise",
    );
    ui.checkbox(&mut stage.show_breath, "Breathing").on_hover_text(
        "A living puppet breathes: the slow cycle in its body type's poses file ([idle] in \
         poses/<body_type>.toml), each creature at its own point in it",
    );
    ui.checkbox(&mut stage.sun_lit_skins, "Lit by the scene").on_hover_text(
        "Shade each skin by the scene's sun and sky -- the paint flat, a warm sun from the front \
         left tinted by the scene's key light, a cool fill tinted by its ambient, a rim along the \
         edges -- instead of the skin's own fixed light; the time of day goes on top. Needs \
         Light this scene",
    );
    ui.checkbox(&mut stage.show_splats, "Splats").on_hover_text(
        "Draw every puppet as a Gaussian splat cloud instead of its smooth skin: one soft \
         splat per ring sample, sorted and blended (no contour, wounds or effects). The look \
         test for a splat renderer; drawn on the CPU like the skin",
    );
    if stage.show_splats {
        let p = &mut stage.splat_params;
        ui.indent("splat_params", |ui| {
            ui.add(egui::Slider::new(&mut p.spacing, 0.01..=0.12).logarithmic(true).text("spacing ft"))
                .on_hover_text("Sample step along each ring and between rings: finer = more, smaller splats (a 6 ft figure is about 12k at 0.03)");
            ui.add(egui::Slider::new(&mut p.overlap, 0.3..=1.5).text("overlap"))
                .on_hover_text("How far each splat reaches past its step: 0.5 = neighbours just touch; more = softer, denser");
            ui.add(egui::Slider::new(&mut p.alpha, 0.1..=1.0).text("opacity"))
                .on_hover_text("Each splat's peak opacity");
            if ui.small_button("Defaults").clicked() {
                *p = gs_puppet::splat::SplatParams::default();
            }
        });
    }
}

// ------------------------------------------------------------------ notes

/// The weather now, as a note records it ("" = clear).
fn weather_label(stage: &StageState) -> String {
    use creature_field_egui::field_weather::WeatherKind;
    stage.weather_kind.checked_sub(1).and_then(|k| WeatherKind::ALL.get(k)).map_or(String::new(), |w| w.label().to_owned())
}

/// The Notes section (Scenes and Preview alike): notes pinned to the scene
/// (`stage::scene_notes`), open while one is being written.
fn notes_section(stage: &mut StageState, ui: &mut egui::Ui) {
    let name = stage.scene_name.clone();
    stage.notes.sync(&name);
    let open = stage.notes.notes().map_or(0, |n| n.open().count());
    let title = super::section_title(ui, if open == 0 { "Notes".to_owned() } else { format!("Notes ({open} open)") });
    let writing = stage.notes.pending.is_some();
    super::section_with(ui, "stage_notes", title, false, writing, |ui| notes_ui(stage, ui));
}

/// Pin notes (a click on the stage pins one, about the lamp or prop
/// there, at the hour and weather on view), the note being written, and
/// the list -- Show (that hour and weather, the lamp or prop selected, its
/// pin ringed), Reply, Done, Delete. Every change is written at once.
fn notes_ui(stage: &mut StageState, ui: &mut egui::Ui) {
    use crate::stage::scene_notes::SceneNote;
    ui.horizontal_wrapped(|ui| {
        let can = stage.notes.notes().is_ok();
        ui.add_enabled(can, egui::Button::selectable(stage.notes.pinning, "Pin notes"))
            .on_hover_text(
                "Click the stage to pin a note there (\"the fire lights nothing behind the bench\", \"too dark by the door\"), \
                 about the lamp or prop under the click, at the hour and weather on view: for whoever works on the scene next",
            )
            .clicked()
            .then(|| stage.notes.pinning = !stage.notes.pinning);
        ui.checkbox(&mut stage.notes.hide_done, "hide done");
    });
    let notes = match stage.notes.notes() {
        Ok(n) => n.notes.clone(),
        Err(why) => {
            ui.weak(why.to_owned());
            return;
        }
    };
    ui.weak(format!("scenes/notes/{}.toml", stage.scene_name.trim()));
    let (mut pin_it, mut cancel) = (false, false);
    if let Some((pin, text)) = stage.notes.pending.as_mut() {
        ui.horizontal_wrapped(|ui| {
            ui.label(if pin.about.is_empty() { "New note:".to_owned() } else { format!("New note on {}:", pin.about) });
            let edit = ui.add(egui::TextEdit::singleline(text).desired_width(220.0).hint_text("what is wrong here"));
            edit.request_focus();
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            pin_it = (ui.button("Pin").clicked() || enter) && !text.trim().is_empty();
            cancel = ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
        });
    }
    if pin_it {
        if let Err(e) = stage.notes.pin_pending(&crate::notes::author(), &crate::notes::today()) {
            stage.push_status(format!("NOTE NOT SAVED: {e}"));
        }
    } else if cancel {
        stage.notes.pending = None;
    }
    let mut acts: Vec<(SceneNote, super::NoteAct)> = Vec::new();
    for note in &notes {
        if stage.notes.hide_done && note.done {
            continue;
        }
        let mut place = note.about.clone();
        if let Some(h) = note.hour {
            let (hh, mm) = (h as u32, ((h - h.floor()) * 60.0) as u32);
            place.push_str(&format!("{}{hh:02}:{mm:02}", if place.is_empty() { "" } else { ", " }));
        }
        if !note.weather.is_empty() {
            place.push_str(&format!(", {}", note.weather));
        }
        let view = super::NoteView {
            id: note.id,
            text: &note.text,
            by: &note.by,
            made: &note.made,
            place,
            done: note.done,
            replies: &note.replies,
            focused: stage.notes.focus == Some(note.id),
        };
        if let Some(act) = super::note_ui(ui, view, &mut stage.notes.draft) {
            acts.push((note.clone(), act));
        }
    }
    for (note, act) in acts {
        let id = note.id;
        let written = match act {
            super::NoteAct::Show => {
                show_scene_note(stage, &note);
                continue;
            }
            super::NoteAct::Done(done) => stage.notes.edit(|n| {
                if let Some(x) = n.notes.iter_mut().find(|x| x.id == id) {
                    x.done = done;
                }
            }),
            super::NoteAct::Delete => stage.notes.edit(|n| {
                n.remove(id);
            }),
            super::NoteAct::Reply(text) => stage.notes.edit(|n| {
                n.reply(id, crate::notes::Reply::new(&crate::notes::author(), &text, &crate::notes::today()));
            }),
        };
        if let Err(e) = written {
            stage.push_status(format!("NOTES NOT SAVED: {e}"));
        }
    }
}

/// Put the stage back to where a note was seen: its hour and weather, its
/// lamp or prop selected (when the list still has it), its pin ringed.
fn show_scene_note(stage: &mut StageState, note: &crate::stage::scene_notes::SceneNote) {
    use creature_field_egui::field_weather::WeatherKind;
    stage.notes.focus = Some(note.id);
    if note.hour.is_some() {
        stage.light_hour = note.hour;
    }
    stage.weather_kind = WeatherKind::ALL.iter().position(|w| w.label() == note.weather).map_or(0, |k| k + 1);
    if let Some(k) = note.lamp.filter(|&k| k < stage.scene().lights.len()) {
        stage.selected_light = Some(k);
        stage.selected_prop = None;
    } else if let Some(k) = note.prop.filter(|&k| k < stage.scene().props.len()) {
        stage.selected_prop = Some(k);
        stage.selected_light = None;
    }
}

/// The scene's open notes as numbered pins on the stage (the one being
/// written too, the one last shown ringed); a pin shows its note on hover
/// and a click on it shows the note.
fn note_pins(stage: &mut StageState, ui: &mut egui::Ui, rect: egui::Rect) {
    let to_screen = stage_to_screen(rect);
    let at = |p: [f32; 2]| to_screen(p[0], p[1]);
    let mut pins: Vec<(u32, [f32; 2], String)> =
        stage.notes.notes().map(|n| n.open().map(|x| (x.id, x.at, x.text.clone())).collect()).unwrap_or_default();
    if let Some((pin, text)) = &stage.notes.pending {
        pins.push((0, pin.at, if text.is_empty() { "(new note)".to_owned() } else { text.clone() }));
    }
    let painter = ui.painter_at(rect);
    let fill = egui::Color32::from_rgb(240, 170, 60);
    let mut show = None;
    for (id, p, text) in pins {
        let c = at(p);
        painter.circle(c, 9.0, fill, egui::Stroke::new(1.5, egui::Color32::from_black_alpha(200)));
        let label = if id == 0 { "+".to_owned() } else { id.to_string() };
        painter.text(c, egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(11.0), egui::Color32::BLACK);
        if id != 0 && stage.notes.focus == Some(id) {
            painter.circle_stroke(c, 14.0, egui::Stroke::new(2.0, fill));
        }
        if id != 0 {
            let r = ui.interact(egui::Rect::from_center_size(c, egui::vec2(20.0, 20.0)), ui.id().with(("note_pin", id)), egui::Sense::click());
            if r.on_hover_text(format!("#{id} {text}")).clicked() {
                show = Some(id);
            }
        }
    }
    if let Some(note) = show.and_then(|id| stage.notes.notes().ok().and_then(|n| n.get(id)).cloned()) {
        show_scene_note(stage, &note);
    }
}

// ------------------------------------------------------------------- cast

/// Every published puppet, grouped bestiary family -> form -> texture, a
/// Spawn button on each form (its bare sheet) and each skin (the creature
/// whose skin it is). The Cast below is built from the art pool and sprite
/// art beats the puppet, so this is the direct way to look at one: it spawns
/// a real creature of that form forced to draw as the puppet.
fn puppets_ui(stage: &mut StageState, ui: &mut egui::Ui) {
    super::section(ui, "stage_puppets", "Puppets", |ui| {
        let mut spawn: Option<(String, Option<(String, String)>)> = None;
        egui::ScrollArea::vertical().id_salt("stage_puppets").max_height(240.0).show(ui, |ui| {
            for family in cast::puppet_tree() {
                let n = family.forms.len();
                egui::CollapsingHeader::new(format!("{} ({n})", family.family)).id_salt(("puppet_family", &family.family)).show(ui, |ui| {
                    for form in &family.forms {
                        ui.horizontal(|ui| {
                            if ui
                                .small_button("Spawn")
                                .on_hover_text("Spawn a creature of this form drawn as its puppet, in the form's own sheet (bare when it has none)")
                                .clicked()
                            {
                                spawn = Some((form.form.clone(), None));
                            }
                            ui.label(&form.form);
                            let body = if form.body_type.is_empty() { "?" } else { form.body_type.as_str() };
                            let sheet = if form.sheet { ", sheet" } else { "" };
                            if form.pool {
                                ui.weak(format!("{body}, pool{sheet}"))
                                    .on_hover_text("From the pool's global/puppets/ (a rig save puts it there); it replaces any built-in one");
                            } else {
                                ui.weak(format!("{body}, built in{sheet}"));
                            }
                        });
                        // Its skins, indented (the sheet is what the form's
                        // own Spawn wears).
                        for texture in form.textures.iter().filter(|t| t.creature.is_some()) {
                            ui.horizontal(|ui| {
                                ui.add_space(24.0);
                                if ui.small_button("Spawn").on_hover_text("Spawn this creature drawn as the puppet, wearing its own skin").clicked() {
                                    spawn = Some((form.form.clone(), texture.creature.clone()));
                                }
                                ui.label(&texture.label);
                                ui.weak("skin");
                            });
                        }
                    }
                });
            }
        });
        ui.weak("Save a rig in the Rigs tab to build its puppet; a creature's skin (puppets/skins/<form>/<creature>.png) lists under its form.");
        if let Some((form, creature)) = spawn {
            stage.spawn_puppet_wearing(&form, creature.as_ref().map(|(n, s)| (n.as_str(), s.as_str())));
        }
    });
}

/// The Preview panel's creatures: the published puppets, the cast to spawn
/// from, and who is on the stage.
fn cast_ui(stage: &mut StageState, ui: &mut egui::Ui) {
    puppets_ui(stage, ui);
    super::section(ui, "stage_cast", "Cast", |ui| cast_list_ui(stage, ui));
    super::section(ui, "stage_roster", "Roster", |ui| roster_ui(stage, ui));
}

/// The cast, from the art pool, to spawn from.
fn cast_list_ui(stage: &mut StageState, ui: &mut egui::Ui) {
    ui.weak("Built from the ART POOL, so most of these draw as their sprites (one with a skin of its own draws as its puppet). Use Puppets above, or a creature's \"Draw as puppet\", to see the 3D puppet.");
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut stage.filter).hint_text("filter").desired_width(140.0));
        if ui
            .button("Add generic")
            .on_hover_text(
                "Spawn an anonymous \"training dummy\". It has no sprite of its own, so it draws as the humanoid puppet.",
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
}

/// Who is on the stage, and the selected one's simulator.
fn roster_ui(stage: &mut StageState, ui: &mut egui::Ui) {
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
                ui.weak("[art] = draws as its sprite   [puppet] = draws as its 3D puppet");
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
        let mut deg = custom.unwrap_or(creature_field_egui::creature_puppet::FIELD_YAW_DEG);
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

fn field_ui(stage: &mut StageState, ui: &mut egui::Ui, art: &SharedCreatureArt, panel: StagePanel, game_hour: f32) {
    // Pinning a note takes the click; else on the Scenes tab a selected
    // prop owns the drag (as in VellumFE), and on the Preview tab drags
    // always move creatures.
    let pinning = stage.notes.pinning && stage.notes.notes().is_ok();
    let dragging_prop = !pinning && panel == StagePanel::Scenes && (stage.selected_prop.is_some() || stage.selected_light.is_some());
    ui.horizontal(|ui| {
        ui.checkbox(&mut stage.show_grid, "Grid");
        ui.checkbox(&mut stage.show_order, "Draw order");
        ui.separator();
        ui.weak(if pinning {
            "click: pin a note there   (Pin notes again to stop)"
        } else if dragging_prop {
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
    creature_field_egui::creature_puppet::set_forced_puppets(ui.ctx(), stage.forced_puppets().cloned());
    creature_field_egui::creature_puppet::set_puppet_facings(ui.ctx(), stage.facings().map(|(id, d)| (id.clone(), *d)));
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
    if pinning {
        let response = ui.interact(rect, ui.id().with("stage_note_pin"), egui::Sense::click());
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let (sx, sy) = stage_pos(pos);
                let hour = stage.scene().light.is_some().then_some(game_hour);
                let pin = stage.note_pin_at(sx, sy, hour, &weather_label(stage));
                stage.notes.pending = Some((pin, String::new()));
            }
        }
        note_pins(stage, ui, rect);
        return;
    }
    if dragging_prop {
        // Drag-to-place: while a prop is selected, dragging the field view
        // moves it on the ground plane (screen -> stage -> ground, through
        // the renderer's own mapping and the solver's inverse projection).
        let response = ui.interact(rect, ui.id().with("scene_prop_drag"), egui::Sense::drag());
        if response.dragged() {
            if let Some(pos) = response.interact_pointer_pos() {
                let (sx, sy) = stage_pos(pos);
                if stage.selected_light.is_some() {
                    stage.drag_selected_light_to(sx, sy);
                } else {
                    stage.drag_selected_prop_to(sx, sy);
                }
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
    if stage.show_walk && panel == StagePanel::Scenes {
        walk_overlay(stage, ui, rect);
    }
    note_pins(stage, ui, rect);
}
