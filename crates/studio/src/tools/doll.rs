//! The injury doll window: one puppet lit by the sky round the player
//! (`injury_doll::view`), with what the game would send it -- wounds
//! and scars per part, every status the doll draws (`injury_doll::state`),
//! a held weapon -- and the sky it stands under (the hour, the way the
//! player faces, the moon, indoors), to try it beside the field in the same
//! frame. It draws through `injury_doll::view::key_for`, as a game client does.
//! Drawing and input only.

use std::sync::Arc;

use injury_doll::state::{DollState, Status, Statuses};
use creature_field::lighting::Sky;
use creature_field_egui::creature_puppet::{PuppetCache, PuppetKey};

/// The compass points a player walks (`GameSky::walked`), and none.
const FACINGS: [(&str, Option<f32>); 9] = [
    ("none", None),
    ("n", Some(0.0)),
    ("ne", Some(45.0)),
    ("e", Some(90.0)),
    ("se", Some(135.0)),
    ("s", Some(180.0)),
    ("sw", Some(225.0)),
    ("w", Some(270.0)),
    ("nw", Some(315.0)),
];

/// The statuses, grouped as the checklist shows them.
const STATUS_GROUPS: [(&str, &[Status]); 5] = [
    ("Pose", &[Status::Kneeling, Status::Sitting, Status::Prone, Status::Stunned, Status::Sleeping, Status::Unconscious, Status::Webbed, Status::Dead]),
    (
        "On the body",
        &[Status::Frozen, Status::Immobile, Status::Poisoned, Status::Diseased, Status::Bleeding, Status::Rooted, Status::Bound, Status::Hidden, Status::Invisible, Status::Possessed, Status::Blinded, Status::Cutthroat, Status::Silenced],
    ),
    ("Holds itself", &[Status::Calmed, Status::Terrified, Status::Demoralized, Status::Feeble, Status::Crippled, Status::Staggered, Status::Dazed, Status::Confused]),
    ("Breath", &[Status::Frenzied, Status::Overexerted, Status::Slowed]),
    ("Else", &[Status::Vulnerable]),
];

/// The layered doll's dot spacing, feet.

pub struct DollWindow {
    pub open: bool,
    family: String,
    /// Held: `<weapon>[+<shield>]`, the stage's key (`family@weapon`).
    hold: String,
    /// Wound and scar rank per doll part, in `DOLL_PARTS` order.
    ranks: Vec<u8>,
    scars: Vec<u8>,
    statuses: Statuses,
    /// Turned from facing the viewer, degrees.
    turn: f32,
    breathe: bool,
    hour: f32,
    facing: usize,
    moon: f32,
    clouds: f32,
    indoor: bool,
    /// Stand under the field's own sky (the stage's scene, clock and
    /// facing) when it has one, not these sliders.
    follow_field: bool,
    /// Lit as a display (`injury_doll::sky::display`: a steady key and fill on a
    /// dark backdrop, the default) or by the sky round the player.
    display: bool,
    /// `GS_STUDIO_DOLL_HIT`: a wound rank to raise at a frame (part index,
    /// rank, frame), and the frames drawn so far -- a hit for a screenshot.
    hit: Option<(usize, u8, u32)>,
    frames: u32,
    /// Draw the skin on the GPU (`skin_gpu`) instead of the CPU.
    gpu_skin: bool,
    /// Draw it as layered splats (`gs_puppet::layers`): wounds open to
    /// flesh and bone, and `coat` builds on it, `coat_progress` of the way.
    layers: bool,
    /// Draw it as its form's modelled body (`gs_puppet::mesh`), where one
    /// is set (`GS_STUDIO_DOLL_BODY`).
    modelled: bool,
    /// ... as: 0 its mesh, 1 the splats fitted to it, 2 splats made from
    /// the mesh (`PuppetKey::body_cloud`).
    body_cloud: u8,
    /// The modelled bodies in the data root, by form (`crate::bodies`), and
    /// the size chosen per form (by name).
    bodies: Vec<crate::bodies::FormBodies>,
    body_size: std::collections::HashMap<String, String>,
    /// The (form, folder) last handed to the puppet cache.
    body_set: Option<(String, std::path::PathBuf)>,
    /// A (form, folder) to hand it when the window next shows
    /// (`GS_STUDIO_DOLL_BODY`, named before there is a cache to hand it to).
    body_named: Option<(String, std::path::PathBuf)>,
    /// The figure `follow_body` last saw.
    body_family: Option<String>,
    /// `GS_STUDIO_DOLL_FIGURE` named the figure (any form, a dev's way past
    /// the roster); else it starts as the roster's first.
    family_named: bool,
    /// `GS_STUDIO_DOLL_BODY_AS` said how to draw the body: a figure's own
    /// `doll.toml` does not override it.
    body_as_named: bool,
    coat: Option<gs_puppet::layers::Material>,
    coat_progress: f32,
    /// The coating came after the wounds (it lies over them) or before
    /// (they cut through it).
    coat_over: bool,
    /// What made each part's wound (`DOLL_PARTS` order), and what the next
    /// hit is: a part's cause is set when its wound rank is raised.
    causes: Vec<gs_puppet::layers::Cause>,
    hit_kind: gs_puppet::layers::Cause,
    /// The game's critical rank of each part's wound (1..=9), and the next
    /// hit's: how bad it is within its wound rank.
    crits: Vec<u8>,
    hit_crit: u8,
}

