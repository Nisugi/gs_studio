//! gs-studio: art-authoring app for GemStone IV clients.
//!
//!   gs-studio [--data <dir>]        (or set GS_STUDIO_DATA)
//!
//! Layering: `editor/` and `data_root` are egui-free and unit-tested;
//! `app` (the shell) and `tools/` only draw and forward input.

mod app;
mod data_root;
mod editor;
mod tools;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = match data_root::parse_args(&args) {
        Ok(cli) => cli,
        Err(usage) => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
    };
    let data = data_root::from_environment(cli);
    eprintln!("gs-studio: data root {} ({})", data.path.display(), data.source);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("gs_studio")
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native("gs_studio", options, Box::new(move |_cc| Ok(Box::new(app::StudioApp::new(data)))))
}
