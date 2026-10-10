//! The injury doll's figures (ADDITION): the playable races, and a few
//! extras (a grid figure, odd ones), each a modelled body in the pool's
//! `global/bodies/<form>/` (`gs_base::data_dirs::bodies_dir`). The doll is not
//! the creature field: a creature's form (`humanoid`, `canine`...) is a
//! ring puppet the field draws, and is no figure here.
//!
//! A body's folder may hold a `doll.toml` saying how the doll shows it:
//!
//! ```toml
//! label = "Half-elf"   # what a picker says (default: the form's name)
//! race = "Half-elf"    # the playable race it is a figure of: a client's
//!                      # doll offers only these (`races`), by race
//! variant = "Male"     # which of the race's figures it is ("Female",
//!                      # "Male - grid"...; default: its label)
//! kind = "race"        # or "extra" (default: race)
//! draw = "splats"      # its trained splats (default), "mesh", or "made":
//!                      # splats made from the mesh and its sheet
//! order = 10           # earlier first, within its kind (default 100)
//! made_splats = 300000 # how many splats "made" makes (default: the
//!                      # renderer's own, 800k): every wound on the figure
//!                      # is rebuilt over them, so fewer is quicker
//! arms_hang = 17       # how far out its arms hang at rest, degrees from
//!                      # straight down (its upper arms' bones): a figure
//!                      # modelled with its arms raised has them lowered to
//!                      # where they clear its body, and a broad chest keeps
//!                      # them out; this sets it (`arms_hang`)
//! ```
//!
//! No egui here.

use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

/// What a figure is to the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FigureKind {
    /// A playable race: what a character is drawn as.
    Race,
    /// Something else worth wearing a character's wounds: a grid figure, an
    /// odd one.
    Extra,
}

/// How the doll draws a figure's body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FigureDraw {
    /// The splat cloud trained from pictures of it (its mesh where it has
    /// none).
    Splats,
    /// Its mesh, posed on the GPU.
    Mesh,
    /// Splats made from its mesh and its painted sheet: as sharp as the
    /// sheet (a grid's lines stay lines).
    Made,
}

impl FigureDraw {
    /// As `creature_field_egui::creature_puppet::PuppetKey::body_cloud` counts:
    /// 0 its mesh, 1 its trained splats, 2 splats made from the mesh.
    pub fn body_cloud(self) -> u8 {
        match self {
            FigureDraw::Mesh => 0,
            FigureDraw::Splats => 1,
            FigureDraw::Made => 2,
        }
    }
}

/// One figure the doll can be.
#[derive(Debug, Clone, PartialEq)]
pub struct DollFigure {
    /// The form: its puppet's family and its body's folder name.
    pub form: String,
    pub label: String,
    pub kind: FigureKind,
    pub draw: FigureDraw,
    pub order: i32,
    /// How many splats its made cloud aims at (None: the renderer's own).
    pub made_splats: Option<usize>,
    /// The playable race it is a figure of, and which of the race's
    /// figures (its `doll.toml`'s `race` and `variant`; None: it is no
    /// race's, and a client's doll does not offer it).
    pub race: Option<String>,
    pub variant: String,
}

