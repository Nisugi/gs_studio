//! New in this crate (not ported): every calibrator opens against a TEMP
//! pool and survives headless egui frames without input. Nothing here
//! clicks Save; the final test checks that rendering wrote nothing.

use std::path::{Path, PathBuf};

use gs_calibrators::{CreatureCalibrationState, FrameCalibrationState, SceneryCalibrationState};

struct TestEnv {
    _guard: std::sync::MutexGuard<'static, ()>,
    dir: tempfile::TempDir,
    ctx: egui::Context,
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        creature_field::data_dirs::clear_base_dir();
        creature_field::pool::invalidate_cache();
    }
}

fn test_env() -> TestEnv {
    let guard = creature_field::data_dirs::VELLUM_FE_DIR_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    creature_field::data_dirs::set_base_dir(dir.path().to_path_buf());
    creature_field::pool::invalidate_cache();
    TestEnv {
        _guard: guard,
        dir,
        ctx: egui::Context::default(),
    }
}

fn write_png(path: &Path, px: u32) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let pixels = vec![0xffu8; (px * px * 4) as usize];
    image::save_buffer(path, &pixels, px, px, image::ExtendedColorType::Rgba8).unwrap();
}

fn pool_dir() -> PathBuf {
    creature_field::pool_dirs::global_images_dir().unwrap()
}

/// Three frames: textures upload on the first, layout settles after.
fn frames(ctx: &egui::Context, mut ui: impl FnMut(&egui::Context)) {
    for _ in 0..3 {
        let _ = ctx.run_ui(egui::RawInput::default(), |root| ui(root.ctx()));
    }
}

/// The tab form: the body drawn into a central panel, no window.
fn tab_frames(ctx: &egui::Context, mut body: impl FnMut(&mut egui::Ui)) {
    for _ in 0..3 {
        let _ = ctx.run_ui(egui::RawInput::default(), |root| {
            egui::CentralPanel::default().show(root, |ui| body(ui));
        });
    }
}

/// Every file under `root` with its length, for a wrote-nothing check.
fn snapshot(root: &Path) -> Vec<(PathBuf, u64)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push((path.clone(), entry.metadata().unwrap().len()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn empty_pool_reports_a_message_and_no_state() {
    let _env = test_env();
    let (state, outcome) = CreatureCalibrationState::open();
    assert!(state.is_none() && !outcome.messages.is_empty());
    let (state, outcome) = SceneryCalibrationState::open();
    assert!(state.is_none() && !outcome.messages.is_empty());
    let (state, outcome) = FrameCalibrationState::open(None);
    assert!(state.is_none() && !outcome.messages.is_empty());
}

#[test]
fn every_calibrator_renders_as_window_and_as_tab_and_writes_nothing() {
    let env = test_env();
    write_png(&pool_dir().join("creatures/kobold/kobold.png"), 8);
    write_png(&pool_dir().join("scenery/boulder.png"), 8);
    write_png(&pool_dir().join("frames/iron.png"), 32);
    creature_field::pool::invalidate_cache();
    let before = snapshot(env.dir.path());

    let (state, _) = CreatureCalibrationState::open();
    let mut state = state.expect("creature calibrator opens");
    frames(&env.ctx, |ctx| assert!(!state.ui(ctx).closed));
    tab_frames(&env.ctx, |ui| assert!(!state.ui_contents(ui).closed));

    let (state, _) = SceneryCalibrationState::open();
    let mut state = state.expect("scenery calibrator opens");
    frames(&env.ctx, |ctx| assert!(!state.ui(ctx).closed));
    tab_frames(&env.ctx, |ui| assert!(!state.ui_contents(ui).closed));

    let (state, _) = FrameCalibrationState::open(Some("iron".to_owned()));
    let mut state = state.expect("frame calibrator opens");
    frames(&env.ctx, |ctx| assert!(!state.ui(ctx).closed));
    tab_frames(&env.ctx, |ui| assert!(!state.ui_contents(ui).closed));

    assert_eq!(before, snapshot(env.dir.path()), "rendering must not write");
}
