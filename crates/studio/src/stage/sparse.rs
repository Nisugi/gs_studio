//! Sparse camera/solver embedding for scene saves, and the default-scene
//! layer. Ported from VellumFE `src/frontend/gui/studio.rs`
//! (`load_default_scene`, `sparse_camera`, `sparse_solver`) at 60c84a79.

use creature_field::scenes::{self, StageScene};
use creature_field::skins::{CreatureFieldCamera, CreatureFieldSolver};
use creature_field::solver::{FieldParams, SeparationBasis};

/// The "default" scene for layering under a room/location scene, unless
/// `excluding` IS the default (saving/loading the default itself).
pub fn load_default_scene(excluding: Option<&str>) -> Option<StageScene> {
    let names = scenes::list_scenes();
    let default = names.iter().find(|name| scenes::matchable(name) == "default")?;
    if excluding.is_some_and(|name| name == default.as_str()) {
        return None;
    }
    StageScene::load(default).ok()
}

/// Is this stem the default scene's?
pub fn is_default_scene(name: &str) -> bool {
    scenes::matchable(name) == "default"
}

/// The params a scene named `name` inherits from underneath: the default
/// scene's camera + solver over the built-ins, or just the built-ins for
/// the default scene itself.
pub fn inherited_params(name: &str) -> FieldParams {
    let mut base = FieldParams::default();
    if !is_default_scene(name) {
        if let Some(default_scene) = load_default_scene(Some(name)) {
            base.apply_camera(&default_scene.camera);
            base.apply_solver(&default_scene.solver);
        }
    }
    base
}

/// Camera keys that differ from `base` -- the sparse embed Save writes so
/// unpinned values inherit from the default scene live.
pub fn sparse_camera(params: &FieldParams, base: &FieldParams) -> CreatureFieldCamera {
    let diff = |value: f32, base: f32| (value != base).then_some(value);
    CreatureFieldCamera {
        focal: diff(params.focal, base.focal),
        eye_height: diff(params.cam_h, base.cam_h),
        near_depth: diff(params.z0, base.z0),
        row_depth: diff(params.dz, base.dz),
        horizon: diff(params.horizon, base.horizon),
        cell_width: diff(params.cell_w, base.cell_w),
    }
}

