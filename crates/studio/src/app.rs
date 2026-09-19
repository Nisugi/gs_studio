//! The app shell: tool tabs across the top, the active tool in the middle,
//! a status line at the bottom. Tools are added by extending [`TOOLS`] and
//! the `match` in [`StudioApp::ui`].

use std::path::PathBuf;

use crate::data_root::DataRoot;
use crate::tools::rigs::RigTool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolId {
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
    ToolTab { id: ToolId::Scenes, label: "Scenes", planned: Some("Scene builder -- planned") },
    ToolTab { id: ToolId::Frames, label: "Frames", planned: Some("Frame calibrator -- planned") },
    ToolTab { id: ToolId::Dolls, label: "Dolls", planned: Some("Injury-doll calibrator -- planned") },
    ToolTab { id: ToolId::Creatures, label: "Creatures", planned: Some("Creature-frame calibrator -- planned") },
    ToolTab { id: ToolId::Scenery, label: "Scenery", planned: Some("Scenery and prop calibrator -- planned") },
    ToolTab { id: ToolId::Preview, label: "Preview", planned: Some("Preview stage -- planned") },
];

pub struct StudioApp {
    data: DataRoot,
    /// The data-root text box (applied with "Open").
    data_edit: String,
    active: ToolId,
    rigs: Result<RigTool, String>,
}

impl StudioApp {
    pub fn new(data: DataRoot) -> StudioApp {
        StudioApp {
            data_edit: data.path.display().to_string(),
            rigs: RigTool::open(&data.path),
            data,
            active: ToolId::Rigs,
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
            ui.add(egui::TextEdit::singleline(&mut self.data_edit).desired_width(320.0))
                .on_hover_text("Data root (holds outlines.json and rigs/). Also: --data <dir> or GS_STUDIO_DATA");
            let dirty = self.dirty();
            let open = ui
                .add_enabled(!dirty, egui::Button::new("Open"))
                .on_disabled_hover_text("Save or discard the current rig first");
            if open.clicked() {
                self.data = DataRoot { path: PathBuf::from(self.data_edit.trim()), source: "set in the app" };
                self.rigs = RigTool::open(&self.data.path);
            }
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            match &self.rigs {
                Ok(tool) => tool.status(ui),
                Err(e) => {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak(format!("data: {} ({})", self.data.path.display(), self.data.source));
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
            // Disabled tabs cannot become active; nothing to draw yet.
            _ => {
                egui::CentralPanel::default().show(ui, |ui| ui.label("Not built yet."));
            }
        }
    }
}
