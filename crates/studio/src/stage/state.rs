//! The Stage sandbox: a fabricated room roster, target state and scene
//! feeding the production creature-field pipeline (`sync_field` + the
//! renderer). Ported from VellumFE `src/frontend/gui/studio.rs`
//! (`StageState`) at commit 60c84a79.
//!
//! VellumFE held a whole `GameState` because its `sync_field` took one;
//! gs_field's takes the roster slice and its generation, so they are held
//! directly here with the current target. The simulator's edits, which the
//! original made straight from its checkbox closures, are methods here so
//! they can be tested without a window.

use std::sync::Arc;

use gs_field::scenes::{SceneProp, StageScene};
use gs_field::solver::{CreatureField, FieldParams, STAGE_W};
use gs_field::{Creature, CreatureFlags};

use super::cast::{self, build_cast, CastEntry};
use super::naming::{NewSceneState, StudioRoomContext};
use super::sparse;

#[cfg(test)]
use crate::status_log::STATUS_CAP;
use crate::status_log::StatusLog;

/// The class checkboxes of the simulator, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassFlag {
    Dead,
    Boss,
    MiniBoss,
    Sympathetic,
    Rider,
    Mount,
}

impl ClassFlag {
    pub const ALL: [ClassFlag; 6] =
        [Self::Dead, Self::Boss, Self::MiniBoss, Self::Sympathetic, Self::Rider, Self::Mount];

    pub fn label(self) -> &'static str {
        match self {
            Self::Dead => "dead",
            Self::Boss => "boss",
            Self::MiniBoss => "mini boss",
            Self::Sympathetic => "sympathetic",
            Self::Rider => "rider",
            Self::Mount => "mount",
        }
    }

    /// dead flips field membership rules; boss flips card size; rider/
    /// mount pair units: all structural, so the field re-syncs.
    /// sympathetic is paint only.
    pub fn structural(self) -> bool {
        self != Self::Sympathetic
    }

    pub fn get(self, flags: &CreatureFlags) -> bool {
        match self {
            Self::Dead => flags.dead,
            Self::Boss => flags.ascension_boss,
            Self::MiniBoss => flags.mini_boss,
            Self::Sympathetic => flags.sympathetic,
            Self::Rider => flags.rider,
            Self::Mount => flags.mount,
        }
    }

    fn slot(self, flags: &mut CreatureFlags) -> &mut bool {
        match self {
            Self::Dead => &mut flags.dead,
            Self::Boss => &mut flags.ascension_boss,
            Self::MiniBoss => &mut flags.mini_boss,
            Self::Sympathetic => &mut flags.sympathetic,
            Self::Rider => &mut flags.rider,
            Self::Mount => &mut flags.mount,
        }
    }
}

pub struct StageState {
    /// The solved field, synced from the roster.
    pub field: CreatureField,
    /// Roster generation / calibration revision `field` was last synced
    /// against (`sync_field`'s bookkeeping).
    field_synced_gen: u64,
    field_synced_cal: u64,
    /// The fabricated room: roster, its generation, the current target.
    creatures: Vec<Creature>,
    creatures_generation: u64,
    current_target: String,
    pub cast: Vec<CastEntry>,
    pub filter: String,
    /// "Spawn by name": what is typed in the bestiary box. The Cast list is
    /// built from the ART POOL, so everything in it has art by definition
    /// and sprite art always beats the family puppet -- this box is the only
    /// way to put a creature the pool has NO art for on the stage, which is
    /// the only way to see the canine / elephantid / suine puppets without
    /// emptying the pool.
    pub spawn_name: String,
    /// What the spawn box shows for `spawn_name`, computed once per name.
    pub spawn_hints: crate::fs_cache::Ttl<String, cast::SpawnHints>,
    next_id: u64,
    pub selected: Option<String>,
    /// Creatures drawn as their family's puppet even when sprite art
    /// resolves (the Puppets list, the simulator's "Draw as puppet").
    forced_puppets: std::collections::BTreeSet<String>,
    /// Puppet facings the simulator set (degrees; absent = the default turn).
    facings: std::collections::BTreeMap<String, f32>,
    pub show_grid: bool,
    pub show_order: bool,
    /// Currently applied rider/mount pair (first flagged rider + mount).
    pair: Option<(String, String)>,
    /// The Stage's scene, held as an Arc; edits go through `Arc::make_mut`
    /// so pointer identity is the "scene changed" bit (see `obstacle_scene`).
    scene: Arc<StageScene>,
    /// Save target / last loaded scene stem.
    pub scene_name: String,
    /// Load-list filter box.
    pub scene_filter: String,
    /// Solver tunables are testing-only UI; hidden unless asked for.
    pub show_solver_tuning: bool,
    /// Scenery-pool filter box.
    pub prop_filter: String,
    /// Lighting preview: the game hour shown (None = the live Elanthian
    /// clock) and the Lighting strength (the player's setting, 0..1).
    pub light_hour: Option<f32>,
    pub light_strength: f32,
    /// Weather preview: 0 = clear, else 1 + the index into the renderer's
    /// weather kinds; how much falls (0..1); wind (-1..1).
    pub weather_kind: usize,
    pub weather_intensity: f32,
    pub weather_wind: f32,
    /// Tint the backdrop's occluders on the stage (checking the mask).
    pub show_occluders: bool,
    /// Puppets hold the weapon their bestiary entry attacks with.
    pub show_props: bool,
    /// Living puppets breathe (their body type's `[idle]` cycle).
    pub show_breath: bool,
    /// Puppets drawn as Gaussian splat clouds instead of the smooth skin.
    pub show_splats: bool,
    /// Selected placed prop (index into scene.props); drag-to-place moves it.
    pub selected_prop: Option<usize>,
    /// Selected point light (index into scene.lights); drag moves it.
    pub selected_light: Option<usize>,
    /// Creature exist id being dragged on the stage (grabbed by its card).
    dragging_creature: Option<String>,
    /// Scene Arc the solver obstacles were last built from (`make_mut`
    /// re-points the Arc on every edit, so pointer identity is the dirty
    /// bit -- sidecars are only re-read when the scene actually changed).
    obstacle_scene: Option<Arc<StageScene>>,
    /// Status lines, newest last, capped (`status_log::STATUS_CAP`; the log type
    /// every tool shares).
    status: StatusLog,
    /// New-scene naming inputs (manual binding + name, room prefill).
    pub new_scene: NewSceneState,
}

