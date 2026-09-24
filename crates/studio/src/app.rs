//! The app shell: tool tabs across the top (with the two directory boxes),
//! the active tool in the middle, a status line at the bottom. Tools are
//! added by extending [`TOOLS`] and the `match` in [`StudioApp::ui`].
//!
//! Three tools live behind the seven tabs: the rig editor (Rigs), the stage
//! (Scenes + Preview) and the calibrators (Frames, Dolls, Creatures,
//! Scenery). Each is built lazily and then kept for the life of the app, so
//! switching tabs never loses work.

use std::path::{Path, PathBuf};

use crate::calib::{CalibKind, HostEffects};
use crate::data_home::{self, ImportPlan, ImportReport};
use crate::data_root::{self, DataRoot};
use crate::pool_dir::{self, PoolDir};
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
    ToolTab { id: ToolId::Frames, label: "Frames", hint: "Frame calibrator: nine-slice geometry per frame image (pool)" },
    ToolTab { id: ToolId::Dolls, label: "Dolls", hint: "Injury-doll calibrators: wound-dot anchors, and doll sets (pool)" },
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

/// The top bar at a given width: everything on one row when it fits, else
/// the tabs on the first row and the two directory boxes sharing a second.
/// Returns (two rows?, width of each directory text box).
pub fn top_bar_layout(available_width: f32) -> (bool, f32) {
    /// Title + seven tabs + separators, generously measured.
    const TABS: f32 = 600.0;
    /// "Data:" + "Open" + "Pool:" + "Set" + "Import from VellumFE..." +
    /// separators and spacing.
    const BOX_CHROME: f32 = 400.0;
    const BOX: f32 = 240.0;
    if available_width >= TABS + BOX_CHROME + 2.0 * BOX {
        return (false, BOX);
    }
    (true, ((available_width - BOX_CHROME) / 2.0).clamp(100.0, 420.0))
}

