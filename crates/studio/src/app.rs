//! The app shell: tool tabs across the top (with the two directory boxes),
//! the active tool in the middle, a status line at the bottom. Tools are
//! added by extending [`TOOLS`] and the `match` in [`StudioApp::ui`].
//!
//! Three tools live behind the seven tabs: the rig editor (Rigs), the stage
//! (Scenes + Preview) and the calibrators (Frames, Dolls, Creatures, Scenery).
//! (The 3D injury doll is the top bar's Doll window; the Dolls tab calibrates
//! Hydra's 2D doll pictures.) Each is built lazily and then kept for the life of the app, so
//! switching tabs never loses work.

use std::path::{Path, PathBuf};

use crate::calib::{CalibKind, HostEffects};
use crate::data_home::{self, ImportPlan, ImportReport};
use crate::data_root::{self, DataRoot};
use crate::pool_dir::{self, PoolDir};
use crate::quit::{CloseSequence, CloseStep, QuitGuard};
use crate::settings;
use crate::stage::StudioRoomContext;
use crate::tools::calibrators::CalibratorsTool;
use crate::tools::preview::{StagePanel, StageTool};
use crate::tools::rigs::RigTool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolId {
    Rigs,
    Scenes,
    Frames,
    Dolls,
    Creatures,
    Scenery,
    Preview,
}

struct ToolTab {
    id: ToolId,
    label: &'static str,
    hint: &'static str,
}

const TOOLS: [ToolTab; 7] = [
    ToolTab { id: ToolId::Rigs, label: "Rigs", hint: "Creature-doll rig editor (data root)" },
    ToolTab { id: ToolId::Scenes, label: "Scenes", hint: "Scene builder: backdrop, props, camera (pool)" },
    ToolTab { id: ToolId::Frames, label: "Frames", hint: "Frame templates for Hydra: slice a frame picture into edge, corner and ornament, preview it assembled, export the folder (pool)" },
    ToolTab { id: ToolId::Dolls, label: "Dolls", hint: "Injury-doll picture calibrator: where each part's wound dot sits, saved into the PNG (pool)" },
    ToolTab { id: ToolId::Creatures, label: "Creatures", hint: "Creature-art calibrator: feet, size, anchors (pool)" },
    ToolTab { id: ToolId::Scenery, label: "Scenery", hint: "Scenery and prop calibrator: feet, size, exclusion edges (pool)" },
    ToolTab { id: ToolId::Preview, label: "Preview", hint: "Preview stage: cast, roster, creature simulator (pool)" },
];

impl ToolId {
    /// `--tab <name>` / `GS_STUDIO_TAB`: a tool by its label, any case.
    pub fn from_name(name: &str) -> Result<ToolId, String> {
        TOOLS.iter().find(|t| t.label.eq_ignore_ascii_case(name.trim())).map(|t| t.id).ok_or_else(|| {
            let names: Vec<String> = TOOLS.iter().map(|t| t.label.to_ascii_lowercase()).collect();
            format!("no tool called '{name}' (tools: {})", names.join(", "))
        })
    }

    /// Which directory the tool works out of (the status line shows it).
    fn uses_pool(self) -> bool {
        self != ToolId::Rigs
    }
}

/// What `main` hands the app.
pub struct Startup {
    pub data: DataRoot,
    pub pool: PoolDir,
    /// Where Publish ships (`rigs/`, `puppets/`, `outfits/` under it: the forms pack).
    pub publish: PathBuf,
    /// The silhouette chart, if one is known (`settings.toml`).
    pub chart: Option<PathBuf>,
    /// A problem reading `settings.toml`, for the status line.
    pub settings_note: Option<String>,
    pub tab: ToolId,
    pub room: StudioRoomContext,
    /// Smoke runs: close the window by itself after this many seconds.
    pub exit_after_secs: Option<f32>,
}