impl StageState {
    /// Scans the creature pool for the cast.
    pub fn new(context: StudioRoomContext) -> Self {
        Self::with_cast(context, build_cast())
    }

    pub fn with_cast(context: StudioRoomContext, cast: Vec<CastEntry>) -> Self {
        let mut stage = Self {
            field: CreatureField::default(),
            field_synced_gen: 0,
            field_synced_cal: 0,
            creatures: Vec::new(),
            creatures_generation: 0,
            current_target: String::new(),
            cast,
            filter: String::new(),
            spawn_name: String::new(),
            spawn_hints: Default::default(),
            next_id: 0,
            selected: None,
            forced_puppets: Default::default(),
            facings: Default::default(),
            show_grid: true,
            show_order: false,
            pair: None,
            scene: Arc::new(StageScene::default()),
            scene_name: String::new(),
            scene_filter: String::new(),
            show_solver_tuning: false,
            prop_filter: String::new(),
            light_hour: None,
            light_strength: 1.0,
            weather_kind: 0,
            weather_intensity: 0.6,
            weather_wind: 0.15,
            show_occluders: false,
            show_props: true,
            show_breath: true,
            show_splats: false,
            selected_prop: None,
            selected_light: None,
            dragging_creature: None,
            obstacle_scene: None,
            status: StatusLog::default(),
            new_scene: NewSceneState::new(context),
        };
        stage.push_status(format!("Stage ready: {} castable bases", stage.cast.len()));
        stage
    }

    // ------------------------------------------------------------ status

    pub fn push_status(&mut self, message: String) {
        self.status.push(message);
    }

    pub fn status_lines(&self) -> &[String] {
        self.status.lines()
    }

    // ------------------------------------------------------------ roster

    pub fn creatures(&self) -> &[Creature] {
        &self.creatures
    }

    pub fn creature(&self, id: &str) -> Option<&Creature> {
        self.creatures.iter().find(|c| c.id == id)
    }

    pub fn current_target(&self) -> &str {
        &self.current_target
    }

    /// Mark the roster dirty. The actual sync runs in the tool's frame
    /// pass AFTER art prep (finding 8: a spawned creature's standing and
    /// prone calibration must reach the geometry store before the solver
    /// commits its placement), so a spawn lands one frame later with its
    /// true envelope instead of immediately with a fallback guess.
    pub fn resync(&mut self) {
        self.creatures_generation += 1;
    }

    /// `gs_field::cards::sync_field` against the roster.
    pub fn sync_field(&mut self) {
        gs_field::cards::sync_field(
            &mut self.field,
            &mut self.field_synced_gen,
            &mut self.field_synced_cal,
            &self.creatures,
            self.creatures_generation,
            &[],
        );
    }

    pub fn spawn(&mut self, display: &str, noun: Option<String>) {
        self.next_id += 1;
        let id = format!("studio-{}", self.next_id);
        self.creatures.push(Creature {
            name: display.to_string(),
            noun,
            id: id.clone(),
            status: None,
            flags: Some(CreatureFlags { hostile: true, ..Default::default() }),
        });
        self.selected = Some(id);
        self.resync();
    }

