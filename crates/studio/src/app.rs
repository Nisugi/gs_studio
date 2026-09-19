//! The app shell: tool tabs across the top (with the two directory boxes),
//! the active tool in the middle, a status line at the bottom. Tools are added by extending [`TOOLS`] and
//! the `match` in [`StudioApp::ui`].

use std::path::PathBuf;

use crate::data_root::DataRoot;
use crate::pool_dir::{self, PoolDir};
use crate::stage::StudioRoomContext;
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
    /// None = built. Some = why the tab is disabled.
    planned: Option<&'static str>,
}

const TOOLS: [ToolTab; 7] = [
    ToolTab { id: ToolId::Rigs, label: "Rigs", planned: None },
    ToolTab { id: ToolId::Scenes, label: "Scenes", planned: None },
    ToolTab { id: ToolId::Frames, label: "Frames", planned: Some("Frame calibrator -- planned") },
    ToolTab { id: ToolId::Dolls, label: "Dolls", planned: Some("Injury-doll calibrator -- planned") },
    ToolTab { id: ToolId::Creatures, label: "Creatures", planned: Some("Creature-frame calibrator -- planned") },
    ToolTab { id: ToolId::Scenery, label: "Scenery", planned: Some("Scenery and prop calibrator -- planned") },
    ToolTab { id: ToolId::Preview, label: "Preview", planned: None },
];

impl ToolId {
    /// `--tab <name>` / `GS_STUDIO_TAB`: a BUILT tool by its label, any case.
    pub fn from_name(name: &str) -> Result<ToolId, String> {
        let built = || TOOLS.iter().filter(|t| t.planned.is_none());
        built().find(|t| t.label.eq_ignore_ascii_case(name.trim())).map(|t| t.id).ok_or_else(|| {
            let names: Vec<String> = built().map(|t| t.label.to_ascii_lowercase()).collect();
            format!("no tool called '{name}' (built: {})", names.join(", "))
        })
    }
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
            }
            // An empty box: put the current path back.
            Err(_) => self.pool_edit = self.pool.path.display().to_string(),
        }
    }

    fn dirty(&self) -> bool {
        self.rigs.as_ref().is_ok_and(RigTool::is_dirty)
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("gs_studio");
            ui.separator();
            for tab in &TOOLS {
                let button = ui.add_enabled(
                    tab.planned.is_none(),
                    egui::Button::selectable(self.active == tab.id, tab.label),
                );
                if button.clicked() {
                    self.active = tab.id;
                }
                if let Some(why) = tab.planned {
                    button.on_disabled_hover_text(why);
                }
            }
            ui.separator();
            ui.label("Data:");
            ui.add(egui::TextEdit::singleline(&mut self.data_edit).desired_width(240.0))
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
            ui.add(egui::TextEdit::singleline(&mut self.pool_edit).desired_width(240.0)).on_hover_text(
                "Art pool: the directory holding global/images/ and global/scenes/ (VellumFE's data directory). \
                 Also: --pool <dir> or GS_STUDIO_POOL",
            );
            if ui.button("Set").on_hover_text("Use this pool directory: re-scans the cast and reloads all art").clicked() {
                self.set_pool();
            }
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let on_stage = matches!(self.active, ToolId::Scenes | ToolId::Preview);
            match (on_stage, &self.stage, &self.rigs) {
                (true, Some(stage), _) => stage.status(ui),
                (true, None, _) => {}
                (false, _, Ok(tool)) => tool.status(ui),
                (false, _, Err(e)) => {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if on_stage {
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
                self.stage.get_or_insert_with(|| StageTool::new(room.clone())).ui(ui, panel);
            }
            // Disabled tabs cannot become active; nothing to draw yet.
            ToolId::Frames | ToolId::Dolls | ToolId::Creatures | ToolId::Scenery => {
                egui::CentralPanel::default().show(ui, |ui| ui.label("Not built yet."));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_resolve_by_label_and_only_when_built() {
        assert_eq!(ToolId::from_name("preview"), Ok(ToolId::Preview));
        assert_eq!(ToolId::from_name(" Scenes "), Ok(ToolId::Scenes));
        assert_eq!(ToolId::from_name("RIGS"), Ok(ToolId::Rigs));
        let err = ToolId::from_name("frames").unwrap_err();
        assert!(err.contains("rigs, scenes, preview"), "{err}");
    }
}