pub struct StudioApp {
    data: DataRoot,
    /// The data-root text box (applied with "Open").
    data_edit: String,
    /// The art pool / scenes directory (creature_field's base dir).
    pool: PoolDir,
    /// The pool text box (applied with "Set").
    pool_edit: String,
    publish: PathBuf,
    /// The publish text box (applied with "Set").
    publish_edit: String,
    /// The silhouette chart box (applied with "Import").
    chart_edit: String,
    /// The last folder message (an import, a failed remember), shown in
    /// the Settings window.
    folders_note: Option<String>,
    /// The Settings window is open.
    settings_open: bool,
    /// The *Your name* box (notes and replies are signed with it).
    author_edit: String,
    /// Filesystem answers the Settings window and the banner need every
    /// frame (see `fs_cache`).
    importable: crate::fs_cache::Background<(), Option<PathBuf>>,
    pool_empty: crate::fs_cache::Background<PathBuf, bool>,
    active: ToolId,
    rigs: Result<RigTool, String>,
    /// Scenes + Preview share one stage. Built lazily on first entry (the
    /// cast scan reads the pool).
    stage: Option<StageTool>,
    /// The four calibrator tabs. Each calibrator opens lazily inside it.
    calibrators: CalibratorsTool,
    /// Room facts from the command line (new-scene prefill only).
    room: StudioRoomContext,
    started: std::time::Instant,
    exit_after_secs: Option<f32>,
    /// `GS_STUDIO_SHOT`: save a frame here, then close.
    shot: Option<Shot>,
    /// `GS_STUDIO_REPORT`: with the shot, write the scene report here.
    report: Option<PathBuf>,
    /// "Import from VellumFE...": the open confirmation, and the last
    /// report. See [`Import`].
    import: Import,
    /// A window close waiting on the unsaved-work questions.
    closing: CloseSequence,
    /// The injury doll window, over any tab.
    doll: crate::tools::doll::DollWindow,
}

/// A dev screenshot (`GS_STUDIO_SHOT`): frames drawn, and asked for yet.
struct Shot {
    path: PathBuf,
    frames: u32,
    requested: bool,
}

/// The VellumFE import: a read-only plan, shown and confirmed before a
/// single byte is copied. Nothing here ever WRITES to the VellumFE
/// directory -- it is the user's game client and is left exactly as found.
#[derive(Default)]
struct Import {
    /// The confirmation window is open with this plan (source, plan).
    asking: Option<(PathBuf, ImportPlan)>,
    /// What the last import did.
    report: Option<ImportReport>,
}

impl StudioApp {
    pub fn new(startup: Startup) -> StudioApp {
        let Startup { data, pool, publish, chart, settings_note, tab, room, exit_after_secs } = startup;
        let doll = crate::tools::doll::DollWindow::from_env().with_bodies(&data.path);
        let mut rigs = RigTool::open(&data.path, publish.clone(), Some(pool.path.clone()));
        if let (Ok(tool), Ok(spec)) = (&mut rigs, std::env::var("GS_STUDIO_POSE_EDIT")) {
            tool.open_pose_editor(&spec);
        }
        if let (Ok(tool), Ok(spec)) = (&mut rigs, std::env::var("GS_STUDIO_NOTE")) {
            tool.open_note(&spec);
        }
        let settings_open = rigs.is_err();
        StudioApp {
            data_edit: data.path.display().to_string(),
            rigs,
            data,
            pool_edit: pool.path.display().to_string(),
            pool,
            publish_edit: publish.display().to_string(),
            publish,
            chart_edit: chart.map(|c| c.display().to_string()).unwrap_or_default(),
            folders_note: settings_note,
            // No rig editor without a data root: say where to set one.
            settings_open,
            author_edit: crate::notes::author(),
            importable: Default::default(),
            pool_empty: Default::default(),
            active: tab,
            stage: None,
            calibrators: CalibratorsTool::new(),
            room,
            started: std::time::Instant::now(),
            exit_after_secs,
            shot: std::env::var_os("GS_STUDIO_SHOT").map(|p| Shot { path: PathBuf::from(p), frames: 0, requested: false }),
            report: std::env::var_os("GS_STUDIO_REPORT").filter(|p| !p.is_empty()).map(PathBuf::from),
            import: Import::default(),
            closing: CloseSequence::default(),
            doll,
        }
    }

