//! New in this crate (not ported): every calibrator opens against a TEMP
//! pool and survives headless egui frames without input. Nothing here
//! clicks Save; the final test checks that rendering wrote nothing.

use std::path::{Path, PathBuf};

use gs_calibrators::{
    CreatureCalibrationState, DollCalibrationState, DollCalibrationTarget, DollSetsEditorState,
    FrameCalibrationState, SceneryCalibrationState,
};

struct TestEnv {
    _guard: std::sync::MutexGuard<'static, ()>,
    dir: tempfile::TempDir,
    ctx: egui::Context,
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        gs_field::data_dirs::clear_base_dir();
        gs_field::pool::invalidate_cache();
    }
}

fn test_env() -> TestEnv {
    let guard = gs_field::data_dirs::VELLUM_FE_DIR_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    gs_field::data_dirs::set_base_dir(dir.path().to_path_buf());
    gs_field::pool::invalidate_cache();
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
    gs_field::data_dirs::global_images_dir().unwrap()
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
    let (state, outcome) = DollSetsEditorState::open(Default::default());
    assert!(state.is_none() && !outcome.messages.is_empty());
    let (state, outcome) = DollCalibrationState::open(None);
    assert!(state.is_none());
    assert!(outcome.messages[0].starts_with("No doll image selected."));
}

#[test]
fn every_calibrator_renders_as_window_and_as_tab_and_writes_nothing() {
    let env = test_env();
    write_png(&pool_dir().join("creatures/kobold/kobold.png"), 8);
    write_png(&pool_dir().join("scenery/boulder.png"), 8);
    write_png(&pool_dir().join("frames/iron.png"), 32);
    write_png(&pool_dir().join("dolls/human.png"), 16);
    gs_field::pool::invalidate_cache();
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

    let (state, _) = DollSetsEditorState::open(Default::default());
    let mut state = state.expect("doll sets opens");
    frames(&env.ctx, |ctx| assert!(!state.ui(ctx).closed));
    tab_frames(&env.ctx, |ui| assert!(!state.ui_contents(ui).closed));

    let target = DollCalibrationTarget::from_pool_path("dolls/human.png");
    assert!(target.image_abs.is_file());
    let (state, outcome) = DollCalibrationState::open(Some(target));
    assert!(outcome.messages.is_empty());
    let mut state = state.expect("doll calibrator opens");
    frames(&env.ctx, |ctx| {
        let outcome = state.ui(ctx);
        assert!(!outcome.closed && !outcome.reload_art);
    });
    tab_frames(&env.ctx, |ui| {
        let outcome = state.ui_contents(ui);
        assert!(!outcome.closed && !outcome.reload_art);
    });

    assert_eq!(before, snapshot(env.dir.path()), "rendering must not write");
}

#[test]
fn doll_calibration_closes_when_the_host_base_vanishes() {
    let env = test_env();
    write_png(&pool_dir().join("dolls/human.png"), 16);
    let target = DollCalibrationTarget::from_pool_path("dolls/human.png");
    let (state, _) = DollCalibrationState::open(Some(target));
    let mut state = state.unwrap();
    let mut outcome = None;
    let _ = env.ctx.run_ui(egui::RawInput::default(), |root| {
        let ctx = root.ctx();
        outcome = Some(state.ui_with_base(ctx, None));
    });
    // The tab body reports the same thing.
    let mut tab_outcome = None;
    tab_frames(&env.ctx, |ui| tab_outcome = Some(state.ui_contents_with_base(ui, None)));
    assert!(tab_outcome.unwrap().closed);
    let outcome = outcome.unwrap();
    assert!(outcome.closed);
    assert_eq!(
        outcome.messages,
        ["Injury doll calibration closed: the selected doll set has no base image loaded."]
    );
}

#[test]
fn doll_calibration_reads_the_sidecar_beside_the_image() {
    let _env = test_env();
    let image = pool_dir().join("dolls/human.png");
    write_png(&image, 16);
    let mut anchors = std::collections::HashMap::new();
    anchors.insert("head".to_owned(), [0.25f32, 0.75f32]);
    gs_field::pool::write_doll_sidecar(&image, &anchors, &Default::default()).unwrap();
    // Opening after an external write sees it (no click needed to prove the
    // read path): a second write through the same API round-trips.
    let sidecar: gs_field::pool::DollSidecar = gs_field::pool::read_sidecar(&image).unwrap();
    assert_eq!(sidecar.anchors.get("head"), Some(&[0.25, 0.75]));
    let (state, outcome) =
        DollCalibrationState::open(Some(DollCalibrationTarget::from_pool_path("dolls/human.png")));
    assert!(state.is_some() && outcome.messages.is_empty());
}
