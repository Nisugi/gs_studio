//! calibrators_demo: every ported calibrator in a bare eframe window.
//!
//!   cargo run -p gs_calibrators --example calibrators_demo
//!
//! A tab strip hosts each calibrator through `ui_contents(ui)` in the
//! central panel (the gs-studio form); the "pop out" toggle hosts them
//! through the `ui(ctx)` floating-window wrapper instead (the VellumFE
//! form). Each opens against the image pool (creature_field's
//! default base dir, `~/.vellum-fe` or `VELLUM_FE_DIR`; override with
//! `GS_STUDIO_POOL=<dir>`). The bottom list shows every
//! `CalibrationOutcome.messages` line plus reload/close events.
//!
//! Smoke-run knobs (the demo itself never clicks or saves anything):
//!   `CALIB_DEMO_OPEN=creatures|scenery|frames|all`  open at start
//!   `CALIB_DEMO_POP_OUT=1|half`     pop out from the start / halfway to the exit
//!   `CALIB_DEMO_EXIT_AFTER_SECS=<n>`                              close by itself

use gs_calibrators::{CalibrationOutcome, CreatureCalibrationState, FrameCalibrationState, SceneryCalibrationState};

#[derive(Default)]
struct Demo {
    creatures: Option<CreatureCalibrationState>,
    scenery: Option<SceneryCalibrationState>,
    frames_cal: Option<FrameCalibrationState>,
    /// Selected tab key (see TABS).
    tab: String,
    /// Host through the `ui(ctx)` window wrapper instead of `ui_contents`.
    pop_out: bool,
    /// Smoke runs: flip `pop_out` on halfway to the exit time.
    auto_pop_out: bool,
    log: Vec<String>,
    auto_open: Vec<String>,
    frames: u64,
    started: Option<std::time::Instant>,
    exit_after: Option<f32>,
}

impl Demo {
    fn new() -> Self {
        if let Ok(dir) = std::env::var("GS_STUDIO_POOL") {
            creature_field::data_dirs::set_base_dir(std::path::PathBuf::from(dir));
            creature_field::pool::invalidate_cache();
        }
        let base = creature_field::pool_dirs::global_images_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|e| format!("<unresolved: {e}>"));
        eprintln!("calibrators_demo: pool = {base}");
        let auto_open = match std::env::var("CALIB_DEMO_OPEN").ok().as_deref() {
            Some("all") => ["creatures", "scenery", "frames"]
                .map(str::to_owned)
                .to_vec(),
            Some(list) => list.split(',').map(|s| s.trim().to_owned()).collect(),
            None => Vec::new(),
        };
        Self {
            log: vec![format!("pool: {base}")],
            auto_open,
            started: Some(std::time::Instant::now()),
            pop_out: std::env::var("CALIB_DEMO_POP_OUT").is_ok_and(|v| v == "1"),
            auto_pop_out: std::env::var("CALIB_DEMO_POP_OUT").is_ok_and(|v| v == "half"),
            exit_after: std::env::var("CALIB_DEMO_EXIT_AFTER_SECS")
                .ok()
                .and_then(|s| s.parse().ok()),
            ..Default::default()
        }
    }

    /// The host half of every calibrator: surface messages, note a reload.
    /// Returns `closed` so the caller drops its state.
    fn absorb(&mut self, who: &str, outcome: CalibrationOutcome) -> bool {
        for message in outcome.messages {
            eprintln!("calibrators_demo: [{who}] {message}");
            self.log.push(format!("[{who}] {message}"));
        }
        if outcome.reload_art {
            // A real host re-resolves its art here (VellumFE:
            // skin_state.force_reload(); gs-studio: CreatureArtStore::force_reload()).
            self.log.push(format!("[{who}] reload_art"));
        }
        if outcome.closed {
            self.log.push(format!("[{who}] closed"));
        }
        outcome.closed
    }

    fn open(&mut self, which: &str) {
        match which {
            "creatures" if self.creatures.is_none() => {
                let (state, outcome) = CreatureCalibrationState::open();
                self.creatures = state;
                self.absorb("creatures", outcome);
            }
            "scenery" if self.scenery.is_none() => {
                let (state, outcome) = SceneryCalibrationState::open();
                self.scenery = state;
                self.absorb("scenery", outcome);
            }
            "frames" if self.frames_cal.is_none() => {
                let (state, outcome) = FrameCalibrationState::open(None);
                self.frames_cal = state;
                self.absorb("frames", outcome);
            }
            other => eprintln!("calibrators_demo: '{other}' already open or unknown"),
        }
        eprintln!(
            "calibrators_demo: open '{which}' -> creatures={} scenery={} frames={}",
            self.creatures.is_some(),
            self.scenery.is_some(),
            self.frames_cal.is_some()
        );
    }
}

