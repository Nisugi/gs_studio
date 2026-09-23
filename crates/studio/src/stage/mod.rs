//! Preview-stage logic (VellumFE's Studio "Stage"). Like `editor/`, NOTHING
//! in this module tree may import egui or eframe
//! (`tests::stage_is_egui_free` enforces it): the sandbox roster and
//! simulator, mount pairing, scene naming, the sparse-save maths, the cast
//! scan and the condition evaluator all work -- and are tested -- without a
//! window. `tools/preview.rs` only draws and forwards input.
//!
//! Ported from VellumFE (GPL-3.0-or-later) `src/frontend/gui/studio.rs` at
//! commit 60c84a79.

pub mod cast;
pub mod color;
pub mod elanthian_time;
pub mod eval;
pub mod naming;
pub mod sparse;
pub mod state;

pub use naming::StudioRoomContext;
pub use state::{ClassFlag, StageState};

#[cfg(test)]
pub mod test_support {
    /// A temporary gs_field base dir for one test. Holds gs_field's
    /// process-wide lock for its whole life: the base dir is global state.
    pub struct TempBase {
        _dir: tempfile::TempDir,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl TempBase {
        pub fn new() -> TempBase {
            let guard = gs_field::data_dirs::VELLUM_FE_DIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let dir = tempfile::tempdir().unwrap();
            gs_field::data_dirs::set_base_dir(dir.path().to_path_buf());
            gs_field::pool::invalidate_cache();
            TempBase { _dir: dir, _guard: guard }
        }

        /// The temporary base dir (the pool root).
        pub fn path(&self) -> &std::path::Path {
            self._dir.path()
        }
    }

    impl Drop for TempBase {
        fn drop(&mut self) {
            gs_field::data_dirs::clear_base_dir();
            gs_field::pool::invalidate_cache();
        }
    }
}

#[cfg(test)]
mod tests {
    /// The egui layer only draws and forwards input; keep it that way.
    #[test]
    fn stage_is_egui_free() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/stage");
        let mut checked = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let text = std::fs::read_to_string(&path).unwrap();
            for (n, line) in text.lines().enumerate() {
                let code = line.split("//").next().unwrap();
                // Built from pieces so this file does not trip itself.
                for banned in [concat!("eg", "ui::"), concat!("efr", "ame::"), concat!("use eg", "ui")] {
                    assert!(!code.contains(banned), "{}:{}: {line}", path.display(), n + 1);
                }
            }
            checked += 1;
        }
        assert!(checked >= 6);
    }
}
