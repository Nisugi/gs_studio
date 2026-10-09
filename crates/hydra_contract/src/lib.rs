//! Hydra's calls into gs_studio's runtime crates, as Hydra makes them.
//!
//! Hydra (`G:\dev\hydra`) takes gs_studio's crates from GitHub, pinned in
//! its `Cargo.lock`, and draws the injury doll's Infinite style through
//! them: `crates/hydra-gui/src/widget/infinite.rs` (the adapter) and
//! `play/options/doll.rs` (the settings page's statuses and time of day).
//! This crate is that code with Hydra's own types (its game state, its
//! widget look) replaced by plain stand-ins, and Hydra's tests of it --
//! so a change here that would break Hydra's build, or what its doll
//! shows, fails gs_studio's tests first.
//!
//! Change it only together with Hydra: when Hydra's adapter changes, copy
//! the change here; when a gs_studio path moves, Hydra moves with it
//! (`docs/crate-split-plan.md`).

use std::sync::Arc;

use gs_field::doll_roster::DollRace;
use gs_field::doll_sky::{DollSky, display, doll_sky, game_doll_sky_at, hex_linear};
use gs_field::doll_state::{DollState, Status};
use gs_field::lighting::Sky;
use gs_field_egui::creature_puppet::PuppetCache;
use gs_field_egui::{doll_view, field_gpu};

/// The playable races gs_studio installed in Hydra's data folder.
pub fn races() -> Vec<DollRace> {
    gs_field::doll_roster::races()
}

/// The figure (its form) drawn for `chosen`: that one where it is one of
/// `races`', else the first race's first; none where there are none.
pub fn figure(races: &[DollRace], chosen: Option<&str>) -> Option<String> {
    let mut all = races.iter().flat_map(|race| &race.figures);
    let first = all.clone().next();
    chosen.and_then(|chosen| all.find(|one| one.form == chosen)).or(first).map(|one| one.form.clone())
}

/// The way the doll's sky is seen: south.
pub const SOUTH: f32 = 180.0;

/// What stands behind the doll (Hydra's `Backdrop`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Backdrop {
    #[default]
    Day,
    Display,
    Black,
    Colour,
}

/// The player's choices for the doll's light (the fields of Hydra's
/// `DollLook` that its `sky` reads).
#[derive(Clone, Debug, Default)]
pub struct Look {
    pub backdrop: Backdrop,
    pub colour: Option<String>,
    pub light_from: Option<i16>,
    pub light_height: Option<i16>,
    pub light_strength: Option<u16>,
    /// Minutes from midnight.
    pub hour: Option<u16>,
}

/// The light and background the doll stands in (Hydra's `sky`, line for
/// line).
pub fn sky(look: Option<&Look>, now: Option<u32>) -> DollSky {
    let look = look.cloned().unwrap_or_default();
    let solid = |rgb: [f32; 3]| DollSky { sky_top: rgb.map(|c| c / 0.55), sky_horizon: rgb.map(|c| c / 0.55), ground: rgb.map(|c| c / 0.7), ..display() };
    let mut sky = match look.backdrop {
        Backdrop::Day => {
            let hour = look.hour.map(|minutes| f32::from(minutes) / 60.0);
            match now {
                Some(now) => game_doll_sky_at(f64::from(now), hour, SOUTH),
                None => doll_sky(hour, &Sky::default(), true),
            }
        }
        Backdrop::Display => display(),
        Backdrop::Black => solid([0.0; 3]),
        Backdrop::Colour => solid(hex_linear(look.colour.as_deref().unwrap_or("#000000"))),
    };
    if look.backdrop != Backdrop::Day
        && let Some(key) = sky.lights.first_mut()
    {
        if let Some(from) = look.light_from {
            key.azimuth = f32::from(from);
        }
        if let Some(height) = look.light_height {
            key.elevation = f32::from(height);
        }
    }
    if let Some(strength) = look.light_strength {
        let k = f32::from(strength) / 100.0;
        for light in &mut sky.lights {
            light.power *= k;
        }
    }
    sky
}

/// The display's key light: direction and height (Hydra's `key_light`).
pub fn key_light() -> (f32, f32) {
    display().lights.first().map_or((0.0, 45.0), |key| (key.azimuth, key.elevation))
}

/// What the character is, as gs_studio reads it (Hydra's `doll_state`,
/// taking the words its game state gives).
pub fn doll_state<'a>(wounds: impl IntoIterator<Item = (&'a str, u8)>, scars: impl IntoIterator<Item = (&'a str, u8)>, statuses: impl IntoIterator<Item = &'a str>) -> DollState {
    DollState::from_game(wounds, scars, statuses)
}

