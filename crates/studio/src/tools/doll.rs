//! The injury doll window: one puppet lit by the sky round the player
//! (`gs_field_egui::doll_view`), with what the game would send it -- wounds
//! and scars per part, every status the doll draws (`gs_field::doll_state`),
//! a held weapon -- and the sky it stands under (the hour, the way the
//! player faces, the moon, indoors), to try it beside the field in the same
//! frame. It draws through `doll_view::key_for`, as a game client does.
//! Drawing and input only.

use std::sync::Arc;

use gs_field::doll_state::{DollState, Status, Statuses};
use gs_field::lighting::Sky;
use gs_field_egui::creature_puppet::{PuppetCache, PuppetKey};

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
    /// Lit as a display (`doll_sky::display`: a steady key and fill on a
    /// dark backdrop, the default) or by the sky round the player.
    display: bool,
    /// `GS_STUDIO_DOLL_HIT`: a wound rank to raise at a frame (part index,
    /// rank, frame), and the frames drawn so far -- a hit for a screenshot.
    hit: Option<(usize, u8, u32)>,
    frames: u32,
    /// Draw the skin on the GPU (`skin_gpu`) instead of the CPU.
    gpu_skin: bool,
}

impl Default for DollWindow {
    fn default() -> Self {
        DollWindow {
            open: false,
            family: "humanoid".into(),
            hold: String::new(),
            ranks: vec![0; gs_field::skins::DOLL_PARTS.len()],
            scars: vec![0; gs_field::skins::DOLL_PARTS.len()],
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
        }
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
        // GS_STUDIO_DOLL_FIGURE=<form>, GS_STUDIO_DOLL_FX=webbed,poisoned,...
        // (any status or effect name the game uses)
        if let Ok(v) = std::env::var("GS_STUDIO_DOLL_FIGURE") {
            w.family = v.trim().to_owned();
        }
        if let Ok(v) = std::env::var("GS_STUDIO_DOLL_FX") {
            w.statuses = Statuses::from_names(v.split(','));
        }
        if let Ok(v) = std::env::var("GS_STUDIO_DOLL_HIT") {
            let parsed = v.split_once('@').and_then(|(wound, frame)| {
                let (part, rank) = wound.split_once(':')?;
                Some((gs_field::dolls::part_label(part.trim())? as usize - 1, rank.trim().parse().ok()?, frame.trim().parse().ok()?))
            });
            w.hit = parsed;
        }
        for (var, ranks) in [("GS_STUDIO_DOLL_WOUNDS", &mut w.ranks), ("GS_STUDIO_DOLL_SCARS", &mut w.scars)] {
            let Ok(v) = std::env::var(var) else { continue };
            for item in v.split(',') {
                if let Some((part, rank)) = item.split_once(':') {
                    if let (Some(label), Ok(rank)) = (gs_field::dolls::part_label(part.trim()), rank.trim().parse::<u8>()) {
                        ranks[label as usize - 1] = rank.min(3);
                    }
                }
            }
        }
        w
    }

    fn sky(&self, ctx: &egui::Context) -> gs_field::doll_sky::DollSky {
        if self.display {
            return gs_field::doll_sky::display();
        }
        if let Some(sky) = self.follow_field.then(|| gs_field_egui::doll_view::field_sky(ctx)).flatten() {
            return sky;
        }
        self.own_sky()
    }

    fn own_sky(&self) -> gs_field::doll_sky::DollSky {
        let sky = Sky { moon: self.moon, clouds: self.clouds, facing: FACINGS[self.facing].1, ..Sky::default() };
        gs_field::doll_sky::doll_sky(Some(self.hour), &sky, !self.indoor)
    }

    /// What a game client would hand the doll.
    fn state(&self) -> DollState {
        let listed = |ranks: &[u8]| -> Vec<(String, u8)> {
            gs_field::skins::DOLL_PARTS
                .iter()
                .zip(ranks)
                .filter(|(_, r)| **r > 0)
                .map(|((part, _, _), r)| (part.to_string(), *r))
                .collect()
        };
        DollState { wounds: listed(&self.ranks), scars: listed(&self.scars), statuses: self.statuses }
    }

    fn key(&self) -> PuppetKey {
        gs_field_egui::doll_view::key_for(&self.state()).with_facing(self.turn)
    }

    /// The window, over whatever tab is showing.
    pub fn show(&mut self, ctx: &egui::Context) {
        if self.open {
            self.frames += 1;
            if let Some((part, rank, at)) = self.hit.filter(|h| self.frames >= h.2) {
                let _ = at;
                self.ranks[part] = self.ranks[part].max(rank.min(3));
                self.hit = None;
            }
        }
        let mut open = self.open;
        egui::Window::new("Doll").open(&mut open).default_size([520.0, 520.0]).resizable(true).show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                let (rect, drag) = ui.allocate_exact_size(egui::vec2(260.0, 440.0), egui::Sense::drag());
                // drag across the doll to turn it round
                if drag.dragged() {
                    self.turn = (self.turn + drag.drag_delta().x * 0.8 + 180.0).rem_euclid(360.0) - 180.0;
                }
                drag.on_hover_text("Drag to turn it round");
                let family: Arc<str> = if self.hold.trim().is_empty() { Arc::from(self.family.as_str()) } else { Arc::from(format!("{}@{}", self.family, self.hold.trim())) };
                let cache = PuppetCache::shared(ctx);
                let started = std::time::Instant::now();
                let drawn = {
                    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
                    gs_field_egui::doll_view::paint(ui, rect, &mut cache, &family, self.key(), &self.sky(ctx), self.breathe.then_some("studio doll"), self.gpu_skin)
                };
                // GS_STUDIO_DOLL_TIMES=1: what each frame's doll cost
                if std::env::var_os("GS_STUDIO_DOLL_TIMES").is_some() {
                    eprintln!("doll frame {} {:.2} ms", self.frames, started.elapsed().as_secs_f64() * 1000.0);
                }
                if !drawn {
                    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, format!("no puppet for '{family}'"), egui::FontId::proportional(14.0), ui.visuals().weak_text_color());
                }
                ui.vertical(|ui| {
                    egui::ScrollArea::vertical().max_height(440.0).show(ui, |ui| self.controls(ui));
                });
            });
        });
        self.open = open;
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        super::group(ui, "Figure");
        egui::ComboBox::from_id_salt("doll_family").selected_text(&self.family).show_ui(ui, |ui| {
            let rigs = gs_field::puppets::puppets();
            let mut families: Vec<&str> = rigs.families().collect();
            families.sort_unstable();
            for f in families {
                ui.selectable_value(&mut self.family, f.to_owned(), f);
            }
        });
        ui.horizontal(|ui| {
            ui.label("Holds");
            ui.add(egui::TextEdit::singleline(&mut self.hold).desired_width(120.0).hint_text("weapon[+shield]"))
                .on_hover_text("A weapon from props/weapons.toml, and a shield after a +");
        });
        ui.checkbox(&mut self.breathe, "Breathes");
        ui.add(egui::Slider::new(&mut self.turn, -180.0..=180.0).text("turned").suffix(" deg"));
        ui.checkbox(&mut self.gpu_skin, "GPU skin")
            .on_hover_text("Draw the skin on the GPU (off: the CPU skin, the same figure, slower to move)");

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
            for (i, (_, name, _)) in gs_field::skins::DOLL_PARTS.iter().enumerate() {
                ui.label(*name);
                ui.add(egui::Slider::new(&mut self.ranks[i], 0..=3));
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
        let field = gs_field_egui::doll_view::field_sky(ui.ctx()).is_some();
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