    fn set_pool(&mut self) {
        self.pool_empty.invalidate();
        match pool_dir::set_in_app(&self.pool_edit) {
            Ok(pool) => {
                let path = pool.path.clone();
                self.folders_note = settings::remember(|s| s.pool = Some(path.clone()));
                if let Ok(tool) = &mut self.rigs {
                    tool.set_pool(path);
                }
                self.pool = pool;
                if let Some(stage) = &mut self.stage {
                    stage.pool_changed();
                }
                // creature_field points at the new pool now: every calibrator
                // state belongs to the old one.
                self.calibrators.pool_changed();
            }
            // An empty box: put the current path back.
            Err(_) => self.pool_edit = self.pool.path.display().to_string(),
        }
    }

    /// (Re)open the rig editor on the current data root.
    fn open_rigs(&mut self) {
        self.rigs = RigTool::open(&self.data.path, self.publish.clone(), Some(self.pool.path.clone()));
    }

    /// The Settings window: the folders (data root, art pool, where Publish
    /// ships, the silhouette chart), each applied with its own button and
    /// remembered in `settings.toml`. Opened from the tab strip's Settings.
    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        // under its button, top right, clear of the tab it is opened over
        let corner = ctx.content_rect().right_top() + egui::vec2(-8.0, 30.0);
        egui::Window::new("Settings")
            .open(&mut open)
            .resizable(true)
            .default_width(620.0)
            .pivot(egui::Align2::RIGHT_TOP)
            .default_pos(corner)
            .show(ctx, |ui| {
            crate::tools::group(ui, "Folders");
            egui::Grid::new("settings_folders").num_columns(3).show(ui, |ui| self.folder_rows(ui));
            if let Some(note) = &self.folders_note {
                ui.add_space(4.0);
                ui.weak(note);
            }
            crate::tools::group(ui, "Notes");
            ui.horizontal(|ui| {
                ui.label("Your name");
                let edit = ui
                    .add(egui::TextEdit::singleline(&mut self.author_edit).desired_width(200.0).hint_text("signs your notes and replies"))
                    .on_hover_text("Notes pinned to a rig or a scene, and replies under them, carry it, so a shared note says who asked");
                if edit.lost_focus() && self.author_edit.trim() != crate::notes::author() {
                    let name = self.author_edit.trim().to_owned();
                    self.folders_note = settings::remember(|s| s.author = (!name.is_empty()).then(|| name.clone()));
                }
            });
            ui.add_space(6.0);
            ui.weak(format!(
                "Remembered in {} with the view choices (Light on the GPU, which panel sections are open) and your name. \
                 A --flag or environment variable wins for its run and is never written.",
                settings::home().join(settings::FILE_NAME).display()
            ));
        });
        self.settings_open = open;
    }

    /// One grid row per folder: its name, its box, and what applies it.
    fn folder_rows(&mut self, ui: &mut egui::Ui) {
        const BOX: f32 = 380.0;
        let dirty = self.dirty();

        ui.label("Data");
        ui.add_sized([BOX, 20.0], egui::TextEdit::singleline(&mut self.data_edit))
            .on_hover_text("Data root (holds outlines.json and rigs/). Also: --data <dir> or GS_STUDIO_DATA");
        let open = ui
            .add_enabled(!dirty, egui::Button::new("Open"))
            .on_disabled_hover_text("Save or discard the current rig first");
        if open.clicked() {
            self.data = DataRoot { path: PathBuf::from(self.data_edit.trim()), source: "set in the app" };
            let path = self.data.path.clone();
            self.folders_note = settings::remember(|s| s.data = Some(path.clone()));
            self.open_rigs();
        }
        ui.end_row();

        ui.label("Pool");
        ui.add_sized([BOX, 20.0], egui::TextEdit::singleline(&mut self.pool_edit)).on_hover_text(
            "Art pool: the directory holding global/images/ and global/scenes/. \
             Left alone it is gs-studio's OWN data home -- this app never reads VellumFE's \
             ~/.vellum-fe unless you point it there. Also: --pool <dir> or GS_STUDIO_POOL",
        );
        ui.horizontal(|ui| {
            if ui
                .button("Set")
                .on_hover_text(
                    "Use this pool directory: re-scans the cast, reloads all art and re-opens every calibrator \
                     (unsaved calibration edits are dropped)",
                )
                .clicked()
            {
                crate::perf_log::log(&format!("begin: set pool to {}", self.pool_edit.trim()));
                self.set_pool();
            }
            self.import_button(ui);
        });
        ui.end_row();

        ui.label("Publish to");
        ui.add_sized([BOX, 20.0], egui::TextEdit::singleline(&mut self.publish_edit)).on_hover_text(
            "Where Publish ships baked rigs, puppets, skins and outfits for game clients: a folder holding \
             rigs/, puppets/, outfits/ (the forms pack's global/, packs/forms/global; a pool installs it). Also: --publish or GS_STUDIO_PUBLISH",
        );
        if ui.button("Set").clicked() {
            let text = self.publish_edit.trim();
            if text.is_empty() {
                self.publish_edit = self.publish.display().to_string();
            } else {
                self.publish = crate::editor::resolve_publish_dir(&self.data.path, Some(Path::new(text)));
                self.publish_edit = self.publish.display().to_string();
                let dir = self.publish.clone();
                self.folders_note = settings::remember(|s| s.publish = Some(dir.clone()));
                if let Ok(tool) = &mut self.rigs {
                    tool.set_publish_dir(self.publish.clone());
                }
            }
        }
        ui.end_row();

        ui.label("Silhouettes");
        ui.add_sized([BOX, 20.0], egui::TextEdit::singleline(&mut self.chart_edit).hint_text("path to scale-chart.html"))
            .on_hover_text("The silhouette chart. Import converts it into outlines.json in the data root.");
        let import = ui
            .add_enabled(!dirty && !self.chart_edit.trim().is_empty(), egui::Button::new("Import"))
            .on_hover_text("Read the chart and (re)write <data>/outlines.json, then reopen the rig editor")
            .on_disabled_hover_text(if dirty { "Save or discard the current rig first" } else { "Type the chart's path first" });
        if import.clicked() {
            let chart = PathBuf::from(self.chart_edit.trim());
            crate::perf_log::log(&format!("begin: import outlines from {}", chart.display()));
            match data_root::import_outlines(&chart, &self.data.path) {
                Ok(summary) => {
                    eprintln!("gs-studio: {summary}");
                    let failed = settings::remember(|s| s.chart = Some(chart.clone()));
                    self.open_rigs();
                    self.folders_note = Some(failed.unwrap_or(summary));
                }
                Err(e) => self.folders_note = Some(format!("Import failed: {e}")),
            }
        }
        ui.end_row();
    }

    /// The scene report (`GS_STUDIO_REPORT`, `--scene-report`): the stage's
    /// last v2 frame, timed offscreen on this GPU -- whole, without each
    /// lamp, and each lamp's shadow alone -- then written as Markdown.
    fn write_report(&self, path: &Path, frame: &eframe::Frame) {
        let Some(stage) = &self.stage else {
            eprintln!("gs-studio: no stage to report on (the Scenes or Preview tab never opened)");
            return;
        };
        let render = frame.wgpu_render_state();
        let pass = creature_field_egui::field_gpu_stage::with_current(|g| g.snapshot()).flatten().map(|snap| {
            let timings = render.map(|r| creature_field_egui::field_gpu_stage::time_lamps(&r.device, &r.queue, &snap, 150));
            let gpu = render.map_or("(unknown GPU)".to_owned(), |r| r.adapter.get_info().name);
            stage.pass_facts(&snap, timings.as_ref(), gpu)
        });
        let text = stage.report(pass.as_ref());
        let written = path.parent().filter(|d| !d.as_os_str().is_empty()).map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(path, &text));
        match written {
            Ok(()) => eprintln!("gs-studio: wrote {}", path.display()),
            Err(e) => eprintln!("gs-studio: could not write {}: {e}", path.display()),
        }
    }

    fn dirty(&self) -> bool {
        self.rigs.as_ref().is_ok_and(RigTool::is_dirty)
    }

    /// What a calibrator's save asks of the stage (VellumFE's Studio:
    /// `drain_outcome` + the scenery tail).
    fn apply_effects(&mut self, effects: HostEffects) {
        if !effects.reload_art {
            return;
        }
        match &mut self.stage {
            Some(stage) => stage.art_changed(effects.invalidate_obstacles),
            // No stage yet, so no art or obstacles are cached; creature_field's
            // process-wide calibration store still is.
            None => creature_field::geometry::invalidate_calibrations(),
        }
    }

    /// The tabs, and Settings (the folders) at the far right.
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            self.tab_strip(ui);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.toggle_value(&mut self.settings_open, "Settings")
                    .on_hover_text("The folders: data root, art pool, where Publish ships, the silhouette chart");
                ui.toggle_value(&mut self.doll.open, "Doll")
                    .on_hover_text("The injury doll: one puppet, wounded and lit by the sky round the player, over any tab");
            });
        });
    }

    fn tab_strip(&mut self, ui: &mut egui::Ui) {
        ui.strong("gs_studio");
        ui.separator();
        for tab in &TOOLS {
            let button = ui.add(egui::Button::selectable(self.active == tab.id, tab.label)).on_hover_text(tab.hint);
            if button.clicked() {
                self.active = tab.id;
            }
        }
    }

    /// "Import from VellumFE...": offers to COPY the game client's art pool
    /// into gs-studio's own home. Disabled, with a reason, when there is no
    /// VellumFE directory to copy from.
    fn import_button(&mut self, ui: &mut egui::Ui) {
        // Not known yet: disabled for the moment it takes to look.
        let source = self.importable.get((), data_home::importable_source).cloned().flatten();
        let enabled = source.is_some();
        let button = ui.add_enabled(enabled, egui::Button::new("Import from VellumFE..."));
        let button = if enabled {
            button.on_hover_text(
                "COPY VellumFE's global/ art pool into this pool directory. \
                 Nothing is moved, linked or written back: your game client is left exactly as it is. \
                 You see what will be copied and confirm before anything happens.",
            )
        } else {
            let where_it_looked = data_home::vellum_fe_dir()
                .map(|d| d.display().to_string())
                .unwrap_or_else(|| "(no home directory)".to_string());
            button.on_disabled_hover_text(format!(
                "No VellumFE data directory to import from.\nLooked in: {where_it_looked}"
            ))
        };
        if button.clicked() {
            if let Some(source) = source {
                let plan = data_home::plan_import(&source);
                self.import.report = None;
                self.import.asking = Some((source, plan));
            }
        }
    }

    /// The import confirmation: what will be copied, and from/to where.
    fn import_window(&mut self, ctx: &egui::Context) {
        let Some((source, plan)) = self.import.asking.clone() else {
            return;
        };
        let mut open = true;
        let mut go = false;
        let mut cancel = false;
        egui::Window::new("Import from VellumFE").collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
            ui.label("Copy VellumFE's art pool into gs-studio's pool directory.");
            ui.add_space(4.0);
            egui::Grid::new("import_paths").num_columns(2).show(ui, |ui| {
                ui.label("From");
                ui.label(source.display().to_string());
                ui.end_row();
                ui.label("To");
                ui.label(self.pool.path.display().to_string());
                ui.end_row();
            });
            ui.add_space(4.0);
            if plan.is_empty() {
                ui.colored_label(ui.visuals().warn_fg_color, "Nothing to copy: that directory has no pool art.");
            } else {
                ui.strong(format!("Will copy {}", plan.summary()));
                for (part, files, bytes) in &plan.parts {
                    ui.weak(format!("    {part}: {files} files, {}", data_home::human_bytes(*bytes)));
                }
                ui.add_space(4.0);
                ui.weak(
                    "COPY only -- nothing is moved, linked or deleted, and nothing is written back into VellumFE. \
                     Files already in the pool are left as they are.",
                );
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                go = ui.add_enabled(!plan.is_empty(), egui::Button::new("Copy")).clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if go {
            crate::perf_log::log(&format!("begin: import from VellumFE ({})", plan.summary()));
            let report = data_home::import(&source, &self.pool.path);
            self.pool_empty.invalidate();
            crate::perf_log::log(&format!("end: {}", report.summary()));
            if let Some(stage) = &mut self.stage {
                stage.pool_changed();
                stage.push_status(report.summary());
            }
            pool_dir::invalidate_caches();
            self.calibrators.pool_changed();
            eprintln!("gs-studio: {} (from {})", report.summary(), source.display());
            for error in &report.errors {
                eprintln!("gs-studio: import error: {error}");
            }
            self.import.report = Some(report);
        }
        if go || cancel || !open {
            self.import.asking = None;
        }
    }

    /// Shown while the pool has no creature art: what directory is in use,
    /// why it is empty, and the two ways to fill it.
    fn first_run_banner(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(ui.visuals().warn_fg_color, "This pool is empty.");
            ui.label(if self.pool.source.is_explicit() {
                format!("No art under {}/global/images/creatures.", self.pool.path.display())
            } else {
                format!(
                    "gs-studio starts with its own data home ({}) rather than VellumFE's, \
                     so there is no art in it yet.",
                    self.pool.path.display()
                )
            });
            self.import_button(ui);
            ui.weak("or drop PNGs into global/images/, or point the Pool box (Settings) at another directory.");
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            match self.active {
                ToolId::Rigs => match &self.rigs {
                    Ok(tool) => tool.status(ui),
                    Err(e) => {
                        ui.colored_label(ui.visuals().error_fg_color, e);
                    }
                },
                ToolId::Scenes | ToolId::Preview => {
                    if let Some(stage) = &self.stage {
                        stage.status(ui);
                    }
                }
                ToolId::Frames | ToolId::Dolls | ToolId::Creatures | ToolId::Scenery => self.calibrators.status(ui),
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.active.uses_pool() {
                    // The answer to "where did it find my artwork": the
                    // path AND whether anything asked for it.
                    ui.weak(self.pool.status()).on_hover_text(if self.pool.source.is_explicit() {
                        "You pointed gs-studio at this directory.".to_string()
                    } else {
                        format!(
                            "Nothing pointed gs-studio anywhere, so this is its own data home.\n\
                             It does NOT use VellumFE's data directory ({}).\n\
                             To use another: --pool <dir>, GS_STUDIO_POOL, or type one in the Pool box (Settings).",
                            data_home::vellum_fe_dir()
                                .map(|d| d.display().to_string())
                                .unwrap_or_else(|| "~/.vellum-fe".to_string())
                        )
                    });
                } else {
                    ui.weak(format!("data: {} ({})", self.data.path.display(), self.data.source));
                }
                if let Some(report) = &self.import.report {
                    ui.separator();
                    ui.weak(report.summary()).on_hover_ui(|ui| {
                        for error in &report.errors {
                            ui.colored_label(ui.visuals().error_fg_color, error);
                        }
                    });
                }
            });
        });
    }
}