/// The game's hour at `now` (unix seconds): the settings page's time of
/// day before the player moves it.
pub fn hour_now(now: u32) -> f32 {
    gs_field::elanthian_time::elanthian_hour_f(f64::from(now))
}

/// The statuses Hydra's settings page offers as buttons, by group (its
/// `STATUS_GROUPS`).
pub const STATUS_GROUPS: [(&str, &[Status]); 5] = {
    use Status as S;
    [
        ("Pose", &[S::Kneeling, S::Sitting, S::Prone, S::Stunned, S::Sleeping, S::Unconscious, S::Webbed, S::Dead]),
        (
            "Body",
            &[S::Frozen, S::Immobile, S::Poisoned, S::Diseased, S::Bleeding, S::Rooted, S::Bound, S::Hidden, S::Invisible, S::Possessed, S::Blinded, S::Cutthroat, S::Silenced],
        ),
        ("Bearing", &[S::Calmed, S::Terrified, S::Demoralized, S::Feeble, S::Crippled, S::Staggered, S::Dazed, S::Confused]),
        ("Breath", &[S::Frenzied, S::Overexerted, S::Slowed]),
        ("Other", &[S::Vulnerable]),
    ]
};

/// Set up as Hydra's window opens: gs_studio's files in Hydra's data
/// folder, and the GPU pass (Hydra's `open`).
pub fn open(ctx: &egui::Context, render: Option<&egui_wgpu::RenderState>, data: &std::path::Path) {
    gs_field::data_dirs::set_base_dir(data.to_path_buf());
    if let Some(render) = render {
        field_gpu::init(ctx, render);
    }
}