impl Default for DollWindow {
    fn default() -> Self {
        DollWindow {
            open: false,
            family: String::new(),
            hold: String::new(),
            ranks: vec![0; creature_field::skins::DOLL_PARTS.len()],
            scars: vec![0; creature_field::skins::DOLL_PARTS.len()],
            statuses: Statuses::NONE,
            turn: 20.0,
            breathe: true,
            hour: 12.0,
            facing: 0,
            moon: 1.0,
            clouds: 0.0,
            indoor: false,
            follow_field: false,
            display: true,
            hit: None,
            frames: 0,
            gpu_skin: true,
            layers: false,
            modelled: false,
            body_cloud: 2,
            bodies: Vec::new(),
            body_size: Default::default(),
            body_set: None,
            body_named: None,
            body_family: None,
            family_named: false,
            body_as_named: false,
            coat: None,
            coat_progress: 1.0,
            coat_over: false,
            causes: vec![Default::default(); creature_field::skins::DOLL_PARTS.len()],
            hit_kind: Default::default(),
            crits: vec![0; creature_field::skins::DOLL_PARTS.len()],
            hit_crit: 5,
        }
    }
}

/// The Doll window's puppet cache: over the pool's figures, then the
/// field's forms (`crate::doll_source::StudioDollSource`), apart from the
/// stage's cache.
fn doll_cache(ctx: &egui::Context) -> std::sync::Arc<std::sync::Mutex<PuppetCache>> {
    ctx.data_mut(|d| d.get_temp_mut_or_insert_with(egui::Id::new("gs_studio::doll_cache"), || std::sync::Arc::new(std::sync::Mutex::new(PuppetCache::over(crate::doll_source::StudioDollSource::default())))).clone())
}

/// `GS_STUDIO_DOLL_BODY_AS=mesh|trained|made` (made where it does not say).
fn body_as_env() -> u8 {
    match std::env::var("GS_STUDIO_DOLL_BODY_AS").as_deref().map(str::trim) {
        Ok("mesh") => 0,
        Ok("trained") | Ok("splat") => 1,
        _ => 2,
    }
}