/// What `main` hands the app.
pub struct Startup {
    pub data: DataRoot,
    pub pool: PoolDir,
    /// Where Publish ships (`rigs/`, `puppets/`, `poses/` under it).
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
    /// The art pool / scenes directory (gs_field's base dir).
    pool: PoolDir,
    /// The pool text box (applied with "Set").
    pool_edit: String,
    publish: PathBuf,
    /// The publish text box (applied with "Set").
    publish_edit: String,
    /// The silhouette chart box (applied with "Import").
    chart_edit: String,
    /// The last folder-strip message (an import, a failed remember).
    folders_note: Option<String>,
    /// Filesystem answers the top bar needs every frame (see `fs_cache`).
    importable: crate::fs_cache::Ttl<(), Option<PathBuf>>,
    pool_empty: crate::fs_cache::Ttl<PathBuf, bool>,
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
    /// "Import from VellumFE...": the open confirmation, and the last
    /// report. See [`Import`].
    import: Import,
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
        StudioApp {
            data_edit: data.path.display().to_string(),
            rigs: RigTool::open(&data.path, publish.clone(), Some(pool.path.clone())),
            data,
            pool_edit: pool.path.display().to_string(),
            pool,
            publish_edit: publish.display().to_string(),
            publish,
            chart_edit: chart.map(|c| c.display().to_string()).unwrap_or_default(),
            folders_note: settings_note,
            importable: Default::default(),
            pool_empty: Default::default(),
            active: tab,
            stage: None,
            calibrators: CalibratorsTool::new(),
            room,
            started: std::time::Instant::now(),
            exit_after_secs,
            import: Import::default(),
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
                // gs_field points at the new pool now: every calibrator
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

    /// The Rigs tab's folder strip: the silhouette chart (Import writes
    /// `<data>/outlines.json` from it) and where Publish ships.
    fn folders_strip(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Silhouettes:");
            ui.add(egui::TextEdit::singleline(&mut self.chart_edit).hint_text("path to scale-chart.html").desired_width(300.0))
                .on_hover_text("The silhouette chart. Import converts it into outlines.json in the data root.");
            let dirty = self.dirty();
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
            ui.separator();
            ui.label("Publish to:");
            ui.add(egui::TextEdit::singleline(&mut self.publish_edit).desired_width(300.0)).on_hover_text(
                "Where Publish ships baked rigs, puppets and pose sets for game clients: a folder holding \
                 rigs/, puppets/, poses/ (the workspace's crates/gs_field/data). Also: --publish or GS_STUDIO_PUBLISH",
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
            if let Some(note) = &self.folders_note {
                ui.separator();
                ui.weak(note);
            }
        });
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
            // No stage yet, so no art or obstacles are cached; gs_field's
            // process-wide calibration store still is.
            None => gs_field::geometry::invalidate_calibrations(),
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let (two_rows, box_width) = top_bar_layout(ui.available_width());
        // Wrapped rows: whatever the fonts measure, nothing clips.
        ui.horizontal_wrapped(|ui| {
            self.tab_strip(ui);
            if !two_rows {
                ui.separator();
                self.directory_boxes(ui, box_width);
            }
        });
        if two_rows {
            ui.horizontal_wrapped(|ui| self.directory_boxes(ui, box_width));
        }
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

    fn directory_boxes(&mut self, ui: &mut egui::Ui, box_width: f32) {
        ui.label("Data:");
        ui.add(egui::TextEdit::singleline(&mut self.data_edit).desired_width(box_width))
            .on_hover_text("Data root (holds outlines.json and rigs/). Also: --data <dir> or GS_STUDIO_DATA");
        let dirty = self.dirty();
        let open = ui
            .add_enabled(!dirty, egui::Button::new("Open"))
            .on_disabled_hover_text("Save or discard the current rig first");
        if open.clicked() {
            self.data = DataRoot { path: PathBuf::from(self.data_edit.trim()), source: "set in the app" };
            let path = self.data.path.clone();
            self.folders_note = settings::remember(|s| s.data = Some(path.clone()));
            self.open_rigs();
        }
        ui.separator();
        ui.label("Pool:");
        ui.add(egui::TextEdit::singleline(&mut self.pool_edit).desired_width(box_width)).on_hover_text(
            "Art pool: the directory holding global/images/ and global/scenes/. \
             Left alone it is gs-studio's OWN data home -- this app never reads VellumFE's \
             ~/.vellum-fe unless you point it there. Also: --pool <dir> or GS_STUDIO_POOL",
        );
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
    }

    /// "Import from VellumFE...": offers to COPY the game client's art pool
    /// into gs-studio's own home. Disabled, with a reason, when there is no
    /// VellumFE directory to copy from.
    fn import_button(&mut self, ui: &mut egui::Ui) {
        let source = self.importable.get((), data_home::importable_source).clone();
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
            ui.weak("or drop PNGs into global/images/, or point the Pool box at another directory.");
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
                             To use another: --pool <dir>, GS_STUDIO_POOL, or type one in the Pool box.",
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
        let tab = self.active;
        self.frame(ui, frame);
        let label = TOOLS.iter().find(|t| t.id == tab).map_or("?", |t| t.label);
        if let Some(line) = crate::perf_log::slow_frame_line(started.elapsed(), label, "") {
            crate::perf_log::log(&line);
        }
    }
}

impl StudioApp {
    /// One frame of the app (timed by [`eframe::App::ui`]).
    fn frame(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Closing the window with unsaved work goes through the same
        // discard prompt as everything else.
        if ui.ctx().input(|i| i.viewport().close_requested()) {
            if let Ok(tool) = &mut self.rigs {
                if !tool.approve_quit() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
                }
            }
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
        let empty = self.active.uses_pool() && *self.pool_empty.get(self.pool.path.clone(), || data_home::is_empty_pool(&self.pool.path));
        if empty {
            egui::Panel::top("first_run").show(ui, |ui| self.first_run_banner(ui));
        }
        egui::Panel::bottom("status_line").show(ui, |ui| self.status_bar(ui));
        let ctx = ui.ctx().clone();
        self.import_window(&ctx);

        if self.active == ToolId::Rigs {
            egui::Panel::top("rig_folders").show(ui, |ui| self.folders_strip(ui));
        }

        match self.active {
            ToolId::Rigs => match &mut self.rigs {
                Ok(tool) => {
                    tool.ui(ui);
                    if tool.quit_approved() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
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
                            "Put the path of scale-chart.html in the Silhouettes box above and press Import: \
                             it writes outlines.json into the data root. Or point the Data box at a directory \
                             that already holds outlines.json and press Open.",
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
                    ToolId::Creatures => self.calibrators.ui(ui, CalibKind::Creatures),
                    ToolId::Scenery => self.calibrators.ui(ui, CalibKind::Scenery),
                    _ => self.calibrators.dolls_ui(ui),
                };
                self.apply_effects(effects);
            }
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

    #[test]
    fn the_top_bar_goes_to_two_rows_before_anything_clips() {
        // The chrome allowance grew when "Import from VellumFE..." joined
        // the boxes, so the one-row threshold moved with it: tabs (600) +
        // chrome (400) + two boxes (480) = 1480.
        assert_eq!(top_bar_layout(1480.0), (false, 240.0));
        assert!(top_bar_layout(1479.0).0, "one pixel short goes to two rows");
        // The default window is now a two-row window; it must still fit.
        let (two_rows, box_width) = top_bar_layout(1384.0);
        assert!(two_rows);
        assert!(2.0 * box_width + 400.0 <= 1384.0, "{box_width}");
        // Minimum window (900 px minus the panel margins): two rows, and
        // the second row's boxes + chrome fit the width.
        let (two_rows, box_width) = top_bar_layout(884.0);
        assert!(two_rows);
        assert!(box_width >= 240.0 && 2.0 * box_width + 400.0 <= 884.0, "{box_width}");
        // Never degenerate, never absurd.
        assert_eq!(top_bar_layout(200.0), (true, 100.0));
    }

    /// The import is offered only against a directory that exists and has
    /// a `global/`, and it never targets the VellumFE directory itself.
    #[test]
    fn the_import_source_must_be_a_real_vellum_fe_directory() {
        let _guard = gs_field::data_dirs::VELLUM_FE_DIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
