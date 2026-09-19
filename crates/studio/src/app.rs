//! The app shell: tool tabs across the top (with the two directory boxes),
//! the active tool in the middle, a status line at the bottom. Tools are
//! added by extending [`TOOLS`] and the `match` in [`StudioApp::ui`].
//!
//! Three tools live behind the seven tabs: the rig editor (Rigs), the stage
//! (Scenes + Preview) and the calibrators (Frames, Dolls, Creatures,
//! Scenery). Each is built lazily and then kept for the life of the app, so
//! switching tabs never loses work.

use std::path::PathBuf;

use crate::calib::{CalibKind, HostEffects};
use crate::data_root::DataRoot;
use crate::pool_dir::{self, PoolDir};
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
    /// "Data:" + "Open" + "Pool:" + "Set" + separators and spacing.
    const BOX_CHROME: f32 = 230.0;
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
}

impl StudioApp {
    pub fn new(startup: Startup) -> StudioApp {
        let Startup { data, pool, tab, room, exit_after_secs } = startup;
        StudioApp {
            data_edit: data.path.display().to_string(),
            rigs: RigTool::open(&data.path),
            data,
            pool_edit: pool.path.display().to_string(),
            pool,
            active: tab,
            stage: None,
            calibrators: CalibratorsTool::new(),
            room,
            started: std::time::Instant::now(),
            exit_after_secs,
        }
    }

    fn set_pool(&mut self) {
        match pool_dir::set_in_app(&self.pool_edit) {
            Ok(pool) => {
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
            self.rigs = RigTool::open(&self.data.path);
        }
        ui.separator();
        ui.label("Pool:");
        ui.add(egui::TextEdit::singleline(&mut self.pool_edit).desired_width(box_width)).on_hover_text(
            "Art pool: the directory holding global/images/ and global/scenes/ (VellumFE's data directory). \
             Also: --pool <dir> or GS_STUDIO_POOL",
        );
        if ui
            .button("Set")
            .on_hover_text(
                "Use this pool directory: re-scans the cast, reloads all art and re-opens every calibrator \
                 (unsaved calibration edits are dropped)",
            )
            .clicked()
        {
            self.set_pool();
        }
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
                    ui.weak(format!("pool: {} ({})", self.pool.path.display(), self.pool.source));
                } else {
                    ui.weak(format!("data: {} ({})", self.data.path.display(), self.data.source));
                }
            });
        });
    }
}

impl eframe::App for StudioApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
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
        egui::Panel::bottom("status_line").show(ui, |ui| self.status_bar(ui));

        match self.active {
            ToolId::Rigs => match &mut self.rigs {
                Ok(tool) => {
                    tool.ui(ui);
                    if tool.quit_approved() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                Err(e) => {
                    egui::CentralPanel::default().show(ui, |ui| {
                        ui.heading("No data");
                        ui.label(format!("Could not open the data root: {e}"));
                        ui.label("Point the Data box above at a directory holding outlines.json, then press Open.");
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
        // Default window: one row, full-size boxes.
        assert_eq!(top_bar_layout(1384.0), (false, 240.0));
        // Minimum window (900 px minus the panel margins): two rows, and
        // the second row's boxes + chrome fit the width.
        let (two_rows, box_width) = top_bar_layout(884.0);
        assert!(two_rows);
        assert!(box_width >= 240.0 && 2.0 * box_width + 230.0 <= 884.0, "{box_width}");
        // Never degenerate, never absurd.
        assert_eq!(top_bar_layout(200.0), (true, 100.0));
        assert!(top_bar_layout(1309.0).0);
        assert_eq!(top_bar_layout(1310.0), (false, 240.0));
    }
}