impl DollWindow {
    /// `GS_STUDIO_DOLL=[<hour>][,<facing>][,indoor][,new][,sky][,field]` opens
    /// it for the run, lit as a display unless `sky` (the hour, facing,
    /// indoors and `new` moon are the sky's) or `field` (under the stage's
    /// sky).
    /// `GS_STUDIO_DOLL_HIT=back:2@60` raises that wound at that frame.
    /// (screenshots): `GS_STUDIO_DOLL=23,s`. `GS_STUDIO_DOLL_WOUNDS=chest:2,leftArm:3`
    /// sets its wounds.
    pub fn from_env() -> Self {
        let mut w = DollWindow::default();
        if let Ok(v) = std::env::var("GS_STUDIO_DOLL") {
            w.open = true;
            for (i, part) in v.split(',').map(str::trim).enumerate() {
                if part == "indoor" {
                    w.indoor = true;
                } else if part == "sky" {
                    w.display = false;
                } else if part == "field" {
                    w.follow_field = true;
                    w.display = false;
                } else if part == "new" {
                    w.moon = 0.0;
                } else if i == 0 {
                    w.hour = part.parse().unwrap_or(w.hour);
                } else if let Some(k) = FACINGS.iter().position(|(n, _)| n.eq_ignore_ascii_case(part)) {
                    w.facing = k;
                }
            }
        }
        // on by default; GS_STUDIO_DOLL_GPU=0 draws the CPU skin
        w.gpu_skin = std::env::var("GS_STUDIO_DOLL_GPU").map_or(true, |v| v.trim() != "0");
        // GS_STUDIO_DOLL_LAYERS=1 draws it as layered splats;
        // GS_STUDIO_DOLL_COAT=ice:0.6 coats it (and turns them on)
        w.layers = std::env::var("GS_STUDIO_DOLL_LAYERS").is_ok_and(|v| v.trim() != "0");
        // GS_STUDIO_DOLL_BODY=<folder> draws the humanoid as the modelled
        // body there (body.obj, color.png, weights.txt), posed on the GPU
        if let Some(dir) = std::env::var_os("GS_STUDIO_DOLL_BODY").filter(|v| !v.is_empty()) {
            w.body_named = Some(("humanoid".to_string(), dir.into()));
            // the humanoid wears it: drawn in place of the roster's first
            if !w.family_named {
                w.family = "humanoid".into();
                w.family_named = true;
            }
            w.modelled = true;
            w.body_cloud = body_as_env();
        }
        if let Some(coat) = std::env::var("GS_STUDIO_DOLL_COAT").ok().and_then(|v| gs_puppet::layers::Coating::parse(&v)) {
            (w.layers, w.coat, w.coat_progress, w.coat_over) = (true, Some(coat.material), coat.progress, coat.over_wounds);
        }
        // GS_STUDIO_DOLL_CAUSE=fire (every wound's), or
        // chest:fire,leftArm:puncture:9 (the game's damage type names; the
        // last number is the critical rank, 1..=9)
        if let Ok(v) = std::env::var("GS_STUDIO_DOLL_CAUSE") {
            for item in v.split(',') {
                match item.split_once(':') {
                    Some((part, rest)) => {
                        let (cause, crit) = rest.split_once(':').map_or((rest, None), |(c, n)| (c, n.trim().parse::<u8>().ok()));
                        if let (Some(label), Some(cause)) = (creature_field::dolls::part_label(part.trim()), gs_puppet::layers::Cause::from_name(cause)) {
                            w.causes[label as usize - 1] = cause;
                            w.crits[label as usize - 1] = crit.unwrap_or(0).min(9);
                        }
                    }
                    None => {
                        if let Some(cause) = gs_puppet::layers::Cause::from_name(item) {
                            w.causes.iter_mut().for_each(|c| *c = cause);
                            w.hit_kind = cause;
                        }
                    }
                }
            }
        }
        // GS_STUDIO_DOLL_FIGURE=<form>, GS_STUDIO_DOLL_FX=webbed,poisoned,...
        // (any status or effect name the game uses)
        if let Ok(v) = std::env::var("GS_STUDIO_DOLL_FIGURE") {
            w.family = v.trim().to_owned();
            w.family_named = true;
        }
        // GS_STUDIO_DOLL_BODY_AS=mesh|trained|made draws the body so, over its
        // doll.toml (with or without GS_STUDIO_DOLL_BODY)
        w.body_as_named = std::env::var_os("GS_STUDIO_DOLL_BODY_AS").is_some();
        if w.body_as_named {
            w.body_cloud = body_as_env();
        }
        if let Ok(v) = std::env::var("GS_STUDIO_DOLL_FX") {
            w.statuses = Statuses::from_names(v.split(','));
        }
        if let Ok(v) = std::env::var("GS_STUDIO_DOLL_HIT") {
            let parsed = v.split_once('@').and_then(|(wound, frame)| {
                let (part, rank) = wound.split_once(':')?;
                Some((creature_field::dolls::part_label(part.trim())? as usize - 1, rank.trim().parse().ok()?, frame.trim().parse().ok()?))
            });
            w.hit = parsed;
        }
        for (var, ranks) in [("GS_STUDIO_DOLL_WOUNDS", &mut w.ranks), ("GS_STUDIO_DOLL_SCARS", &mut w.scars)] {
            let Ok(v) = std::env::var(var) else { continue };
            for item in v.split(',') {
                if let Some((part, rank)) = item.split_once(':') {
                    if let (Some(label), Ok(rank)) = (creature_field::dolls::part_label(part.trim()), rank.trim().parse::<u8>()) {
                        ranks[label as usize - 1] = rank.min(3);
                    }
                }
            }
        }
        w
    }

