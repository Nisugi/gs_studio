//! gs-studio: art-authoring app for GemStone IV clients.
//!
//!   gs-studio [--data <dir>] [--pool <dir>]
//!             [--tab rigs|scenes|frames|dolls|creatures|scenery|preview]
//!             [--room-uid <n>] [--room-title <text>] [--room-location <text>]
//!   gs-studio [--data <dir>] --project <family>[/<creature>]:side|back [--grazing <facing>]
//!   gs-studio [--data <dir>] --build-sheet <family>[/<creature>]
//!   gs-studio [--data <dir>] [--pool <dir>] [--publish <dir>] --save <family>[,<family>...]
//!   gs-studio [--data <dir>] --import-chart <scale-chart.html>
//!   gs-studio [--data <dir>] --export-jobs
//!
//! Both run headless, for a texture generator to call, and exit: `--project`
//! writes the projection-first renders it paints the side and back from
//! (`editor::texture::project_cli`); `--export-jobs` writes every family's
//! and outfit's templates and `textures/jobs.json`
//! (`editor::texture::export_jobs`).
//!
//! Two directories: the DATA ROOT (`--data` / GS_STUDIO_DATA: outlines.json
//! and rigs/, for the rig editor) and the POOL (`--pool` / GS_STUDIO_POOL:
//! global/images/ and global/scenes/, for the stage and the calibrators).
//! `GS_STUDIO_TAB` is the env form of `--tab`;
//! `GS_STUDIO_EXIT_AFTER_SECS=<n>` closes the window by itself (smoke runs);
//! `GS_STUDIO_SHOT=<path.png>` saves one frame of the window there and
//! closes (after 30 frames, or `GS_STUDIO_SHOT_FRAMES`); `GS_STUDIO_POSE_EDIT=[<family>/]<pose>[:<bone>]` opens the Rigs tab's 3D
//! preview with the pose editor on that pose (and row) -- visual checks;
//! `GS_STUDIO_NOTE=<family>#<id>` opens it on a note pinned to the puppet
//! (`editor::rig_notes`).
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
mod bodies;
mod bundle;
mod calib;
mod cli;
mod data_home;
mod data_root;
mod doll_source;
mod editor;
mod notes;
mod fs_cache;
mod perf_log;
mod pool_dir;
mod quit;
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
    // --scene-report: the scene on the Scenes tab, lit by the v2 pass (at
    // 23:00 unless GS_STUDIO_LIGHT_HOUR says), and once the night relight
    // is in, its report and a screenshot in scene-reports/, then exit.
    if let Some(scene) = &cli.scene_report {
        let dir = std::path::Path::new("scene-reports");
        if let Err(e) = std::fs::create_dir_all(dir) {
            fail(&format!("gs-studio: --scene-report: {}: {e}", dir.display()));
        }
        let unless_set = |key: &str, value: &str| {
            if std::env::var_os(key).is_none() {
                std::env::set_var(key, value);
            }
        };
        std::env::set_var("GS_STUDIO_SCENE", scene);
        std::env::set_var("GS_STUDIO_V2_LIGHT", "1");
        std::env::set_var(cli::TAB_ENV_VAR, "scenes");
        unless_set("GS_STUDIO_LIGHT_HOUR", "23");
        // a crowd on it: a shadow's cost is marching every creature, so an
        // empty stage would say the lamps cost nothing
        unless_set("GS_STUDIO_SMOKE", "1");
        unless_set("GS_STUDIO_SHOT_FRAMES", "2500");
        std::env::set_var("GS_STUDIO_SHOT", dir.join(format!("{scene}.png")));
        std::env::set_var("GS_STUDIO_REPORT", dir.join(format!("{scene}.md")));
    }
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
    let gpu_light = saved.gpu_light == Some(true);
    let data = data_root::from_environment(cli.data, saved.data);
    eprintln!("gs-studio: data root {} ({})", data.path.display(), data.source);
    if let Some(target) = &cli.project {
        match editor::texture::project_cli(&data.path, target, cli.grazing) {
            Ok(files) => {
                for f in files {
                    println!("{}", f.display());
                }
                std::process::exit(0);
            }
            Err(e) => fail(&format!("gs-studio: --project: {e}")),
        }
    }
    if let Some(chart) = &cli.import_chart {
        match data_root::import_outlines(chart, &data.path) {
            Ok(summary) => {
                println!("{summary}");
                std::process::exit(0);
            }
            Err(e) => fail(&format!("gs-studio: --import-chart: {e}")),
        }
    }
    if let Some(target) = &cli.build_sheet {
        match editor::texture::build_sheet_cli(&data.path, target) {
            Ok((path, count)) => {
                println!("{} ({count} paintings)", path.display());
                std::process::exit(0);
            }
            Err(e) => fail(&format!("gs-studio: --build-sheet: {e}")),
        }
    }
    if let Some(pool) = &cli.install_doll {
        match bodies::install_doll(&data.path, pool) {
            Ok(forms) => {
                println!("{}: {}", pool.display(), if forms.is_empty() { "nothing changed".to_owned() } else { forms.join(", ") });
                std::process::exit(0);
            }
            Err(e) => fail(&format!("gs-studio: --install-doll: {e}")),
        }
    }
    if cli.export_jobs {
        let outlines = rig::Outlines::load(&data.path.join("outlines.json")).unwrap_or_else(|e| fail(&format!("gs-studio: outlines.json: {e}")));
        match editor::texture::export_jobs(&data.path, &outlines) {
            Ok(done) => {
                for s in &done.skipped {
                    eprintln!("gs-studio: --export-jobs: skipped {s}");
                }
                println!("{} jobs: {}", done.jobs, done.path.display());
                std::process::exit(0);
            }
            Err(e) => fail(&format!("gs-studio: --export-jobs: {e}")),
        }
    }
    let publish_env = std::env::var_os(editor::PUBLISH_ENV_VAR).filter(|v| !v.is_empty()).map(PathBuf::from);
    let publish_override = cli.publish.or(publish_env).or(saved.publish);
    let publish = editor::resolve_publish_dir(&data.path, publish_override.as_deref());
    eprintln!("gs-studio: publish to {}", publish.display());
    let pool = pool_dir::from_environment(cli.pool, saved.pool);
    seed_from_bundle(&data, &pool);
    eprintln!("gs-studio: {}", pool.status());
    if let Some(families) = &cli.save {
        // The Rigs tab's Save, headless: bake, puppet, skins and outfits into
        // the pool, Publish.
        let mut session = editor::Session::open(&data.path, publish.clone(), Some(pool.path.clone())).unwrap_or_else(|e| fail(&format!("gs-studio: --save: {e}")));
        for family in families.split(',').map(str::trim).filter(|f| !f.is_empty()) {
            session.request(editor::DocAction::OpenFamily(family.to_owned()));
            if session.editor.rig().family != family {
                fail(&format!("gs-studio: --save: no rig '{family}' ({})", session.status));
            }
            match session.save() {
                Ok(()) => println!("{family}: {}", session.status),
                Err(e) => fail(&format!("gs-studio: --save {family}: {e}")),
            }
        }
        std::process::exit(0);
    }
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
    eframe::run_native(
        "gs_studio",
        options,
        Box::new(move |cc| {
            // The v2 field lighting pass needs the wgpu renderer.
            if let Some(rs) = cc.wgpu_render_state.as_ref() {
                gs_field_egui::field_gpu::init(&cc.egui_ctx, rs);
            }
            // on as last ticked in the Preview tab, or for one run by the variable
            // (=0, false or off turns it off for the run, whatever was ticked)
            let v2 = std::env::var("GS_STUDIO_V2_LIGHT").ok().map(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "off" | ""));
            if v2.unwrap_or(gpu_light) {
                gs_field_egui::field_gpu_stage::set_enabled(&cc.egui_ctx, true);
            }
            Ok(Box::new(app::StudioApp::new(startup)))
        }),
    )
}