/// Run one calibrator for this frame: as a tab body when `$ui` is `Some`,
/// else through its floating-window wrapper. This is all a host writes.
macro_rules! host {
    ($self:ident, $field:ident, $who:literal, $ctx:expr, $ui:expr) => {
        if let Some(mut state) = $self.$field.take() {
            let outcome = match $ui {
                Some(ui) => state.ui_contents(ui),
                None => state.ui($ctx),
            };
            if !$self.absorb($who, outcome) {
                $self.$field = Some(state);
            }
        }
    };
}

const TABS: [(&str, &str); 3] = [("Creatures", "creatures"), ("Scenery", "scenery"), ("Frames", "frames")];

impl eframe::App for Demo {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.frames += 1;
        if self.frames == 2 {
            for which in std::mem::take(&mut self.auto_open) {
                self.open(&which);
                self.tab = which;
            }
        }
        if self.frames == 1 || self.frames == 60 {
            eprintln!(
                "calibrators_demo: frame {} ok (tab '{}', pop_out={})",
                self.frames, self.tab, self.pop_out
            );
        }

        let mut want: Option<&'static str> = None;
        egui::Panel::top("demo_bar").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                // The tab strip: selecting a tab opens its calibrator.
                for (label, key) in TABS {
                    if ui.selectable_label(self.tab == key, label).clicked() {
                        self.tab = key.to_owned();
                        want = Some(key);
                    }
                }
                ui.separator();
                ui.checkbox(&mut self.pop_out, "pop out")
                    .on_hover_text("Host every open calibrator through ui(ctx), the floating-window wrapper, instead of ui_contents(ui) in the tab");
                ui.separator();
                if ui.button("Clear log").clicked() {
                    self.log.clear();
                }
            });
        });
        if let Some(which) = want {
            self.open(which);
        }

        egui::Panel::bottom("demo_log")
            .resizable(true)
            .default_size(120.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for line in &self.log {
                            ui.label(line);
                        }
                    });
            });

        let pop_out = self.pop_out;
        let tab = self.tab.clone();
        egui::CentralPanel::default().show(ui, |ui| {
            if pop_out {
                ui.weak("(pop out: open calibrators are floating windows)");
                return;
            }
            // TAB form: only the selected tab's calibrator draws.
            match tab.as_str() {
                "creatures" => host!(self, creatures, "creatures", &ctx, Some(&mut *ui)),
                "scenery" => host!(self, scenery, "scenery", &ctx, Some(&mut *ui)),
                "frames" => host!(self, frames_cal, "frames", &ctx, Some(&mut *ui)),
                _ => {
                    ui.weak("Pick a tab.");
                }
            }
        });
        if pop_out {
            // WINDOW form: every open calibrator, VellumFE style.
            host!(self, creatures, "creatures", &ctx, None::<&mut egui::Ui>);
            host!(self, scenery, "scenery", &ctx, None::<&mut egui::Ui>);
            host!(self, frames_cal, "frames", &ctx, None::<&mut egui::Ui>);
        }

        if let (Some(secs), Some(started)) = (self.exit_after, self.started) {
            // Smoke runs flip to the pop-out path halfway through, so both
            // hosting forms run unattended.
            if self.auto_pop_out && !self.pop_out && started.elapsed().as_secs_f32() >= secs / 2.0 {
                self.pop_out = true;
                eprintln!("calibrators_demo: frame {}: switching to pop out", self.frames);
            }
            if started.elapsed().as_secs_f32() >= secs {
                eprintln!("calibrators_demo: {} frames, exiting cleanly", self.frames);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ctx.request_repaint();
        }
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("gs_calibrators demo")
            .with_inner_size([1200.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        "gs_calibrators_demo",
        options,
        Box::new(|_cc| Ok(Box::new(Demo::new()))),
    )
}