/// A playable race and its figures ([`races`]).
#[derive(Debug, Clone, PartialEq)]
pub struct DollRace {
    pub name: String,
    pub figures: Vec<DollFigure>,
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct DollToml {
    label: Option<String>,
    kind: Option<String>,
    draw: Option<String>,
    order: Option<i32>,
    made_splats: Option<usize>,
    race: Option<String>,
    variant: Option<String>,
    arms_hang: Option<f32>,
}

/// How far out the arms of the figure whose body is in `dir` hang at rest,
/// degrees from straight down, when its `doll.toml` says (`arms_hang`):
/// the puppet's own clearing otherwise. The file beside the body, else
/// the one in the folder over it (a source's `<form>/master/` body, its
/// `doll.toml` in `<form>/`).
pub fn arms_hang(dir: &Path) -> Option<f32> {
    [Some(dir), dir.parent()].into_iter().flatten().find_map(|d| {
        let text = std::fs::read_to_string(d.join("doll.toml")).ok()?;
        toml::from_str::<DollToml>(&text).ok()?.arms_hang.filter(|deg| deg.is_finite())
    })
}

/// `half_elf` -> "Half elf".
fn label_of(form: &str) -> String {
    let words = form.replace('_', " ");
    let mut chars = words.chars();
    chars.next().map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}

/// The figure whose body is in `dir` (named `form`), from its `doll.toml`
/// where it has one. A file that does not parse is said on stderr and the
/// defaults stand.
pub fn read_figure(form: &str, dir: &Path) -> DollFigure {
    let file = dir.join("doll.toml");
    let toml: DollToml = match std::fs::read_to_string(&file) {
        Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
            eprintln!("{}: {e}", file.display());
            DollToml::default()
        }),
        Err(_) => DollToml::default(),
    };
    let kind = match toml.kind.as_deref().map(str::trim) {
        Some(k) if k.eq_ignore_ascii_case("extra") => FigureKind::Extra,
        _ => FigureKind::Race,
    };
    let draw = match toml.draw.as_deref().map(|d| d.trim().to_ascii_lowercase()) {
        Some(d) if d == "mesh" => FigureDraw::Mesh,
        Some(d) if d == "made" => FigureDraw::Made,
        _ => FigureDraw::Splats,
    };
    let label = toml.label.unwrap_or_else(|| label_of(form));
    let race = toml.race.map(|r| r.trim().to_owned()).filter(|r| !r.is_empty());
    let variant = toml.variant.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()).unwrap_or_else(|| label.clone());
    DollFigure { form: form.to_owned(), label, kind, draw, order: toml.order.unwrap_or(100), made_splats: toml.made_splats.filter(|n| *n > 0), race, variant }
}

/// Every figure in `bodies` (a `global/bodies` folder): each subfolder
/// holding a `body.obj`. Races first, then extras; within each by `order`,
/// then label.
pub fn scan(bodies: &Path) -> Vec<DollFigure> {
    let mut out: Vec<DollFigure> = std::fs::read_dir(bodies)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("body.obj").is_file())
        .map(|e| read_figure(&e.file_name().to_string_lossy(), &e.path()))
        .collect();
    out.sort_by(|a, b| (a.kind, a.order, &a.label).cmp(&(b.kind, b.order, &b.label)));
    out
}

type Kept = Mutex<Option<(u64, std::path::PathBuf, Arc<Vec<DollFigure>>)>>;

