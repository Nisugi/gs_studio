//! gs-studio: art-authoring app for GemStone IV clients.
//!
//!   gs-studio [--data <dir>] [--pool <dir>]
//!             [--tab rigs|scenes|frames|dolls|creatures|scenery|preview]
//!             [--room-uid <n>] [--room-title <text>] [--room-location <text>]
//!
//! Two directories: the DATA ROOT (`--data` / GS_STUDIO_DATA: outlines.json
//! and rigs/, for the rig editor) and the POOL (`--pool` / GS_STUDIO_POOL:
//! global/images/ and global/scenes/, for the stage and the calibrators).
//! `GS_STUDIO_TAB` is the env form of `--tab`;
//! `GS_STUDIO_EXIT_AFTER_SECS=<n>` closes the window by itself (smoke runs).
//!
//! gs-studio is a STANDALONE app: with no `--pool` / `GS_STUDIO_POOL` the
//! pool is gs-studio's OWN data home (`data_home`), never VellumFE's
//! `~/.vellum-fe`. See `pool_dir` for why, and "Import from VellumFE..." in
//! the top bar for getting art across.
//!
//! Layering: `editor/`, `stage/`, `calib`, `status_log`, `cli`, `data_root`,
//! `data_home` and `pool_dir` are egui-free and unit-tested; `app` (the
//! shell) and `tools/` only draw and forward input.
//!
//! Licence: GPL-3.0-or-later (the stage is ported from VellumFE).

mod app;
mod bundle;
mod calib;
mod cli;
mod data_home;
mod data_root;
mod editor;
mod pool_dir;
mod settings;
mod stage;

use std::path::PathBuf;
mod status_log;
mod tools;

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(2);
}

/// Fill gs-studio's OWN folders from the art and rigs packed into the
/// binary (the `bundled-art` feature), adding only what is missing. A pool or
/// data root the user pointed elsewhere is never touched.
fn seed_from_bundle(data: &data_root::DataRoot, pool: &pool_dir::PoolDir) {
    let home = data_home::default_home_or_local();
    let targets = [
        (bundle::Root::Pool, "art", pool.path.clone(), pool.source == pool_dir::PoolSource::StudioHome),
        (bundle::Root::Data, "rigs", data.path.clone(), data.path == home.join("data")),
    ];
    for (root, what, dest, ours) in targets {
        if !ours || bundle::contents(root).0 == 0 {
            continue;
        }
        let report = bundle::seed(root, &dest);
        if report.written > 0 || !report.errors.is_empty() {
            eprintln!("gs-studio: {}", report.summary(what, &dest));
        }
    }
    pool_dir::invalidate_caches();
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = cli::parse_args(&args).unwrap_or_else(|usage| fail(&usage));
    let tab_name = cli.tab.clone().or_else(|| std::env::var(cli::TAB_ENV_VAR).ok().filter(|v| !v.is_empty()));
    let tab = match tab_name {
        Some(name) => app::ToolId::from_name(&name).unwrap_or_else(|e| fail(&format!("{e}\n{}", cli::USAGE))),
        None => app::ToolId::Rigs,
    };

    // Every folder: --flag > environment variable > settings.toml > default.
    let (saved, settings_note) = settings::Settings::load(&settings::home());
    if let Some(note) = &settings_note {
        eprintln!("gs-studio: {note}");
    }
    let data = data_root::from_environment(cli.data, saved.data);
    eprintln!("gs-studio: data root {} ({})", data.path.display(), data.source);
    let publish_env = std::env::var_os(editor::PUBLISH_ENV_VAR).filter(|v| !v.is_empty()).map(PathBuf::from);
    let publish_override = cli.publish.or(publish_env).or(saved.publish);
    let publish = editor::resolve_publish_dir(&data.path, publish_override.as_deref());
    eprintln!("gs-studio: publish to {}", publish.display());
    let pool = pool_dir::from_environment(cli.pool, saved.pool);
    seed_from_bundle(&data, &pool);
    eprintln!("gs-studio: {}", pool.status());
    if !pool.source.is_explicit() {
        eprintln!("gs-studio: nothing pointed the pool anywhere, so this is gs-studio's own data home.");
        eprintln!("gs-studio: it does NOT read ~/.vellum-fe -- use --pool, GS_STUDIO_POOL or the Pool box for that.");
    }
    let exit_after_secs = std::env::var("GS_STUDIO_EXIT_AFTER_SECS").ok().and_then(|s| s.parse().ok());

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("gs_studio")
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    let startup =
        app::Startup { data, pool, publish, chart: saved.chart, settings_note, tab, room: cli.room, exit_after_secs };
    eframe::run_native("gs_studio", options, Box::new(move |_cc| Ok(Box::new(app::StudioApp::new(startup)))))
}