    fn sky(&self, ctx: &egui::Context) -> injury_doll::sky::DollSky {
        if self.display {
            return injury_doll::sky::display();
        }
        if let Some(sky) = self.follow_field.then(|| creature_field_egui::creature_light::field_sky_now(ctx)).flatten().map(|f| injury_doll::sky::doll_sky(f.game_hour, &f.sky, f.outdoor)) {
            return sky;
        }
        self.own_sky()
    }

    fn own_sky(&self) -> injury_doll::sky::DollSky {
        let sky = Sky { moon: self.moon, clouds: self.clouds, facing: FACINGS[self.facing].1, ..Sky::default() };
        injury_doll::sky::doll_sky(Some(self.hour), &sky, !self.indoor)
    }

    /// What a game client would hand the doll.
    fn state(&self) -> DollState {
        let listed = |ranks: &[u8]| -> Vec<(String, u8)> {
            creature_field::skins::DOLL_PARTS
                .iter()
                .zip(ranks)
                .filter(|(_, r)| **r > 0)
                .map(|((part, _, _), r)| (part.to_string(), *r))
                .collect()
        };
        DollState { wounds: listed(&self.ranks), scars: listed(&self.scars), statuses: self.statuses }
    }

    /// The window knowing the data root's modelled bodies (`<data>/meshes`):
    /// their own puppets installed into the pool, and a form that has a
    /// body drawn as it.
    pub fn with_bodies(mut self, data: &std::path::Path) -> Self {
        self.bodies = crate::bodies::scan(data);
        match crate::bodies::install_all(data) {
            Ok(forms) if !forms.is_empty() => crate::pool_dir::invalidate_caches(),
            Ok(_) => {}
            Err(e) => eprintln!("gs-studio: modelled bodies: {e}"),
        }
        // the doll is one of its figures (the playable races, the extras),
        // not a creature's form
        if !self.family_named {
            if let Some(first) = injury_doll::roster::roster().first() {
                self.family = first.form.clone();
            }
        }
        // GS_STUDIO_DOLL_BODY=<size> picks that size of the figure's body
        // (a folder path still names one outright, for the humanoid)
        if let Some(v) = std::env::var("GS_STUDIO_DOLL_BODY").ok().filter(|v| !v.is_empty() && !std::path::Path::new(v).is_dir()) {
            for b in &self.bodies {
                self.body_size.insert(b.form.clone(), v.trim().to_owned());
            }
            self.modelled = true;
            self.body_cloud = body_as_env();
        }
        self
    }

    /// The folder of the figure's chosen body, when its form has one here.
    fn body_dir(&self) -> Option<std::path::PathBuf> {
        let form = self.bodies.iter().find(|b| b.form == self.family)?;
        let want = self.body_size.get(&self.family);
        form.sizes.iter().find(|s| Some(&s.0) == want).or(form.sizes.first()).map(|s| s.1.clone())
    }

    /// Hand the figure's chosen body to the puppet cache when it changes.
    fn follow_body(&mut self, ctx: &egui::Context) {
        let name_body = |form: &str, dir: std::path::PathBuf| doll_cache(ctx).lock().unwrap_or_else(|e| e.into_inner()).set_body_dir(form, Some(dir));
        if let Some((form, dir)) = self.body_named.take() {
            name_body(&form, dir);
        }
        // a form that is its body's own (it brought its puppet) is drawn as it
        if self.body_family.as_deref() != Some(self.family.as_str()) {
            if self.bodies.iter().any(|b| b.form == self.family && b.own_puppet) {
                self.modelled = true;
            }
            // drawn as its doll.toml says, unless GS_STUDIO_DOLL_BODY_AS said
            if let Some(figure) = injury_doll::roster::figure(&self.family).filter(|_| !self.body_as_named) {
                self.modelled = true;
                self.body_cloud = figure.draw.body_cloud();
            }
            self.body_family = Some(self.family.clone());
        }
        let Some(dir) = self.body_dir() else { return };
        let now = (self.family.clone(), dir);
        if self.body_set.as_ref() != Some(&now) {
            name_body(&now.0, now.1.clone());
            crate::pool_dir::invalidate_caches();
            self.body_set = Some(now);
        }
    }