    /// Spawn by NAME, whether or not the pool has art for it: the noun is
    /// the name's last word unless one is given. Pushes a status line
    /// saying which it got -- sprite art, or the family puppet -- because
    /// the difference is invisible until the card draws.
    ///
    /// Returns false (and spawns nothing) for a blank name.
    pub fn spawn_named(&mut self, name: &str, noun: Option<String>) -> bool {
        let name = name.trim();
        if name.is_empty() {
            return false;
        }
        let noun = noun.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| cast::last_word(name));
        let noun = (!noun.is_empty()).then_some(noun);
        let art = cast::has_art(name, noun.as_deref());
        let note = if art {
            "sprite art from the pool".to_string()
        } else {
            match cast::puppet_family(name, noun.as_deref()) {
                Some(family) => format!("NO art in the pool -- drawing as the {family} puppet"),
                None => "NO art in the pool and no puppet fits -- drawing as a placeholder".to_string(),
            }
        };
        self.spawn(name, noun);
        self.push_status(format!("Spawned '{name}': {note}"));
        true
    }

    /// Put `family`'s puppet on the stage: spawn a bestiary creature of that
    /// family (see [`cast::puppet_showcase`]) and draw it as its puppet
    /// whatever art the pool holds. Returns false, with a status line saying
    /// why, when no creature resolves to that puppet.
    pub fn spawn_puppet(&mut self, family: &str) -> bool {
        match cast::puppet_showcase(family) {
            Ok(pick) => {
                self.spawn(&pick.name, Some(pick.noun));
                let id = self.selected.clone().expect("spawn selects");
                self.forced_puppets.insert(id);
                self.push_status(format!("Spawned '{}': drawing as the {family} puppet", pick.name));
                true
            }
            Err(why) => {
                self.push_status(format!("Cannot show the {family} puppet: {why}"));
                false
            }
        }
    }

    /// Put `form`'s puppet on the stage wearing one of its textures: the
    /// creature whose skin it is (`creature` = name and noun, forced to
    /// draw as its puppet, so the cache picks its skin), or with None the
    /// bare sheet via [`spawn_puppet`](Self::spawn_puppet).
    pub fn spawn_puppet_wearing(&mut self, form: &str, creature: Option<(&str, &str)>) -> bool {
        let Some((name, noun)) = creature else { return self.spawn_puppet(form) };
        self.spawn(name, Some(noun.to_string()));
        let id = self.selected.clone().expect("spawn selects");
        self.forced_puppets.insert(id);
        self.push_status(format!("Spawned '{name}': drawing as the {form} puppet in its own skin"));
        true
    }

    /// Is this creature drawn as its puppet regardless of art?
    pub fn is_forced_puppet(&self, id: &str) -> bool {
        self.forced_puppets.contains(id)
    }

    pub fn set_forced_puppet(&mut self, id: &str, on: bool) {
        if on {
            self.forced_puppets.insert(id.to_string());
        } else {
            self.forced_puppets.remove(id);
        }
    }

    /// The simulator's facing for this creature's puppet; None = default.
    pub fn facing(&self, id: &str) -> Option<f32> {
        self.facings.get(id).copied()
    }

    /// Turn this creature's puppet to `deg`, or back to the default.
    pub fn set_facing(&mut self, id: &str, deg: Option<f32>) {
        match deg.filter(|d| d.is_finite()) {
            Some(d) => {
                self.facings.insert(id.to_string(), d);
            }
            None => {
                self.facings.remove(id);
            }
        }
    }

    /// Every facing set (for the renderer).
    pub fn facings(&self) -> impl Iterator<Item = (&String, &f32)> {
        self.facings.iter()
    }

    /// Every creature drawn as its puppet regardless of art (for the renderer).
    pub fn forced_puppets(&self) -> impl Iterator<Item = &String> {
        self.forced_puppets.iter()
    }

    pub fn remove(&mut self, id: &str) {
        self.forced_puppets.remove(id);
        self.facings.remove(id);
        self.creatures.retain(|c| c.id != id);
        if self.selected.as_deref() == Some(id) {
            self.selected = None;
        }
        if self.current_target == id {
            self.current_target = String::new();
        }
        self.resync();
    }

    pub fn clear(&mut self) {
        self.creatures.clear();
        self.forced_puppets.clear();
        self.facings.clear();
        self.selected = None;
        self.current_target = String::new();
        self.resync();
    }

    /// First flagged rider pairs with first flagged mount; changes tear
    /// down the old pair (when both halves still stand) before applying.
    pub fn refresh_mounts(&mut self) {
        let rider = self.creatures.iter().find(|c| c.flags.as_ref().is_some_and(|f| f.rider)).map(|c| c.id.clone());
        let mount = self
            .creatures
            .iter()
            .find(|c| c.flags.as_ref().is_some_and(|f| f.mount) && Some(&c.id) != rider.as_ref())
            .map(|c| c.id.clone());
        let desired = rider.zip(mount);
        if desired == self.pair {
            return;
        }
        if let Some((old_rider, _)) = self.pair.take() {
            let still_paired = self.field.unit_of(&old_rider).is_some_and(|u| u.members.len() > 1);
            if still_paired {
                let (standing, prone) = self
                    .creatures
                    .iter()
                    .find(|c| c.id == old_rider)
                    .map(gs_field::cards::card_boxes_for)
                    .unwrap_or_default();
                self.field.dismount(&old_rider, standing, prone);
            }
        }
        if let Some((rider, mount)) = &desired {
            let field = &mut self.field;
            if field.unit_of(rider).is_some_and(|u| u.members.len() == 1)
                && field.unit_of(mount).is_some_and(|u| u.members.len() == 1)
            {
                field.mount(rider, mount);
                self.pair = desired;
            }
        } else {
            self.pair = None;
        }
    }

    /// The applied rider/mount pair, if any.
    pub fn pair(&self) -> Option<(&str, &str)> {
        self.pair.as_ref().map(|(r, m)| (r.as_str(), m.as_str()))
    }

    // --------------------------------------------------------- simulator

    /// The creature's flags, created hostile if the roster entry had none
    /// (what opening the simulator on it did in the original).
    fn flags_mut(&mut self, id: &str) -> Option<&mut CreatureFlags> {
        let creature = self.creatures.iter_mut().find(|c| c.id == id)?;
        Some(creature.flags.get_or_insert_with(|| CreatureFlags { hostile: true, ..Default::default() }))
    }

    pub fn is_target(&self, id: &str) -> bool {
        self.current_target == id
    }

    /// The "Target" checkbox: on targets this creature, off clears the
    /// target (only if it was this creature).
    pub fn set_target(&mut self, id: &str, on: bool) {
        if on {
            self.current_target = id.to_string();
        } else if self.current_target == id {
            self.current_target = String::new();
        }
    }

    /// A card was clicked on the stage: target it locally (the renderer
    /// reports the id without a leading '#').
    pub fn target_clicked(&mut self, exist_id: String) {
        self.current_target = exist_id;
    }

    /// One canonical crtrStatus name on or off.
    pub fn set_status(&mut self, id: &str, canonical: &str, on: bool) {
        let Some(flags) = self.flags_mut(id) else {
            return;
        };
        let has = flags.statuses.iter().any(|s| s == canonical);
        if on && !has {
            flags.statuses.push(canonical.to_string());
        } else if !on {
            flags.statuses.retain(|s| s != canonical);
        }
    }

    pub fn set_class(&mut self, id: &str, class: ClassFlag, on: bool) {
        let Some(flags) = self.flags_mut(id) else {
            return;
        };
        let slot = class.slot(flags);
        let changed = *slot != on;
        *slot = on;
        if changed && class.structural() {
            self.resync();
        }
    }

    /// The "Health bar" checkbox: on starts at 100/100, off removes both.
    pub fn set_health_bar(&mut self, id: &str, on: bool) {
        let Some(flags) = self.flags_mut(id) else {
            return;
        };
        if on {
            flags.health = Some(100);
            flags.max_health = Some(100);
        } else {
            flags.health = None;
            flags.max_health = None;
        }
    }

    /// The health slider (0..=100 of a fixed 100). Ignored with no bar.
    pub fn set_health(&mut self, id: &str, health: u32) {
        let Some(flags) = self.flags_mut(id) else {
            return;
        };
        if flags.health.is_some() {
            flags.health = Some(health.min(100) as _);
            flags.max_health = Some(100);
        }
    }

    /// Wound rank 0-3 on one body part; 0 removes the entry.
    pub fn set_wound(&mut self, id: &str, part: &str, rank: u8) {
        let Some(flags) = self.flags_mut(id) else {
            return;
        };
        flags.injuries.retain(|(p, _)| p != part);
        let rank = rank.min(3);
        if rank > 0 {
            flags.injuries.push((part.to_string(), rank));
        }
    }

    pub fn wound_rank(&self, id: &str, part: &str) -> u8 {
        self.creature(id)
            .and_then(|c| c.flags.as_ref())
            .and_then(|f| f.injuries.iter().find(|(p, _)| p == part))
            .map_or(0, |(_, r)| *r)
    }

    // ------------------------------------------------- dragging creatures

    /// The exist id of the card under a stage-space point (topmost =
    /// nearest), for grabbing.
    pub fn creature_at(&self, sx: f32, sy: f32) -> Option<String> {
        let field = &self.field;
        let mut hit: Option<(f32, String)> = None;
        for u in field.units() {
            let r = field.rect(u);
            let inside = sx >= r.x0 && sx <= r.x1 && sy >= r.y0 && sy <= r.y1;
            if inside && hit.as_ref().is_none_or(|(z, _)| r.z < *z) {
                hit = Some((r.z, u.members[0].clone()));
            }
        }
        hit.map(|(_, id)| id)
    }

    pub fn begin_creature_drag(&mut self, sx: f32, sy: f32) {
        self.dragging_creature = self.creature_at(sx, sy);
    }

    /// Move the grabbed creature's ground point. The solver inverts the
    /// projection; separation is deliberately bypassed -- the Stage exists
    /// to compose exact arrangements.
    pub fn drag_creature_to(&mut self, sx: f32, sy: f32) {
        if let Some(id) = self.dragging_creature.clone() {
            self.field.place_at(&id, sx, sy);
        }
    }

    pub fn end_creature_drag(&mut self) {
        self.dragging_creature = None;
    }

    // ------------------------------------------------------------- scene

    pub fn scene(&self) -> &Arc<StageScene> {
        &self.scene
    }

    /// Edit the scene. Re-points the Arc, which is what marks the solver's
    /// obstacles stale.
    pub fn scene_mut(&mut self) -> &mut StageScene {
        Arc::make_mut(&mut self.scene)
    }

    /// Scene edits re-point the Arc (make_mut); rebuild the solver's
    /// AUTHORED exclusion metadata only then. Camera edits need no rebuild
    /// -- the solver projects the spans with whatever the current geometry
    /// is, per placement decision. Returns whether it rebuilt.
    pub fn refresh_obstacles(&mut self) -> bool {
        let scene_changed = self.obstacle_scene.as_ref().is_none_or(|prev| !Arc::ptr_eq(prev, &self.scene));
        if scene_changed {
            let obstacles = gs_field::cards::scene_obstacles(Some(&self.scene));
            self.field.set_obstacles(obstacles);
            self.obstacle_scene = Some(self.scene.clone());
        }
        scene_changed
    }

    /// Sidecars or the pool moved under the scene: obstacles are stale even
    /// though the scene Arc is not.
    pub fn invalidate_obstacles(&mut self) {
        self.obstacle_scene = None;
    }

    /// The pool directory changed: re-scan the cast, re-read prop sidecars,
    /// re-sync the roster against the new calibrations.
    pub fn pool_changed(&mut self) {
        self.cast = build_cast();
        self.invalidate_obstacles();
        self.resync();
        self.push_status(format!("Pool changed: {} castable bases", self.cast.len()));
    }

    pub fn clear_scene(&mut self) {
        self.scene = Arc::new(StageScene::default());
        self.scene_name.clear();
        self.selected_prop = None;
    }

    /// New props spawn at stage centre, mid depth, selected.
    pub fn add_prop(&mut self, image: String) {
        let (z_near, z_far) = self.field.depth_range();
        let scene = Arc::make_mut(&mut self.scene);
        scene.props.push(SceneProp { image, x: STAGE_W / 2.0, z: (z_near + z_far) / 2.0, scale: 1.0, shadow: false });
        self.selected_prop = Some(scene.props.len() - 1);
    }

    /// Remove prop `k`; the selection follows the prop it pointed at.
    /// A new point light of `kind`, mid-floor at the stage's centre,
    /// selected so a drag places it.
    pub fn add_light(&mut self, kind: gs_field::lighting::LampKind) {
        let (z_near, z_far) = self.field.depth_range();
        let light = gs_field::lighting::PointLight::new(kind, 440.0, (z_near + z_far) * 0.5);
        let lights = &mut Arc::make_mut(&mut self.scene).lights;
        lights.push(light);
        self.selected_light = Some(lights.len() - 1);
        self.selected_prop = None;
    }

    pub fn remove_light(&mut self, k: usize) {
        if k >= self.scene.lights.len() {
            return;
        }
        Arc::make_mut(&mut self.scene).lights.remove(k);
        self.selected_light = match self.selected_light {
            Some(s) if s == k => None,
            Some(s) if s > k => Some(s - 1),
            other => other,
        };
    }

    /// Move the selected light to the floor point under a stage position.
    pub fn drag_selected_light_to(&mut self, sx: f32, sy: f32) {
        let Some(k) = self.selected_light else { return };
        let (x, z) = self.field.ground_from_screen(sx, sy);
        if let Some(light) = Arc::make_mut(&mut self.scene).lights.get_mut(k) {
            light.x = x;
            light.z = z;
        }
    }

    pub fn remove_prop(&mut self, k: usize) {
        if k >= self.scene.props.len() {
            return;
        }
        Arc::make_mut(&mut self.scene).props.remove(k);
        self.selected_prop = match self.selected_prop {
            Some(s) if s == k => None,
            Some(s) if s > k => Some(s - 1),
            other => other,
        };
    }

    /// Keep prop `k` on the floor: x on the stage, z inside the depth range.
    pub fn clamp_prop(&mut self, k: usize) {
        let (z_near, z_far) = self.field.depth_range();
        let Some(prop) = self.scene.props.get(k) else {
            return;
        };
        let (x, z) = (prop.x.clamp(0.0, STAGE_W), prop.z.clamp(z_near, z_far));
        // Only re-point the Arc (= rebuild obstacles) when something moved.
        if x != prop.x || z != prop.z {
            let prop = &mut Arc::make_mut(&mut self.scene).props[k];
            prop.x = x;
            prop.z = z;
        }
    }

    /// Drag-to-place: move the selected prop on the ground plane (stage
    /// point -> ground, through the solver's inverse projection).
    pub fn drag_selected_prop_to(&mut self, sx: f32, sy: f32) {
        let Some(k) = self.selected_prop else {
            return;
        };
        let (x, z) = self.field.ground_from_screen(sx, sy);
        if let Some(prop) = Arc::make_mut(&mut self.scene).props.get_mut(k) {
            prop.x = x;
            prop.z = z;
        }
    }

    /// Make `stem` the save target; the stage carries over.
    pub fn adopt_scene_name(&mut self, stem: String) {
        self.scene_name = stem;
    }

    /// Save the on-stage scene under its stem, embedding the live camera/
    /// solver tuning SPARSELY: only what differs from the layer underneath
    /// (the default scene for room/location scenes, the built-in defaults
    /// for the default scene itself). Unset keys inherit live -- retune the
    /// default and every scene that didn't pin a value follows.
    pub fn save_current_scene(&mut self) {
        let name = self.scene_name.trim().to_string();
        if name.is_empty() {
            return;
        }
        // Validate before serialization with the same limits loading uses:
        // whatever previews is exactly what a reload will produce.
        if self.field.params.sanitize_camera(&FieldParams::default()) {
            self.push_status("Camera values clamped to valid range before save".to_string());
        }
        let params = self.field.params.clone();
        let base = sparse::inherited_params(&name);
        {
            let scene = Arc::make_mut(&mut self.scene);
            scene.camera = sparse::sparse_camera(&params, &base);
            scene.solver = sparse::sparse_solver(&params, &base);
        }
        match self.scene.save(&name) {
            Ok(()) => {
                self.push_status(format!("Scene '{name}' saved (camera/solver embedded sparse)"));
                // Calibration sidecars may have changed along with the
                // scene files; re-seed geometry (same call the game's
                // `reload_creature_field_files` makes).
                gs_field::geometry::invalidate_calibrations();
            }
            Err(err) => self.push_status(format!("Scene save failed: {err:#}")),
        }
    }

    /// Load a scene onto the stage, adopting its effective camera/solver
    /// (default-scene layer included, so a sparse room scene shows as it
    /// will in game).
    pub fn load_scene_by_name(&mut self, name: &str) {
        match StageScene::load(name) {
            Ok(scene) => {
                let default_scene =
                    if sparse::is_default_scene(name) { None } else { sparse::load_default_scene(Some(name)) };
                let params = gs_field::cards::resolve_field_params(
                    default_scene.as_ref(),
                    Some(&scene),
                    None,
                    &gs_field::field_config::FieldOverrides::default(),
                );
                let field = &mut self.field;
                if field.params != params {
                    field.params = params;
                    field.generation = field.generation.wrapping_add(1);
                }
                self.scene = Arc::new(scene);
                self.scene_name = name.to_string();
                self.selected_prop = None;
            }
            Err(err) => self.push_status(format!("Scene load failed: {err:#}")),
        }
    }

    // ------------------------------------------------------------ camera

    /// Is "Reset camera (inherited)" on offer? Only for a named,
    /// non-default scene.
    pub fn has_inherited_camera(&self) -> bool {
        let name = self.scene_name.trim();
        !name.is_empty() && !sparse::is_default_scene(name)
    }

    /// The solver's built-in default camera.
    pub fn reset_camera_builtin(&mut self) {
        self.copy_camera_from(&FieldParams::default());
    }

    /// The camera this scene inherits from the default scene layer -- what
    /// unpinned keys resolve to.
    pub fn reset_camera_inherited(&mut self) {
        let mut base = FieldParams::default();
        if let Some(default_scene) = sparse::load_default_scene(Some(self.scene_name.trim())) {
            base.apply_camera(&default_scene.camera);
        }
        self.copy_camera_from(&base);
    }

    fn copy_camera_from(&mut self, base: &FieldParams) {
        let params = &mut self.field.params;
        params.focal = base.focal;
        params.cam_h = base.cam_h;
        params.z0 = base.z0;
        params.dz = base.dz;
        params.horizon = base.horizon;
        params.cell_w = base.cell_w;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::test_support::TempBase;

    fn stage() -> StageState {
        StageState::with_cast(StudioRoomContext::default(), Vec::new())
    }

    fn flags<'a>(stage: &'a StageState, id: &str) -> &'a CreatureFlags {
        stage.creature(id).unwrap().flags.as_ref().unwrap()
    }

    #[test]
    fn spawn_selects_and_numbers_and_remove_cleans_up_after_itself() {
        let mut s = stage();
        s.spawn("forest troll", Some("troll".into()));
        s.spawn("kobold", Some("kobold".into()));
        assert_eq!(s.creatures().iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["studio-1", "studio-2"]);
        assert_eq!(s.selected.as_deref(), Some("studio-2"));
        assert!(flags(&s, "studio-1").hostile);

        s.set_target("studio-2", true);
        s.remove("studio-2");
        assert_eq!(s.selected, None);
        assert_eq!(s.current_target(), "");
        // Ids are never reused.
        s.spawn("kobold", Some("kobold".into()));
        assert_eq!(s.selected.as_deref(), Some("studio-3"));

        s.target_clicked("studio-1".into());
        s.clear();
        assert!(s.creatures().is_empty());
        assert_eq!((s.selected.clone(), s.current_target()), (None, ""));
    }

    /// Spawn-by-name puts a creature the pool has no art for on the stage
    /// -- the only way to see a non-humanoid puppet without emptying the
    /// pool -- and says so in the status line.
    /// The Puppets list puts a family's puppet up even when the pool's art
    /// (the `default` tier matches every creature) would otherwise win.
    #[test]
    fn spawning_a_puppet_forces_it_over_the_art() {
        let base = TempBase::new();
        let default = base.path().join("global/images/creatures/default");
        std::fs::create_dir_all(&default).unwrap();
        std::fs::write(default.join("default.png"), b"not really a png").unwrap();
        gs_field::pool::invalidate_cache();
        let mut s = stage();
        assert!(cast::has_art("timber wolf", Some("wolf")), "the default tier matches everything");

        assert!(s.spawn_puppet("canine"));
        let id = s.selected.clone().unwrap();
        assert!(s.is_forced_puppet(&id));
        assert_eq!(s.forced_puppets().count(), 1);
        assert!(s.status_lines().last().unwrap().contains("canine puppet"));
        // The simulator's checkbox, and removal, both clear it.
        s.set_forced_puppet(&id, false);
        assert!(!s.is_forced_puppet(&id));
        s.set_forced_puppet(&id, true);
        s.remove(&id);
        assert_eq!(s.forced_puppets().count(), 0);
        assert!(s.spawn_puppet("canine"));
        s.clear();
        assert_eq!(s.forced_puppets().count(), 0);
        assert!(!s.spawn_puppet("no_such_family"), "a family nobody belongs to is refused, with a reason");
    }

    #[test]
    fn spawning_by_name_works_without_art_and_reports_what_it_got() {
        let _base = TempBase::new();
        let mut s = stage();
        assert!(!s.spawn_named("   ", None), "a blank name spawns nothing");
        assert!(s.creatures().is_empty());

        assert!(s.spawn_named("  a large timber wolf  ", None));
        let c = &s.creatures()[0];
        assert_eq!((c.name.as_str(), c.noun.as_deref()), ("a large timber wolf", Some("wolf")));
        let line = s.status_lines().last().unwrap();
        assert!(line.contains("NO art in the pool") && line.contains("canine puppet"), "{line}");

        // An explicit noun overrides the last word.
        assert!(s.spawn_named("the Beast", Some("  dummy  ".into())));
        assert_eq!(s.creatures()[1].noun.as_deref(), Some("dummy"));
        assert!(s.status_lines().last().unwrap().contains("humanoid puppet"));

        // With art in the pool the sprite wins, and the line says so.
        let creatures = gs_field::data_dirs::global_image_category_dir("creatures").unwrap();
        std::fs::create_dir_all(&creatures).unwrap();
        std::fs::write(creatures.join("kobold.png"), b"x").unwrap();
        gs_field::pool::invalidate_cache();
        assert!(s.spawn_named("kobold", None));
        assert!(s.status_lines().last().unwrap().contains("sprite art"), "{:?}", s.status_lines().last());
    }

    #[test]
    fn target_checkbox_only_clears_its_own_target() {
        let mut s = stage();
        s.spawn("a", None);
        s.spawn("b", None);
        s.set_target("studio-1", true);
        s.set_target("studio-2", false);
        assert!(s.is_target("studio-1"), "unticking b leaves a targeted");
        s.set_target("studio-1", false);
        assert_eq!(s.current_target(), "");
    }

    #[test]
    fn roster_changes_reach_the_field_on_the_next_sync() {
        let mut s = stage();
        s.spawn("kobold", Some("kobold".into()));
        assert!(s.field.units().is_empty(), "spawn only marks the roster dirty");
        s.sync_field();
        assert_eq!(s.field.units().len(), 1);

        // A corpse keeps its card (tinted) until it leaves the roster.
        s.set_class("studio-1", ClassFlag::Dead, true);
        s.sync_field();
        assert!(s.field.unit_of("studio-1").is_some());

        s.remove("studio-1");
        s.sync_field();
        assert!(s.field.units().is_empty());
    }

    #[test]
    fn only_structural_class_changes_mark_the_roster_dirty() {
        let mut s = stage();
        s.spawn("kobold", Some("kobold".into()));
        let gen = s.creatures_generation;
        s.set_class("studio-1", ClassFlag::Sympathetic, true);
        s.set_status("studio-1", "stunned", true);
        s.set_wound("studio-1", "head", 2);
        s.set_health_bar("studio-1", true);
        assert_eq!(s.creatures_generation, gen);
        s.set_class("studio-1", ClassFlag::Boss, true);
        assert_eq!(s.creatures_generation, gen + 1);
        s.set_class("studio-1", ClassFlag::Boss, true);
        assert_eq!(s.creatures_generation, gen + 1, "no change, no resync");
        for class in ClassFlag::ALL {
            s.set_class("studio-1", class, true);
            assert!(class.get(flags(&s, "studio-1")), "{}", class.label());
        }
    }

    #[test]
    fn statuses_health_and_wounds_edit_the_flags() {
        let mut s = stage();
        s.spawn("kobold", Some("kobold".into()));
        let id = "studio-1";

        s.set_status(id, "stunned", true);
        s.set_status(id, "stunned", true);
        s.set_status(id, "webbed", true);
        assert_eq!(flags(&s, id).statuses, ["stunned", "webbed"]);
        s.set_status(id, "stunned", false);
        assert_eq!(flags(&s, id).statuses, ["webbed"]);

        s.set_health(id, 40);
        assert_eq!(flags(&s, id).health, None, "no bar, no health");
        s.set_health_bar(id, true);
        assert_eq!((flags(&s, id).health, flags(&s, id).max_health), (Some(100), Some(100)));
        s.set_health(id, 40);
        assert_eq!(flags(&s, id).health_percent(), Some(40));
        s.set_health_bar(id, false);
        assert_eq!((flags(&s, id).health, flags(&s, id).max_health), (None, None));

        s.set_wound(id, "head", 2);
        s.set_wound(id, "leftArm", 9);
        s.set_wound(id, "head", 3);
        assert_eq!((s.wound_rank(id, "head"), s.wound_rank(id, "leftArm"), s.wound_rank(id, "chest")), (3, 3, 0));
        assert_eq!(flags(&s, id).injuries.len(), 2, "one entry per part");
        s.set_wound(id, "head", 0);
        assert_eq!(flags(&s, id).injuries, [("leftArm".to_string(), 3)]);

        // Unknown ids are ignored, not panics.
        s.set_wound("nobody", "head", 1);
        s.set_class("nobody", ClassFlag::Dead, true);
    }

    #[test]
    fn a_roster_entry_without_flags_gets_hostile_ones_on_first_edit() {
        let mut s = stage();
        s.spawn("kobold", None);
        s.creatures[0].flags = None;
        s.set_status("studio-1", "prone", true);
        let f = flags(&s, "studio-1");
        assert!(f.hostile && f.statuses == ["prone"]);
    }

    #[test]
    fn first_rider_pairs_with_first_mount_and_unpairs_again() {
        let mut s = stage();
        s.spawn("orc", Some("orc".into()));
        s.spawn("warg", Some("warg".into()));
        s.spawn("wolf", Some("wolf".into()));
        s.set_class("studio-1", ClassFlag::Rider, true);
        s.sync_field();
        s.refresh_mounts();
        assert_eq!(s.pair(), None, "a rider alone pairs with nothing");

        s.set_class("studio-2", ClassFlag::Mount, true);
        s.set_class("studio-3", ClassFlag::Mount, true);
        s.sync_field();
        s.refresh_mounts();
        assert_eq!(s.pair(), Some(("studio-1", "studio-2")), "first mount wins");
        let unit = s.field.unit_of("studio-1").unwrap();
        assert_eq!(unit.members, ["studio-2", "studio-1"], "mount first");
        assert_eq!(s.field.units().len(), 2);

        // A creature flagged both ways never pairs with itself.
        s.set_class("studio-2", ClassFlag::Mount, false);
        s.set_class("studio-1", ClassFlag::Mount, true);
        s.sync_field();
        s.refresh_mounts();
        assert_eq!(s.pair(), Some(("studio-1", "studio-3")));

        s.set_class("studio-1", ClassFlag::Rider, false);
        s.sync_field();
        s.refresh_mounts();
        assert_eq!(s.pair(), None);
        assert_eq!(s.field.units().len(), 3, "everyone stands alone again");
    }

    #[test]
    fn grabbing_picks_the_card_under_the_point_and_drag_moves_it() {
        let mut s = stage();
        s.spawn("kobold", Some("kobold".into()));
        s.sync_field();
        let r = s.field.rect(&s.field.units()[0]);
        let (cx, cy) = ((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0);
        assert_eq!(s.creature_at(cx, cy).as_deref(), Some("studio-1"));
        assert_eq!(s.creature_at(r.x0 - 50.0, cy), None);

        s.begin_creature_drag(cx, cy);
        let before = s.field.foot(&s.field.units()[0]);
        s.drag_creature_to(before.0 + 120.0, before.1);
        let after = s.field.foot(&s.field.units()[0]);
        assert!((after.0 - (before.0 + 120.0)).abs() < 1.0, "{before:?} -> {after:?}");
        s.end_creature_drag();
        s.drag_creature_to(0.0, 0.0);
        assert_eq!(s.field.foot(&s.field.units()[0]), after, "no grab, no move");
    }

    #[test]
    fn props_add_selected_at_centre_and_removal_keeps_the_selection_honest() {
        let mut s = stage();
        for name in ["scenery/a.png", "scenery/b.png", "scenery/c.png"] {
            s.add_prop(name.into());
        }
        let (z_near, z_far) = s.field.depth_range();
        let c = &s.scene().props[2];
        assert_eq!((c.x, c.z, c.scale, c.shadow), (STAGE_W / 2.0, (z_near + z_far) / 2.0, 1.0, false));
        assert_eq!(s.selected_prop, Some(2));

        s.remove_prop(0);
        assert_eq!(s.selected_prop, Some(1), "still c");
        assert_eq!(s.scene().props[1].image, "scenery/c.png");
        s.selected_prop = Some(0);
        s.remove_prop(1);
        assert_eq!(s.selected_prop, Some(0), "removing a later prop changes nothing");
        s.remove_prop(0);
        assert_eq!(s.selected_prop, None);
        s.remove_prop(7);
        assert!(s.scene().props.is_empty());
    }

    #[test]
    fn prop_clamping_and_dragging_stay_on_the_floor() {
        let mut s = stage();
        s.add_prop("scenery/a.png".into());
        let (z_near, z_far) = s.field.depth_range();
        {
            let prop = &mut s.scene_mut().props[0];
            prop.x = -40.0;
            prop.z = z_far + 10.0;
        }
        s.clamp_prop(0);
        assert_eq!((s.scene().props[0].x, s.scene().props[0].z), (0.0, z_far));

        // An in-range clamp must not re-point the Arc (no obstacle rebuild).
        s.refresh_obstacles();
        s.clamp_prop(0);
        assert!(!s.refresh_obstacles());

        let ((sx, sy), _) = s.field.project_ground(300.0, (z_near + z_far) / 2.0);
        s.drag_selected_prop_to(sx, sy);
        let prop = &s.scene().props[0];
        assert!((prop.x - 300.0).abs() < 0.5 && (prop.z - (z_near + z_far) / 2.0).abs() < 0.01, "{prop:?}");
        assert!(s.refresh_obstacles(), "a drag is a scene edit");
    }

    #[test]
    fn obstacles_rebuild_only_when_the_scene_arc_moves() {
        let mut s = stage();
        assert!(s.refresh_obstacles(), "first frame");
        assert!(!s.refresh_obstacles());
        s.scene_mut().background_color = Some("#102030".into());
        assert!(s.refresh_obstacles());
        s.invalidate_obstacles();
        assert!(s.refresh_obstacles());
        s.clear_scene();
        assert!(s.refresh_obstacles());
    }

    #[test]
    fn save_embeds_sparse_tuning_and_load_restores_the_effective_camera() {
        let _base = TempBase::new();
        let mut s = stage();

        // Nameless: Save is a no-op.
        s.save_current_scene();
        assert!(gs_field::scenes::list_scenes().is_empty());

        // The default scene pins what differs from the built-ins.
        s.field.params.horizon = 160.0;
        s.adopt_scene_name("default".into());
        assert!(!s.has_inherited_camera());
        s.save_current_scene();
        let default_scene = StageScene::load("default").unwrap();
        assert_eq!(default_scene.camera.horizon, Some(160.0));
        assert_eq!(default_scene.camera.focal, None);

        // A room scene over it pins only ITS difference; an out-of-range
        // value is clamped (and reported) before it is written.
        s.field.params.focal = 20.0;
        s.scene_mut().background = Some("scenes/desert.png".into());
        s.adopt_scene_name("Barley_Field".into());
        assert!(s.has_inherited_camera());
        s.save_current_scene();
        assert!(s.status_lines().iter().any(|l| l.contains("clamped")));
        let room = StageScene::load("Barley_Field").unwrap();
        assert_eq!((room.camera.focal, room.camera.horizon), (Some(60.0), None));
        assert!(room.solver.is_empty());

        // The two resets.
        s.reset_camera_builtin();
        assert_eq!(s.field.params.horizon, FieldParams::default().horizon);
        s.reset_camera_inherited();
        assert_eq!((s.field.params.horizon, s.field.params.focal), (160.0, FieldParams::default().focal));

        // A fresh stage loading the room scene sees default layer + room.
        let mut fresh = stage();
        fresh.selected_prop = Some(3);
        fresh.load_scene_by_name("Barley_Field");
        assert_eq!((fresh.field.params.focal, fresh.field.params.horizon), (60.0, 160.0));
        assert_eq!(fresh.scene_name, "Barley_Field");
        assert_eq!(fresh.scene().background.as_deref(), Some("scenes/desert.png"));
        assert_eq!(fresh.selected_prop, None);

        fresh.load_scene_by_name("no_such_scene");
        assert!(fresh.status_lines().last().unwrap().starts_with("Scene load failed"));
        assert_eq!(fresh.scene_name, "Barley_Field", "a failed load changes nothing");
    }

    #[test]
    fn the_status_log_is_capped() {
        let mut s = stage();
        for n in 0..(STATUS_CAP + 20) {
            s.push_status(format!("line {n}"));
        }
        assert_eq!(s.status_lines().len(), STATUS_CAP);
        assert_eq!(s.status_lines().last().unwrap(), &format!("line {}", STATUS_CAP + 19));
    }
}
