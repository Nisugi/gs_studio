//! gs-studio: art-authoring app for GemStone IV clients.
//!
//!   gs-studio [--data <dir>] [--pool <dir>]
//!             [--tab rigs|scenes|frames|dolls|creatures|scenery|preview]
//!             [--room-uid <n>] [--room-title <text>] [--room-location <text>]
//!
//! Two directories: the DATA ROOT (`--data` / GS_STUDIO_DATA: outlines.json
//! and rigs/, for the rig editor) and the POOL (`--pool` / GS_STUDIO_POOL:
//! global/images/ and global/scenes/, VellumFE's data directory, for the
//! stage). `GS_STUDIO_TAB` is the env form of `--tab`;
//! `GS_STUDIO_EXIT_AFTER_SECS=<n>` closes the window by itself (smoke runs).
//!
//! Layering: `editor/`, `stage/`, `calib`, `status_log`, `cli`, `data_root`
//! and `pool_dir` are egui-free and unit-tested; `app` (the shell) and
//! `tools/` only draw and forward input.
//!
//! Licence: GPL-3.0-or-later (the stage is ported from VellumFE).

mod app;
mod calib;
mod cli;
mod data_root;
mod editor;
mod pool_dir;
mod stage;
mod status_log;
mod tools;

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(2);
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = cli::parse_args(&args).unwrap_or_else(|usage| fail(&usage));
    let tab_name = cli.tab.clone().or_else(|| std::env::var(cli::TAB_ENV_VAR).ok().filter(|v| !v.is_empty()));
    let tab = match tab_name {
        Some(name) => app::ToolId::from_name(&name).unwrap_or_else(|e| fail(&format!("{e}\n{}", cli::USAGE))),
        None => app::ToolId::Rigs,
    };

    // The rig session reads this each time it opens, so the flag rides the env.
    if let Some(dir) = &cli.publish {
        std::env::set_var("GS_STUDIO_PUBLISH", dir);
    }
    let data = data_root::from_environment(cli.data);
    eprintln!("gs-studio: data root {} ({})", data.path.display(), data.source);
    let pool = pool_dir::from_environment(cli.pool);
    eprintln!("gs-studio: pool {} ({})", pool.path.display(), pool.source);
    let exit_after_secs = std::env::var("GS_STUDIO_EXIT_AFTER_SECS").ok().and_then(|s| s.parse().ok());

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("gs_studio")
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    let startup = app::Startup { data, pool, tab, room: cli.room, exit_after_secs };
    eframe::run_native("gs_studio", options, Box::new(move |_cc| Ok(Box::new(app::StudioApp::new(startup)))))
}