/// Solver keys that differ from `base`; same sparse discipline.
pub fn sparse_solver(params: &FieldParams, base: &FieldParams) -> CreatureFieldSolver {
    let s = &params.solver;
    let b = &base.solver;
    let diff = |value: f32, base: f32| (value != base).then_some(value);
    CreatureFieldSolver {
        zone: (s.zone_on != b.zone_on).then(|| if s.zone_on { "ellipse" } else { "grid" }.to_string()),
        zone_inset: diff(s.zone_inset, b.zone_inset),
        centre_pull: diff(s.centre_pull, b.centre_pull),
        depth_samples: (s.depth_samples != b.depth_samples).then_some(s.depth_samples),
        depth_jitter: diff(s.depth_jitter, b.depth_jitter),
        lateral_jitter: diff(s.lateral_jitter, b.lateral_jitter),
        depth_spread: diff(s.depth_spread, b.depth_spread),
        row_band_push: diff(s.row_band_push, b.row_band_push),
        row_band_px: diff(s.row_band_px, b.row_band_px),
        occlusion_cap: diff(s.occlusion_cap, b.occlusion_cap),
        variation: diff(s.variation, b.variation),
        seed_front: (s.seed_front != b.seed_front).then_some(s.seed_front),
        fall_reserve: diff(s.fall_reserve, b.fall_reserve),
        fall_reserve_hard: (s.fall_reserve_hard != b.fall_reserve_hard).then_some(s.fall_reserve_hard),
        separation_basis: (s.separation_basis != b.separation_basis).then(|| {
            match s.separation_basis {
                SeparationBasis::Contact => "contact",
                SeparationBasis::Card => "card",
            }
            .to_string()
        }),
        relax_steps: (s.relax_steps != b.relax_steps).then_some(s.relax_steps),
        shuffle_ties: (s.shuffle_ties != b.shuffle_ties).then_some(s.shuffle_ties),
        grow: (s.grow != b.grow).then(|| creature_field::solver::growth_name(s.grow).to_string()),
        max_depth: diff(s.max_depth, b.max_depth),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::test_support::TempBase;
    use creature_field::cards::resolve_field_params;
    use creature_field::field_config::FieldOverrides;

    // Ported as written; the lint is about the original's style.
    #[allow(clippy::field_reassign_with_default)]
    #[test]
    fn camera_saves_reproduce_valid_effective_params() {
        let _base = TempBase::new();

        // Standalone default scene: an out-of-range transient (focal=20)
        // sanitizes to the loader's bound BEFORE serialization, so the
        // save reproduces exactly what previewed.
        let mut params = FieldParams::default();
        params.focal = 20.0;
        params.horizon = 160.0;
        assert!(params.sanitize_camera(&FieldParams::default()));
        assert_eq!(params.focal, 60.0, "previewed value = loader's clamp");
        let mut default_scene = StageScene::default();
        default_scene.camera = sparse_camera(&params, &FieldParams::default());
        default_scene.save("default").unwrap();
        let loaded_default = StageScene::load("default").unwrap();
        let resolved = resolve_field_params(None, Some(&loaded_default), None, &FieldOverrides::default());
        assert_eq!(resolved.focal, params.focal);
        assert_eq!(resolved.horizon, 160.0);

        // Room scene over the default-scene layer: nonfinite transient
        // falls back to the inherited value; sparse save pins only what
        // differs, and the layered reload reproduces the effective params.
        let mut base = FieldParams::default();
        base.apply_camera(&loaded_default.camera);
        let mut room_params = base.clone();
        room_params.cam_h = f32::NAN; // invalid transient edit
        assert!(room_params.sanitize_camera(&base));
        assert_eq!(room_params.cam_h, base.cam_h, "NaN never reaches params");
        room_params.focal = 300.0;
        let mut room = StageScene::default();
        room.camera = sparse_camera(&room_params, &base);
        assert!(room.camera.horizon.is_none(), "inherited keys stay sparse");
        room.save("Barley_Field").unwrap();
        let loaded_room = StageScene::load("Barley_Field").unwrap();
        let resolved =
            resolve_field_params(Some(&loaded_default), Some(&loaded_room), None, &FieldOverrides::default());
        assert_eq!(resolved.focal, 300.0);
        assert_eq!(resolved.horizon, 160.0, "unpinned key inherits live");
        assert_eq!(resolved.cam_h, base.cam_h);

        // inherited_params is that same layer, and nothing for the default.
        assert_eq!(inherited_params("Barley_Field").horizon, 160.0);
        assert_eq!(inherited_params("default"), FieldParams::default());
    }

    #[test]
    fn sparse_solver_pins_only_what_differs_and_round_trips() {
        let base = FieldParams::default();
        assert!(sparse_solver(&base, &base).is_empty());
        assert!(sparse_camera(&base, &base).is_empty());

        let mut params = base.clone();
        params.solver.zone_on = !base.solver.zone_on;
        params.solver.centre_pull = base.solver.centre_pull + 0.5;
        params.solver.relax_steps = base.solver.relax_steps + 1;
        params.solver.separation_basis = match base.solver.separation_basis {
            SeparationBasis::Contact => SeparationBasis::Card,
            SeparationBasis::Card => SeparationBasis::Contact,
        };
        let sparse = sparse_solver(&params, &base);
        assert!(sparse.zone.is_some() && sparse.centre_pull.is_some());
        assert!(sparse.relax_steps.is_some() && sparse.separation_basis.is_some());
        assert!(sparse.zone_inset.is_none() && sparse.variation.is_none() && sparse.shuffle_ties.is_none());

        let mut rebuilt = base.clone();
        rebuilt.apply_solver(&sparse);
        assert_eq!(rebuilt, params);
    }
}