    fn key(&self) -> PuppetKey {
        let key = injury_doll::view::key_for(&self.state()).with_facing(self.turn).with_modelled(self.modelled, self.body_cloud);
        // a modelled body's splats take wounds as the layered cloud does:
        // layered once it is hurt, scarred or coated (unhurt, the GPU poses it)
        let marked = injury_doll::view::body_marked(&key) || self.coat.is_some();
        if !self.layers && !(self.modelled && self.body_cloud > 0 && marked) {
            return key;
        }
        // finer than the stage's cloud: one figure, seen close
        let params = gs_puppet::splat::SplatParams { spacing: injury_doll::view::LAYERS_SPACING_FT, ..Default::default() };
        let causes: Vec<(String, gs_puppet::layers::Cause)> = creature_field::skins::DOLL_PARTS.iter().zip(&self.causes).map(|((part, _, _), c)| (part.to_string(), *c)).collect();
        let mut key = key.with_layers(true, params.into(), self.coat.map(|m| gs_puppet::layers::Coating::new(m, self.coat_progress).over_wounds(self.coat_over))).with_causes(&causes);
        key.crits.iter_mut().zip(&self.crits).for_each(|(k, c)| *k = *c);
        key
    }

    /// The window, over whatever tab is showing.
    pub fn show(&mut self, ctx: &egui::Context) {
        if self.open {
            self.follow_body(ctx);
            self.frames += 1;
            if let Some((part, rank, at)) = self.hit.filter(|h| self.frames >= h.2) {
                let _ = at;
                self.ranks[part] = self.ranks[part].max(rank.min(3));
                self.hit = None;
            }
        }
        let mut open = self.open;
        egui::Window::new("Doll").open(&mut open).default_size([900.0, 900.0]).resizable(true).show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                // as tall as the window gives it (a figure drawn small shows little of a modelled body)
                let tall = ui.available_height().clamp(440.0, 1400.0);
                let (rect, drag) = ui.allocate_exact_size(egui::vec2(tall * 260.0 / 440.0, tall), egui::Sense::drag());
                // drag across the doll to turn it round
                if drag.dragged() {
                    self.turn = (self.turn + drag.drag_delta().x * 0.8 + 180.0).rem_euclid(360.0) - 180.0;
                }
                drag.on_hover_text("Drag to turn it round");
                let family: Arc<str> = if self.hold.trim().is_empty() { Arc::from(self.family.as_str()) } else { Arc::from(format!("{}@{}", self.family, self.hold.trim())) };
                let cache = doll_cache(ctx);
                let started = std::time::Instant::now();
                let drawn = {
                    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
                    injury_doll::view::paint(ui, rect, &mut cache, &family, self.key(), &self.sky(ctx), self.breathe.then_some("studio doll"), self.gpu_skin)
                };
                // GS_STUDIO_DOLL_TIMES=1: what each frame's doll cost
                if std::env::var_os("GS_STUDIO_DOLL_TIMES").is_some() {
                    eprintln!("doll frame {} {:.2} ms", self.frames, started.elapsed().as_secs_f64() * 1000.0);
                }
                if !drawn {
                    let said = if family.is_empty() { "no figure installed: Save a form with a modelled body".to_string() } else { format!("no puppet for '{family}'") };
                    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, said, egui::FontId::proportional(14.0), ui.visuals().weak_text_color());
                }
                ui.vertical(|ui| {
                    egui::ScrollArea::vertical().max_height(tall).show(ui, |ui| self.controls(ui));
                });
            });
        });
        self.open = open;
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        super::group(ui, "Figure");
        // the doll's figures: the playable races, then the extras (the
        // pool's modelled bodies, `injury_doll::roster`) -- a creature's
        // form is the field's, not the doll's
        let roster = injury_doll::roster::roster();
        let shown = roster.iter().find(|f| f.form == self.family).map_or_else(|| self.family.clone(), |f| f.label.clone());
        egui::ComboBox::from_id_salt("doll_family")
            .selected_text(shown)
            .show_ui(ui, |ui| {
                let mut kind = None;
                for f in roster.iter() {
                    if kind.is_some_and(|k| k != f.kind) {
                        ui.separator();
                    }
                    kind = Some(f.kind);
                    ui.selectable_value(&mut self.family, f.form.clone(), &f.label);
                }
                if roster.is_empty() {
                    ui.label("No modelled bodies in the pool");
                }
            })
            .response
            .on_hover_text("The playable races, then the extras: the pool's modelled bodies (global/bodies, each with its doll.toml). GS_STUDIO_DOLL_FIGURE names any form.");
        ui.horizontal(|ui| {
            ui.label("Holds");
            ui.add(egui::TextEdit::singleline(&mut self.hold).desired_width(120.0).hint_text("weapon[+shield]"))
                .on_hover_text("A weapon from props/weapons.toml, and a shield after a +");
        });
        ui.checkbox(&mut self.breathe, "Breathes");
        ui.add(egui::Slider::new(&mut self.turn, -180.0..=180.0).text("turned").suffix(" deg"));
        ui.checkbox(&mut self.gpu_skin, "GPU skin")
            .on_hover_text("Draw the skin on the GPU (off: the CPU skin, the same figure, slower to move)");
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.modelled, "Modelled body").on_hover_text("The form's modelled body, posed on the GPU (GS_STUDIO_DOLL_BODY names its folder)");
            ui.add_enabled_ui(self.modelled, |ui| {
                let names = ["as its mesh", "as trained splats", "as made splats"];
                // splats are made from the mesh's paint, which does not land
                // on a thin mesh (nor without a sheet): it draws its trained
                // cloud instead (`creature_puppet::body_cloud`)
                let dir = self.body_dir();
                let paint_lands = dir.as_ref().is_some_and(|d| !d.join("body_thin.obj").is_file() && d.join("color.png").is_file());
                let shown = if self.body_cloud == 2 && !paint_lands { 1 } else { self.body_cloud.min(2) };
                egui::ComboBox::from_id_salt("doll_body_as").selected_text(names[shown as usize]).show_ui(ui, |ui| {
                    for (k, name) in names.iter().enumerate() {
                        ui.add_enabled_ui(k != 2 || paint_lands, |ui| ui.selectable_value(&mut self.body_cloud, k as u8, *name))
                            .response
                            .on_disabled_hover_text("Its paint does not land on its thin mesh (body_thin.obj): it draws its trained splats");
                    }
                })
                .response
                .on_hover_text("Its mesh; the splat cloud trained from pictures of it (splat.ply); or splats made straight from the mesh and its sheet, as sharp as the mesh");
            });
        });
        if let Some(form) = self.bodies.iter().find(|b| b.form == self.family).filter(|b| b.sizes.len() > 1).cloned() {
            let chosen = self.body_size.get(&self.family).cloned().unwrap_or_else(|| form.sizes[0].0.clone());
            ui.add_enabled_ui(self.modelled, |ui| {
                egui::ComboBox::from_id_salt("doll_body_size").selected_text(format!("size: {chosen}")).show_ui(ui, |ui| {
                    for (name, _) in &form.sizes {
                        if ui.selectable_label(*name == chosen, name).clicked() {
                            self.body_size.insert(self.family.clone(), name.clone());
                        }
                    }
                })
                .response
                .on_hover_text("Which size of the form's body is drawn (<data>/meshes/<form>/<size>)");
            });
        }
        ui.checkbox(&mut self.layers, "Layered splats")
            .on_hover_text("Dots anchored to the skin, in layers: flesh and bone under it where a rank 2 or 3 wound opens it, a coating built on it");
        ui.add_enabled_ui(self.layers, |ui| {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("doll_coat").selected_text(self.coat.map_or("no coating", |m| m.name())).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.coat, None, "no coating");
                    for m in gs_puppet::layers::Material::ALL {
                        ui.selectable_value(&mut self.coat, Some(m), m.name());
                    }
                });
                ui.add_enabled(self.coat.is_some(), egui::Slider::new(&mut self.coat_progress, 0.0..=1.0).text("built"));
            });
            ui.add_enabled(self.coat.is_some(), egui::Checkbox::new(&mut self.coat_over, "coated after the wounds"))
                .on_hover_text("On: it lies over the wounds, whole (hurt, then frozen). Off: the wounds came after and cut through it");
            egui::ComboBox::from_id_salt("doll_hit_kind").selected_text(format!("hits are: {}", self.hit_kind.name())).show_ui(ui, |ui| {
                for c in gs_puppet::layers::Cause::ALL {
                    ui.selectable_value(&mut self.hit_kind, c, c.name());
                }
            })
            .response
            .on_hover_text("What makes the next wound, by the game's damage types: raise a part's wound and it is that kind");
            ui.add(egui::Slider::new(&mut self.hit_crit, 1..=9).text("critical"))
                .on_hover_text("The game's critical rank of the next hit: three to a wound rank, each a little worse");
        });

        super::group(ui, "Statuses");
        for (group, list) in STATUS_GROUPS {
            ui.weak(group);
            ui.horizontal_wrapped(|ui| {
                for &st in list {
                    let mut on = self.statuses.has(st);
                    if ui.checkbox(&mut on, st.id()).changed() {
                        self.statuses.set(st, on);
                    }
                }
            });
        }
        if ui.button("None").clicked() {
            self.statuses = Statuses::NONE;
        }

        super::group(ui, "Wounds and scars");
        egui::Grid::new("doll_wounds").num_columns(3).show(ui, |ui| {
            ui.label("");
            ui.weak("wound");
            ui.weak("scar");
            ui.end_row();
            for (i, (_, name, _)) in creature_field::skins::DOLL_PARTS.iter().enumerate() {
                ui.label(*name);
                let before = self.ranks[i];
                ui.add(egui::Slider::new(&mut self.ranks[i], 0..=3));
                if self.ranks[i] > before {
                    self.causes[i] = self.hit_kind;
                    self.crits[i] = self.hit_crit;
                }
                ui.add(egui::Slider::new(&mut self.scars[i], 0..=3));
                ui.end_row();
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Heal").on_hover_text("Herbs: every wound becomes a scar of its rank").clicked() {
                for (r, s) in self.ranks.iter_mut().zip(self.scars.iter_mut()) {
                    *s = (*s).max(*r);
                    *r = 0;
                }
            }
            if ui.button("Clear").clicked() {
                self.ranks.iter_mut().chain(self.scars.iter_mut()).for_each(|r| *r = 0);
            }
        });

        super::group(ui, "Light");
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.display, true, "Display")
                .on_hover_text("A steady light on a dark backdrop: wounds and statuses read the same at any hour");
            ui.radio_value(&mut self.display, false, "The sky")
                .on_hover_text("The sun or moons round the player, from the way they face, over that sky");
        });
        if self.display {
            return;
        }
        let field = creature_field_egui::creature_light::field_sky_now(ui.ctx()).is_some();
        ui.checkbox(&mut self.follow_field, "The field's sky")
            .on_hover_text(if field {
                "Stand under the stage's own sky: its scene, hour, weather and the way it faces"
            } else {
                "Stand under the stage's own sky -- once the stage has drawn a scene (Preview)"
            });
        let own = !(self.follow_field && field);
        ui.add_enabled(own, egui::Checkbox::new(&mut self.indoor, "Indoors"));
        ui.add_enabled_ui(own && !self.indoor, |ui| {
            ui.add(egui::Slider::new(&mut self.hour, 0.0..=24.0).text("hour"));
            egui::ComboBox::from_id_salt("doll_facing").selected_text(format!("facing {}", FACINGS[self.facing].0)).show_ui(ui, |ui| {
                for (i, (name, _)) in FACINGS.iter().enumerate() {
                    ui.selectable_value(&mut self.facing, i, *name);
                }
            })
            .response
            .on_hover_text("The way the player last walked: the sun and moons keep to the compass round it");
            ui.add(egui::Slider::new(&mut self.moon, 0.0..=1.0).text("moon"));
            ui.add(egui::Slider::new(&mut self.clouds, 0.0..=1.0).text("clouds"));
        });
    }
}