/// Bumped by [`invalidate`]: the roster is read again.
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Read the figures again at the next [`roster`]: a body was installed or
/// removed behind the doll's back (gs-studio's `--install-doll` calls it).
pub fn invalidate() {
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Which reading of the pool's figures is in force: another after
/// [`invalidate`] or when the base dir moves (`data_dirs::base_generation`).
pub fn generation() -> u64 {
    GENERATION.load(std::sync::atomic::Ordering::Relaxed).wrapping_add(gs_base::data_dirs::base_generation() << 32)
}

/// The doll's figures in the pool: [`scan`] of `global/bodies`, kept until
/// [`invalidate`] or the base dir moves (`data_dirs::base_generation`). It
/// reads the bodies folder alone -- not the creature set, which a client's
/// doll never loads.
pub fn roster() -> Arc<Vec<DollFigure>> {
    static KEPT: OnceLock<Kept> = OnceLock::new();
    let Ok(dir) = gs_base::data_dirs::bodies_dir() else { return Arc::default() };
    let generation = generation();
    let mut kept = KEPT.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some((g, d, list)) = kept.as_ref() {
        if *g == generation && *d == dir {
            return list.clone();
        }
    }
    let list = Arc::new(scan(&dir));
    *kept = Some((generation, dir, list.clone()));
    list
}

/// The playable races among `figures`: each figure that says its race,
/// grouped by it -- races by name, a race's figures by `order`, then
/// variant.
pub fn races_of(figures: &[DollFigure]) -> Vec<DollRace> {
    let mut out: Vec<DollRace> = Vec::new();
    for f in figures.iter().filter(|f| f.race.is_some()) {
        let name = f.race.clone().unwrap_or_default();
        match out.iter_mut().find(|r| r.name == name) {
            Some(r) => r.figures.push(f.clone()),
            None => out.push(DollRace { name, figures: vec![f.clone()] }),
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    for r in &mut out {
        r.figures.sort_by(|a, b| (a.order, &a.variant).cmp(&(b.order, &b.variant)));
    }
    out
}

/// The playable races in the pool ([`races_of`] the [`roster`]): what a
/// client's doll offers.
pub fn races() -> Vec<DollRace> {
    races_of(&roster())
}

/// `form`'s figure, when the doll has one by that name (a pool key's skin
/// and arms, `#...` and `@...`, are not the form's).
pub fn figure(form: &str) -> Option<DollFigure> {
    let form = form.split(['#', '@']).next().unwrap_or(form);
    roster().iter().find(|f| f.form == form).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_roster_is_the_bodies_races_first_as_their_files_say() {
        let root = std::env::temp_dir().join(format!("gs_field_roster_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (form, toml) in [
            ("half_elf", None),
            ("grid_figure", Some("label = \"Grid figure\"\nkind = \"extra\"\ndraw = \"made\"\nmade_splats = 300000\n")),
            ("aelotoi", Some("order = 5\n")),
            ("broken", Some("kind = [\n")),
        ] {
            let dir = root.join(form);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("body.obj"), "").unwrap();
            if let Some(t) = toml {
                std::fs::write(dir.join("doll.toml"), t).unwrap();
            }
        }
        std::fs::create_dir_all(root.join("no_body")).unwrap();
        let found = scan(&root);
        assert_eq!(found.iter().map(|f| f.form.as_str()).collect::<Vec<_>>(), ["aelotoi", "broken", "half_elf", "grid_figure"]);
        assert_eq!(found[2].label, "Half elf");
        assert_eq!((found[2].kind, found[2].draw), (FigureKind::Race, FigureDraw::Splats));
        let grid = &found[3];
        assert_eq!((grid.label.as_str(), grid.kind, grid.draw.body_cloud(), grid.made_splats), ("Grid figure", FigureKind::Extra, 2, Some(300000)));
        assert_eq!(found[2].made_splats, None);
        assert_eq!(found[1].kind, FigureKind::Race, "a file that does not parse: the defaults");
        assert!(races_of(&found).is_empty(), "none says its race");
        std::fs::remove_dir_all(&root).ok();
    }

    /// A client's doll offers only the figures that say their race,
    /// grouped by race, each race's in the order their files give.
    #[test]
    fn the_races_are_the_figures_that_say_theirs() {
        let root = std::env::temp_dir().join(format!("gs_field_races_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (form, toml) in [
            ("half_elf_male", "race = \"Half-elf\"
variant = \"Male\"
order = 3
"),
            ("half_elf_female", "race = \"Half-elf\"
variant = \"Female\"
order = 1
"),
            ("grid_figure", "label = \"Grid figure\"
kind = \"extra\"
race = \"Half-elf\"
variant = \"Male - grid\"
order = 4
"),
            ("half_elf_female_grid", "race = \"Half-elf\"
variant = \"Female - grid\"
order = 2
"),
            ("dwarf_male", ""),
            ("aelotoi_female", "race = \"Aelotoi\"
"),
        ] {
            let dir = root.join(form);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("body.obj"), "").unwrap();
            std::fs::write(dir.join("doll.toml"), toml).unwrap();
        }
        let races = races_of(&scan(&root));
        assert_eq!(races.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["Aelotoi", "Half-elf"], "a figure that says no race is none");
        let half_elf: Vec<(&str, &str)> = races[1].figures.iter().map(|f| (f.variant.as_str(), f.form.as_str())).collect();
        assert_eq!(half_elf, [("Female", "half_elf_female"), ("Female - grid", "half_elf_female_grid"), ("Male", "half_elf_male"), ("Male - grid", "grid_figure")]);
        assert_eq!(races[0].figures[0].variant, "Aelotoi female", "no variant: its label");
        std::fs::remove_dir_all(&root).ok();
    }
}