impl eframe::App for StudioApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let started = std::time::Instant::now();
        crate::perf_log::begin_frame();
        let tab = self.active;
        self.frame(ui, frame);
        let label = TOOLS.iter().find(|t| t.id == tab).map_or("?", |t| t.label);
        // GS_STUDIO_FRAME_TIMES=1: every frame's time on the UI thread
        if std::env::var_os("GS_STUDIO_FRAME_TIMES").is_some() {
            eprintln!("frame {:.2} ms", started.elapsed().as_secs_f64() * 1000.0);
        }
        if let Some(line) = crate::perf_log::slow_frame_line(started.elapsed(), label, &crate::perf_log::spans_detail()) {
            crate::perf_log::log(&line);
        }
    }
}

impl StudioApp {
    /// One frame of the app (timed by [`eframe::App::ui`]).
    fn frame(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if let Some(shot) = &mut self.shot {
            // a modelled body loading on a worker: not counted, the shot is of it
            if !creature_field_egui::creature_puppet::bodies_loading() {
                shot.frames += 1;
            } else {
                ui.ctx().request_repaint();
            }
            // GS_STUDIO_SHOT_FRAMES waits longer, for work done off the UI
            // thread (scene lighting) to arrive.
            let wait = std::env::var("GS_STUDIO_SHOT_FRAMES").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(30);
            if !shot.requested && shot.frames >= wait {
                shot.requested = true;
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            let image = ui.ctx().input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(image) = image {
                let [w, h] = image.size;
                let rgba: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                match crate::editor::texture::write_png(&shot.path, w, h, &rgba) {
                    Ok(()) => eprintln!("gs-studio: wrote {} ({w}x{h})", shot.path.display()),
                    Err(e) => eprintln!("gs-studio: could not write {}: {e}", shot.path.display()),
                }
                if let Some(path) = &self.report {
                    self.write_report(path, frame);
                }
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ui.ctx().request_repaint();
        }

        if let Some(secs) = self.exit_after_secs {
            if self.started.elapsed().as_secs_f32() >= secs {
                eprintln!("gs-studio: GS_STUDIO_EXIT_AFTER_SECS reached, closing");
                if let Some(stage) = &self.stage {
                    eprintln!("gs-studio: stage: {}", stage.summary());
                }
                eprintln!("gs-studio: calibrators: {}", self.calibrators.summary());
                self.exit_after_secs = None;
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ui.ctx().request_repaint();
        }

        egui::Panel::top("tool_tabs").show(ui, |ui| self.top_bar(ui));
        // First run: the new home is empty, so every pool tab would just be
        // blank. Say so, and say how to fill it.
        let pool = self.pool.path.clone();
        let empty = self.active.uses_pool() && self.pool_empty.get(pool.clone(), move || data_home::is_empty_pool(&pool)).copied().unwrap_or(false);
        if self.pool_empty.pending() || self.importable.pending() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
        if empty {
            egui::Panel::top("first_run").show(ui, |ui| self.first_run_banner(ui));
        }
        egui::Panel::bottom("status_line").show(ui, |ui| self.status_bar(ui));
        let ctx = ui.ctx().clone();
        self.import_window(&ctx);
        self.settings_window(&ctx);
        self.doll.show(&ctx);

        match self.active {
            ToolId::Rigs => match &mut self.rigs {
                Ok(tool) => {
                    tool.ui(ui);
                    if let Some(family) = tool.take_show_on_stage() {
                        let room = &self.room;
                        self.stage.get_or_insert_with(|| StageTool::new(room.clone())).spawn_puppet(&family);
                        self.active = ToolId::Preview;
                    }
                }
                Err(e) => {
                    egui::CentralPanel::default().show(ui, |ui| {
                        ui.heading("No silhouettes yet");
                        ui.label(format!("Could not open the data root: {e}"));
                        ui.label(
                            "In Settings (top right), put the path of scale-chart.html in the Silhouettes box and \
                             press Import: it writes outlines.json into the data root. Or point the Data box at a \
                             directory that already holds outlines.json and press Open.",
                        );
                    });
                }
            },
            ToolId::Scenes | ToolId::Preview => {
                let panel = if self.active == ToolId::Scenes { StagePanel::Scenes } else { StagePanel::Preview };
                let room = &self.room;
                let calibrate_scenery = self.stage.get_or_insert_with(|| StageTool::new(room.clone())).ui(ui, panel);
                if calibrate_scenery {
                    // "Calibrate scenery..." in the Scenes panel.
                    self.active = ToolId::Scenery;
                }
            }
            ToolId::Frames | ToolId::Dolls | ToolId::Creatures | ToolId::Scenery => {
                let effects = match self.active {
                    ToolId::Frames => self.calibrators.ui(ui, CalibKind::Frames),
                    ToolId::Dolls => self.calibrators.ui(ui, CalibKind::Dolls),
                    ToolId::Creatures => self.calibrators.ui(ui, CalibKind::Creatures),
                    _ => self.calibrators.ui(ui, CalibKind::Scenery),
                };
                self.apply_effects(effects);
            }
        }

        self.close_with_unsaved_work(ui.ctx());
    }

    /// Closing the window with unsaved work (the rig, its poses, the stage
    /// scene): each tool asks its own question in turn, drawn over whichever
    /// tab is showing, and the window closes once every one agrees.
    fn close_with_unsaved_work(&mut self, ctx: &egui::Context) {
        if let Ok(tool) = &mut self.rigs {
            tool.modal_prompts(ctx);
        }
        if let Some(stage) = &mut self.stage {
            stage.modal_prompts(ctx);
        }
        let mut guards: Vec<&mut dyn QuitGuard> = Vec::new();
        if let Ok(tool) = &mut self.rigs {
            guards.push(tool);
        }
        if let Some(stage) = &mut self.stage {
            guards.push(stage);
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.closing.close_requested(&mut guards) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        match self.closing.step(&mut guards) {
            CloseStep::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            // An answer given this frame is acted on next frame.
            CloseStep::Waiting => ctx.request_repaint(),
            CloseStep::Idle | CloseStep::Cancelled => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tab_resolves_by_label_in_any_case() {
        assert_eq!(ToolId::from_name("preview"), Ok(ToolId::Preview));
        assert_eq!(ToolId::from_name(" Scenes "), Ok(ToolId::Scenes));
        assert_eq!(ToolId::from_name("RIGS"), Ok(ToolId::Rigs));
        assert_eq!(ToolId::from_name("frames"), Ok(ToolId::Frames));
        assert_eq!(ToolId::from_name("Dolls"), Ok(ToolId::Dolls));
        assert_eq!(ToolId::from_name("creatures"), Ok(ToolId::Creatures));
        assert_eq!(ToolId::from_name("scenery"), Ok(ToolId::Scenery));
        for tab in &TOOLS {
            assert_eq!(ToolId::from_name(tab.label), Ok(tab.id));
        }
        let err = ToolId::from_name("stage").unwrap_err();
        assert!(err.contains("rigs, scenes, frames, dolls, creatures, scenery, preview"), "{err}");
    }

    /// The usage line names exactly the tabs that exist, in order.
    #[test]
    fn the_usage_line_lists_every_tab() {
        let names: Vec<String> = TOOLS.iter().map(|t| t.label.to_ascii_lowercase()).collect();
        assert!(crate::cli::USAGE.contains(&format!("--tab {}]", names.join("|"))), "{}", crate::cli::USAGE);
    }

    #[test]
    fn only_the_rig_editor_works_out_of_the_data_root() {
        for tab in &TOOLS {
            assert_eq!(tab.id.uses_pool(), tab.id != ToolId::Rigs);
        }
    }

    /// The import is offered only against a directory that exists and has
    /// a `global/`, and it never targets the VellumFE directory itself.
    #[test]
    fn the_import_source_must_be_a_real_vellum_fe_directory() {
        let _guard = creature_field::data_dirs::VELLUM_FE_DIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("nothing-here");
        std::env::set_var("VELLUM_FE_DIR", &fake);
        assert_eq!(data_home::vellum_fe_dir().as_deref(), Some(fake.as_path()));
        assert_eq!(data_home::importable_source(), None, "no global/, nothing to import");
        std::fs::create_dir_all(fake.join("global").join("images")).unwrap();
        assert_eq!(data_home::importable_source().as_deref(), Some(fake.as_path()));
        std::env::remove_var("VELLUM_FE_DIR");
    }
}