/// Draw the doll for `doll` in `rect` as Hydra's widget does: `false` when
/// it could not be, and Hydra falls back to its own doll.
pub fn paint(ui: &egui::Ui, rect: egui::Rect, form: &str, doll: &DollState, facing: i16, sky: &DollSky, id: egui::Id) -> bool {
    let context = ui.ctx().clone();
    let form: Arc<str> = Arc::from(form);
    let shared = PuppetCache::shared(&context);
    let mut cache = shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let name = format!("doll:{}", id.value());
    let key = doll_view::key_for(doll).with_facing(f32::from(facing));
    doll_view::paint_in_view(ui, rect, &mut cache, &form, key, sky, Some(&name), field_gpu::available(&context), id.value())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gs_field::doll_roster::{DollFigure, FigureDraw, FigureKind};

    /// The doll stands in the day, the display, or the display's light
    /// over black or the colour picked (Hydra's test).
    #[test]
    fn the_doll_stands_before_the_chosen_backdrop() {
        let with = |backdrop, colour: Option<&str>| Look { backdrop, colour: colour.map(str::to_owned), ..Look::default() };
        assert!(!sky(None, None).indoor, "a day outdoors");
        assert!(sky(Some(&with(Backdrop::Display, None)), None).indoor);
        let black = sky(Some(&with(Backdrop::Black, None)), None);
        let dark = |c: [f32; 3]| c.iter().all(|c| c.abs() < f32::EPSILON);
        assert!(dark(black.sky_top) && dark(black.ground), "{black:?}");
        assert!(!black.lights.is_empty(), "lit all the same");
        let red = sky(Some(&with(Backdrop::Colour, Some("#ff0000"))), None);
        assert!(red.sky_top[0] > 1.0 && red.sky_top[1] == 0.0, "{red:?}");
        assert!(dark(sky(Some(&with(Backdrop::Colour, None)), None).sky_top), "black with no colour");
    }

    /// The game's sky once its clock is known; the player's light turned,
    /// raised and dimmed over the display, only dimmed under the sky; the
    /// game's own hour or the player's (Hydra's test, with the hour).
    #[test]
    fn the_players_light_moves_over_the_backdrop() {
        let noonish = 1_790_000_000;
        let day = sky(None, Some(noonish));
        assert!(!day.indoor && day.floor[0] > 0.0);
        let own = Look { backdrop: Backdrop::Display, light_from: Some(90), light_height: Some(10), light_strength: Some(50), ..Look::default() };
        let lit = sky(Some(&own), None);
        let key = lit.lights.first().expect("a key light");
        assert!((key.azimuth - 90.0).abs() < 1e-3 && (key.elevation - 10.0).abs() < 1e-3);
        let plain = sky(Some(&Look { backdrop: Backdrop::Display, ..Look::default() }), None);
        assert!((key.power - plain.lights[0].power * 0.5).abs() < 1e-4, "half as strong");
        let under = Look { backdrop: Backdrop::Day, ..own };
        let sun = sky(Some(&under), Some(noonish));
        for (a, b) in sun.lights.iter().zip(&day.lights) {
            assert!((a.azimuth - b.azimuth).abs() < 1e-3, "the sun stays where it is");
            assert!((a.power - b.power * 0.5).abs() < 1e-4, "but dims");
        }
        let hour = hour_now(noonish);
        assert!((0.0..24.0).contains(&hour));
        let at_midnight = sky(Some(&Look { hour: Some(0), ..Look::default() }), Some(noonish));
        assert!(at_midnight.lights.iter().map(|l| l.power).sum::<f32>() >= 0.0);
        assert!(key_light().1 > 0.0);
    }

    /// The doll is the figure the player chose, the first when none was or
    /// it is gone, nothing when there are none (Hydra's test: its
    /// `DollFigure` literal is Hydra's, so a field added here breaks it).
    #[test]
    fn the_doll_is_the_chosen_figure() {
        let one = |form: &str, variant: &str| DollFigure {
            form: form.to_owned(),
            label: variant.to_owned(),
            kind: FigureKind::Race,
            draw: FigureDraw::Splats,
            order: 100,
            made_splats: None,
            race: Some("Half-elf".to_owned()),
            variant: variant.to_owned(),
        };
        let races = [DollRace { name: "Half-elf".to_owned(), figures: vec![one("half_elf_male", "Male"), one("grid_figure", "Male - grid")] }];
        assert_eq!(figure(&races, Some("grid_figure")).as_deref(), Some("grid_figure"));
        assert_eq!(figure(&races, None).as_deref(), Some("half_elf_male"));
        assert_eq!(figure(&races, Some("humanoid")).as_deref(), Some("half_elf_male"));
        assert_eq!(figure(&[], Some("half_elf_male")), None);
    }

    /// The character's wounds, scars and statuses as gs_studio reads them
    /// (Hydra's test, from the words its state gives: its foot is on its
    /// leg; a status shown from the page joins the character's own).
    #[test]
    fn the_character_is_told_to_gs_studio_in_its_words() {
        let told = doll_state([("leftFoot", 2)], [("chest", 1)], ["stunned"]);
        assert!(told.wounds.iter().any(|(part, rank)| part == "leftLeg" && *rank == 2), "{told:?}");
        assert!(told.scars.iter().any(|(part, rank)| part == "chest" && *rank == 1), "{told:?}");
        assert!(told.statuses.has(Status::Stunned), "{told:?}");
        assert!(!told.statuses.has(Status::Prone), "not on: {told:?}");
        let shown = doll_state([], [], ["stunned", "frozen"]);
        assert!(shown.statuses.has(Status::Frozen) && shown.statuses.has(Status::Stunned));
        // every status the page offers has the id its buttons are keyed by
        for (_, statuses) in STATUS_GROUPS {
            for status in statuses {
                assert!(!status.id().is_empty());
                assert!(doll_state([], [], [status.id()]).statuses.has(*status), "{} reads back", status.id());
            }
        }
    }

    /// Hydra's data folder as gs_studio reads it: the races its installed
    /// figures name, in their order; and with no figure for the form asked,
    /// the doll is not drawn (Hydra falls back to its own), without a panic.
    /// One test: the data folder is the process's.
    #[test]
    fn the_installed_races_and_a_doll_with_nothing_to_draw() {
        let data = tempfile::tempdir().expect("a data folder");
        let bodies = data.path().join("global/bodies");
        for (form, variant, order) in [("half_elf_male", "Male", 3), ("half_elf_female", "Female", 1), ("grid_figure", "Male - grid", 4)] {
            let dir = bodies.join(form);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("body.obj"), "v 0 0 0\n").unwrap();
            std::fs::write(dir.join("doll.toml"), format!("race = \"Half-elf\"\nvariant = \"{variant}\"\norder = {order}\n")).unwrap();
        }
        let ctx = egui::Context::default();
        open(&ctx, None, data.path());
        let races = races();
        assert_eq!(races.len(), 1, "{races:?}");
        let order: Vec<&str> = races[0].figures.iter().map(|f| f.variant.as_str()).collect();
        assert_eq!(order, ["Female", "Male", "Male - grid"]);
        let mut drawn = None;
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 300.0));
            drawn = Some(paint(ui, rect, "no_such_form", &doll_state([], [], []), 0, &sky(None, None), egui::Id::new("doll")));
        });
        assert_eq!(drawn, Some(false), "nothing to draw: Hydra falls back");
    }
}
