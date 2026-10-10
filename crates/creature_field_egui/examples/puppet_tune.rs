//! puppet_tune: a contact sheet of the creature puppet at the sizes and
//! poses the creature field actually draws it at, for judging the look.
//!
//!   PUPPET_TUNE_SHOT=sheet.png cargo run -p creature_field_egui --example puppet_tune
//!
//! Rows are the drawn standing heights a card asks for; columns are the
//! poses and states. Nothing here is part of the renderer -- it calls the
//! same `creature_puppet` code the field does, so what it shows is what the
//! field draws.

use creature_field_egui::creature_puppet::{PoseChoice, PuppetCache, PuppetKey, PuppetPlacement};
use creature_field_egui::field_puppets::FieldCache;

const HEIGHTS: [f32; 4] = [36.0, 64.0, 110.0, 190.0];

/// `(family, label, pose, wounds, dead, boss)`.
type Case = (&'static str, &'static str, PoseChoice, Vec<(String, u8)>, bool, bool);

fn cases() -> Vec<Case> {
    let w = |list: &[(&str, u8)]| -> Vec<(String, u8)> {
        list.iter().map(|(p, r)| (p.to_string(), *r)).collect()
    };
    vec![
        ("humanoid", "stand", PoseChoice::Standing, w(&[]), false, false),
        ("humanoid", "chest2", PoseChoice::Standing, w(&[("chest", 2)]), false, false),
        ("humanoid", "arm3", PoseChoice::Standing, w(&[("leftArm", 3)]), false, false),
        ("humanoid", "kneel", PoseChoice::Kneeling, w(&[]), false, false),
        ("humanoid", "stunned", PoseChoice::Stunned, w(&[]), false, false),
        ("humanoid", "prone", PoseChoice::Prone { on_side: true }, w(&[]), false, false),
        ("humanoid", "dead", PoseChoice::Dead { variant: 0 }, w(&[]), true, false),
        ("humanoid", "boss", PoseChoice::Standing, w(&[]), false, true),
        ("canine", "stand", PoseChoice::Standing, w(&[]), false, false),
        ("canine", "sit", PoseChoice::Sitting, w(&[]), false, false),
        ("canine", "prone", PoseChoice::Prone { on_side: true }, w(&[]), false, false),
        ("canine", "dead", PoseChoice::Dead { variant: 0 }, w(&[]), true, false),
    ]
}

struct Sheet {
    frames: u64,
    shot: Option<std::path::PathBuf>,
    requested: bool,
}

impl eframe::App for Sheet {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.frames += 1;
        egui::CentralPanel::default().show(ui, |ui| {
            let painter = ui.painter().clone();
            let full = ui.max_rect();
            painter.rect_filled(full, 0.0, egui::Color32::from_gray(26));
            let shared = PuppetCache::shared(&ctx);
            let mut cache = shared.lock().unwrap();
            cache.begin_frame(ctx.cumulative_frame_nr());

            let cases = cases();
            let col_w = full.width() / cases.len() as f32;
            let mut y = full.top() + 18.0;
            for h in HEIGHTS {
                let row_h = h + 26.0;
                painter.text(
                    egui::pos2(full.left() + 4.0, y),
                    egui::Align2::LEFT_TOP,
                    format!("{h:.0}px -> step {}", creature_field_egui::creature_puppet::size_step(h)),
                    egui::FontId::monospace(11.0),
                    egui::Color32::from_gray(150),
                );
                let base = y + row_h;
                for (i, (family, label, pose, wounds, dead, boss)) in cases.iter().enumerate() {
                    let family: std::sync::Arc<str> = std::sync::Arc::from(*family);
                    let body = if *family == *"canine" {
                        [140u8, 91, 117]
                    } else {
                        [91u8, 135, 140]
                    };
                    let key = PuppetKey::new(*pose, wounds, *dead, *boss, body, i % 2 == 1, h);
                    let Some(posed) = cache.posed(&family, &key) else {
                        continue;
                    };
                    let foot = egui::pos2(full.left() + col_w * (i as f32 + 0.5), base);
                    let place = PuppetPlacement::fit(&posed, foot, h);
                    place.paint(&painter, &posed.shadow);
                    place.paint(&painter, &posed.prims);
                    if h == HEIGHTS[0] {
                        painter.text(
                            egui::pos2(foot.x, full.top() + 2.0),
                            egui::Align2::CENTER_TOP,
                            label,
                            egui::FontId::monospace(10.0),
                            egui::Color32::from_gray(170),
                        );
                    }
                    painter.text(
                        egui::pos2(foot.x, base + 2.0),
                        egui::Align2::CENTER_TOP,
                        format!("{}", posed.prims.len()),
                        egui::FontId::monospace(9.0),
                        egui::Color32::from_gray(110),
                    );
                }
                y += row_h + 16.0;
            }
        });

        if let Some(path) = self.shot.clone() {
            if !self.requested && self.frames >= 12 {
                self.requested = true;
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
                    Ok(()) => eprintln!("puppet_tune: wrote {} ({w}x{h})", path.display()),
                    Err(e) => eprintln!("puppet_tune: {}: {e}", path.display()),
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ctx.request_repaint();
        }
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("puppet tune")
            .with_inner_size([1500.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "gs_field_egui_puppet_tune",
        options,
        Box::new(|_cc| {
            Ok(Box::new(Sheet {
                frames: 0,
                shot: std::env::var_os("PUPPET_TUNE_SHOT").map(std::path::PathBuf::from),
                requested: false,
            }))
        }),
    )
}
