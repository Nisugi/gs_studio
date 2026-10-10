//! The structured condition-tree builder (dropdowns, no expression syntax):
//! edits a `creature_field::Condition` in place. Ported from VellumFE
//! `src/frontend/gui/app/editors/hotbars.rs` @60c84a79 (GPL-3.0-or-later),
//! lines 17-152 (leaf tables + `default_leaf`) and 1076-1567
//! (`render_condition_group`, `render_leaf_condition` and the combos). The
//! rest of that file is the hotbar editor and stays in VellumFE.
//!
//! One deliberate difference: the "Spell affordable" row's name/cost hint
//! came from VellumFE's `core::spell_table` (effect-list.xml). Here it comes
//! from an optional process-wide hook, [`set_spell_lookup`]; with no hook
//! installed the hint is simply not drawn. See PORTING.md.

use creature_field::condition::{Cmp, Condition, EffectCategory, NameMatch, VitalKind, VitalUnit};
use std::sync::RwLock;

/// What the "Spell affordable" row shows next to the spell number. The
/// fields mirror the ones the builder read from VellumFE's `SpellInfo`.
#[derive(Debug, Clone, Default)]
pub struct SpellHint {
    pub name: String,
    pub mana: Option<u16>,
    pub stamina: Option<u16>,
    pub spirit: Option<u16>,
    /// True when any cost is a formula (never affordable to the evaluator).
    pub dynamic_cost: bool,
}

type SpellLookup = Box<dyn Fn(u16) -> Option<SpellHint> + Send + Sync>;
static SPELL_LOOKUP: RwLock<Option<SpellLookup>> = RwLock::new(None);

/// Install the spell-number lookup behind the "Spell affordable" hint.
/// VellumFE passes a closure over `core::spell_table::spell`.
pub fn set_spell_lookup(lookup: impl Fn(u16) -> Option<SpellHint> + Send + Sync + 'static) {
    *SPELL_LOOKUP.write().unwrap() = Some(Box::new(lookup));
}

/// `None` = no hook installed (draw no hint); `Some(None)` = unknown number.
fn spell_hint(number: u16) -> Option<Option<SpellHint>> {
    SPELL_LOOKUP.read().unwrap().as_ref().map(|lookup| lookup(number))
}

const INDICATOR_IDS: &[&str] = &[
    "standing",
    "kneeling",
    "sitting",
    "prone",
    "stunned",
    "bleeding",
    "hidden",
    "invisible",
    "webbed",
    "joined",
    "dead",
];

const CMPS: &[Cmp] = &[Cmp::Lt, Cmp::Le, Cmp::Gt, Cmp::Ge];

const VITALS: &[VitalKind] = &[
    VitalKind::Health,
    VitalKind::Mana,
    VitalKind::Stamina,
    VitalKind::Spirit,
];

fn vital_name(v: VitalKind) -> &'static str {
    match v {
        VitalKind::Health => "health",
        VitalKind::Mana => "mana",
        VitalKind::Stamina => "stamina",
        VitalKind::Spirit => "spirit",
    }
}

fn category_name(c: EffectCategory) -> &'static str {
    match c {
        EffectCategory::Buffs => "Buffs",
        EffectCategory::Debuffs => "Debuffs",
        EffectCategory::Cooldowns => "Cooldowns",
        EffectCategory::ActiveSpells => "Active Spells",
    }
}

/// Human-readable label for a leaf condition kind (combo entries).
const LEAF_KINDS: &[&str] = &[
    "Effect active",
    "Effect inactive",
    "Effect time remaining",
    "Roundtime active",
    "Casttime active",
    "Indicator",
    "Vital",
    "Injury",
    "Spell affordable",
    "Hand empty",
    "Hand holds",
    "Spell prepared",
    "Time of day",
];

fn leaf_kind_index(cond: &Condition) -> usize {
    match cond {
        Condition::EffectActive { .. } => 0,
        Condition::EffectInactive { .. } => 1,
        Condition::EffectTime { .. } => 2,
        Condition::RtActive => 3,
        Condition::CtActive => 4,
        Condition::Indicator { .. } => 5,
        Condition::Vital { .. } => 6,
        Condition::Injury { .. } => 7,
        Condition::SpellAffordable { .. } => 8,
        Condition::HandEmpty { .. } => 9,
        Condition::HandHolds { .. } => 10,
        Condition::SpellPrepared { .. } => 11,
        Condition::TimeOfDay { .. } => 12,
        // Creature-scoped: not offered in LEAF_KINDS (player-scoped surfaces
        // can never satisfy it); a hand-authored one displays via the
        // fallback arm below without being rewritten.
        Condition::CrtrStatus { .. } => 0,
        Condition::All { .. } | Condition::Any { .. } => 0,
    }
}

fn default_leaf(kind: usize) -> Condition {
    match kind {
        0 => Condition::EffectActive {
            category: EffectCategory::Buffs,
            name: String::new(),
            name_match: NameMatch::Exact,
        },
        1 => Condition::EffectInactive {
            category: EffectCategory::Buffs,
            name: String::new(),
            name_match: NameMatch::Exact,
        },
        2 => Condition::EffectTime {
            category: EffectCategory::Buffs,
            name: String::new(),
            name_match: NameMatch::Exact,
            cmp: Cmp::Lt,
            seconds: 60,
        },
        3 => Condition::RtActive,
        4 => Condition::CtActive,
        5 => Condition::Indicator {
            id: "hidden".to_string(),
            active: true,
        },
        6 => Condition::Vital {
            vital: VitalKind::Stamina,
            cmp: Cmp::Lt,
            value: 25,
            unit: VitalUnit::Percent,
        },
        7 => Condition::Injury {
            area: "neck".to_string(),
            cmp: Cmp::Ge,
            level: 1,
        },
        8 => Condition::SpellAffordable { number: 101 },
        9 => Condition::HandEmpty {
            hand: creature_field::condition::HandSlot::Right,
        },
        10 => Condition::HandHolds {
            hand: creature_field::condition::HandSlot::Right,
            item_type: Some("weapon".to_string()),
            name: None,
            name_match: NameMatch::Contains,
        },
        11 => Condition::SpellPrepared {
            name: None,
            name_match: NameMatch::Contains,
        },
        _ => Condition::TimeOfDay {
            phase: creature_field::condition::DayPhase::Night,
        },
    }
}

/// Condition group editor. Groups may nest one level deep (editors enforce);
/// deeper hand-authored trees still render read-only as a summary.
/// Shared condition-tree builder (also used by the hand-icons editor).
pub fn render_condition_group(
    ui: &mut egui::Ui,
    id: &str,
    cond: &mut Condition,
    depth: usize,
    suggestions: &std::collections::HashMap<&'static str, Vec<String>>,
) -> bool {
    let mut changed = false;

    // Normalize a bare leaf at the root into a group so the UI is uniform
    if depth == 0 && !matches!(cond, Condition::All { .. } | Condition::Any { .. }) {
        let leaf = cond.clone();
        *cond = Condition::All {
            conditions: vec![leaf],
        };
        changed = true;
    }

    let is_all = matches!(cond, Condition::All { .. });
    ui.horizontal(|ui| {
        ui.label(if depth == 0 { "When" } else { "Group:" });
        let mut all_selected = is_all;
        egui::ComboBox::from_id_salt(format!("{}_grouptype", id))
            .selected_text(if all_selected { "all of" } else { "any of" })
            .show_ui(ui, |ui| {
                if ui
                    .selectable_value(&mut all_selected, true, "all of")
                    .clicked()
                    || ui
                        .selectable_value(&mut all_selected, false, "any of")
                        .clicked()
                {
                    if all_selected != is_all {
                        let conditions = match cond {
                            Condition::All { conditions } | Condition::Any { conditions } => {
                                std::mem::take(conditions)
                            }
                            _ => vec![],
                        };
                        *cond = if all_selected {
                            Condition::All { conditions }
                        } else {
                            Condition::Any { conditions }
                        };
                        changed = true;
                    }
                }
            });
        if ui.small_button("+ condition").clicked() {
            if let Condition::All { conditions } | Condition::Any { conditions } = cond {
                conditions.push(default_leaf(3));
                changed = true;
            }
        }
        if depth == 0 && ui.small_button("+ group").clicked() {
            if let Condition::All { conditions } | Condition::Any { conditions } = cond {
                conditions.push(Condition::Any {
                    conditions: vec![default_leaf(3)],
                });
                changed = true;
            }
        }
    });

    let (Condition::All { conditions } | Condition::Any { conditions }) = cond else {
        return changed;
    };

    let mut delete_idx: Option<usize> = None;
    for (idx, child) in conditions.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.add_space(12.0 * (depth as f32 + 1.0));
            if ui.small_button("x").clicked() {
                delete_idx = Some(idx);
            }
            match child {
                Condition::All { .. } | Condition::Any { .. } => {
                    if depth == 0 {
                        ui.vertical(|ui| {
                            changed |= render_condition_group(
                                ui,
                                &format!("{}_g{}", id, idx),
                                child,
                                depth + 1,
                                suggestions,
                            );
                        });
                    } else {
                        // Deeper nesting is file-authored only; keep intact.
                        // This builder is shared by hotbars, hand icons,
                        // indicators, and highlight alerts, so it must not
                        // name any one config file here.
                        ui.weak("(nested group - edit in the config file)");
                    }
                }
                leaf => {
                    changed |=
                        render_leaf_condition(ui, &format!("{}_l{}", id, idx), leaf, suggestions);
                }
            }
        });
    }
    if let Some(idx) = delete_idx {
        conditions.remove(idx);
        changed = true;
    }
    changed
}

/// One leaf condition row: kind combo plus that kind's fields.
fn render_leaf_condition(
    ui: &mut egui::Ui,
    id: &str,
    cond: &mut Condition,
    suggestions: &std::collections::HashMap<&'static str, Vec<String>>,
) -> bool {
    let mut changed = false;
    let current_kind = leaf_kind_index(cond);
    egui::ComboBox::from_id_salt(format!("{}_kind", id))
        .selected_text(LEAF_KINDS[current_kind])
        .width(150.0)
        .show_ui(ui, |ui| {
            for (kind, label) in LEAF_KINDS.iter().enumerate() {
                if ui.selectable_label(kind == current_kind, *label).clicked()
                    && kind != current_kind
                {
                    *cond = default_leaf(kind);
                    changed = true;
                }
            }
        });

    match cond {
        Condition::EffectActive {
            category,
            name,
            name_match,
        }
        | Condition::EffectInactive {
            category,
            name,
            name_match,
        } => {
            changed |= category_combo(ui, &format!("{}_cat", id), category);
            changed |= effect_name_field(ui, &format!("{}_name", id), name, category, suggestions);
            changed |= match_combo(ui, &format!("{}_match", id), name_match);
        }
        Condition::EffectTime {
            category,
            name,
            name_match,
            cmp,
            seconds,
        } => {
            changed |= category_combo(ui, &format!("{}_cat", id), category);
            changed |= effect_name_field(ui, &format!("{}_name", id), name, category, suggestions);
            changed |= match_combo(ui, &format!("{}_match", id), name_match);
            changed |= cmp_combo(ui, &format!("{}_cmp", id), cmp);
            changed |= ui
                .add(egui::DragValue::new(seconds).range(0..=86_400).suffix("s"))
                .changed();
        }
        Condition::Indicator { id: ind_id, active } => {
            egui::ComboBox::from_id_salt(format!("{}_ind", id))
                .selected_text(ind_id.as_str())
                .show_ui(ui, |ui| {
                    for candidate in INDICATOR_IDS {
                        if ui
                            .selectable_label(ind_id == candidate, *candidate)
                            .clicked()
                        {
                            *ind_id = candidate.to_string();
                            changed = true;
                        }
                    }
                });
            changed |= ui.checkbox(active, "active").changed();
        }
        Condition::Vital {
            vital,
            cmp,
            value,
            unit,
        } => {
            egui::ComboBox::from_id_salt(format!("{}_vital", id))
                .selected_text(vital_name(*vital))
                .show_ui(ui, |ui| {
                    for candidate in VITALS {
                        if ui
                            .selectable_label(vital == candidate, vital_name(*candidate))
                            .clicked()
                        {
                            *vital = *candidate;
                            changed = true;
                        }
                    }
                });
            changed |= cmp_combo(ui, &format!("{}_cmp", id), cmp);
            changed |= ui
                .add(egui::DragValue::new(value).range(0..=100_000))
                .changed();
            let unit_label = match unit {
                VitalUnit::Percent => "%",
                VitalUnit::Absolute => "abs",
            };
            egui::ComboBox::from_id_salt(format!("{}_unit", id))
                .selected_text(unit_label)
                .width(60.0)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(matches!(unit, VitalUnit::Percent), "%")
                        .clicked()
                    {
                        *unit = VitalUnit::Percent;
                        changed = true;
                    }
                    if ui
                        .selectable_label(matches!(unit, VitalUnit::Absolute), "abs")
                        .clicked()
                    {
                        *unit = VitalUnit::Absolute;
                        changed = true;
                    }
                });
        }
        Condition::Injury { area, cmp, level } => {
            egui::ComboBox::from_id_salt(format!("{}_area", id))
                .selected_text(area.as_str())
                .show_ui(ui, |ui| {
                    for candidate in creature_field::condition::INJURY_AREAS {
                        if ui.selectable_label(area == candidate, *candidate).clicked() {
                            *area = candidate.to_string();
                            changed = true;
                        }
                    }
                });
            changed |= cmp_combo(ui, &format!("{}_cmp", id), cmp);
            // Levels: 1-3 wounds, 4-6 scars (0 = healthy).
            changed |= ui
                .add(egui::DragValue::new(level).range(0..=6))
                .on_hover_text("1-3 = wounds, 4-6 = scars, 0 = healthy")
                .changed();
        }
        Condition::TimeOfDay { phase } => {
            use creature_field::condition::DayPhase;
            ui.label("phase:");
            egui::ComboBox::from_id_salt(format!("{id}_phase"))
                .selected_text(phase.as_str())
                .show_ui(ui, |ui| {
                    for option in [
                        DayPhase::Dawn,
                        DayPhase::Day,
                        DayPhase::Dusk,
                        DayPhase::Night,
                    ] {
                        changed |= ui
                            .selectable_value(phase, option, option.as_str())
                            .changed();
                    }
                });
            ui.weak("Elanthian time (US Eastern), from your system clock");
        }
        Condition::SpellAffordable { number } => {
            ui.label("spell #:");
            changed |= ui
                .add(egui::DragValue::new(number).range(1..=9999))
                .changed();
            // Bundled-table lookup as instant feedback on the number.
            // `None` here = no lookup hook installed: draw no hint.
            match spell_hint(*number) {
                None => {}
                Some(Some(info)) if info.dynamic_cost => {
                    ui.weak(format!(
                        "{} (variable cost - never affordable here)",
                        info.name
                    ));
                }
                Some(Some(info)) => {
                    let mut costs = Vec::new();
                    if let Some(m) = info.mana {
                        costs.push(format!("{} mana", m));
                    }
                    if let Some(s) = info.stamina {
                        costs.push(format!("{} stamina", s));
                    }
                    if let Some(s) = info.spirit {
                        costs.push(format!("{} spirit", s));
                    }
                    let costs = if costs.is_empty() {
                        "no cost".to_string()
                    } else {
                        costs.join(", ")
                    };
                    ui.weak(format!("{} ({})", info.name, costs));
                }
                Some(None) => {
                    ui.weak("unknown spell number");
                }
            }
        }
        Condition::HandEmpty { hand } => {
            changed |= hand_slot_combo(ui, &format!("{}_hand", id), hand);
        }
        Condition::HandHolds {
            hand,
            item_type,
            name,
            name_match,
        } => {
            changed |= hand_slot_combo(ui, &format!("{}_hand", id), hand);
            ui.label("type:");
            let mut type_text = item_type.clone().unwrap_or_default();
            if ui
                .add(
                    egui::TextEdit::singleline(&mut type_text)
                        .hint_text("weapon")
                        .desired_width(90.0),
                )
                .on_hover_text(
                    "gameobj-data type tag (weapon, armor, gem, wand, ...). \
                     Empty = any item. No 'shield' type exists — use armor \
                     plus a name match.",
                )
                .changed()
            {
                let trimmed = type_text.trim();
                *item_type = (!trimmed.is_empty()).then(|| trimmed.to_string());
                changed = true;
            }
            ui.label("name:");
            let mut name_text = name.clone().unwrap_or_default();
            if ui
                .add(
                    egui::TextEdit::singleline(&mut name_text)
                        .hint_text("any")
                        .desired_width(120.0),
                )
                .changed()
            {
                let trimmed = name_text.trim();
                *name = (!trimmed.is_empty()).then(|| trimmed.to_string());
                changed = true;
            }
            changed |= match_combo(ui, &format!("{}_match", id), name_match);
        }
        Condition::SpellPrepared { name, name_match } => {
            ui.label("spell:");
            let mut name_text = name.clone().unwrap_or_default();
            if ui
                .add(
                    egui::TextEdit::singleline(&mut name_text)
                        .hint_text("any spell")
                        .desired_width(140.0),
                )
                .changed()
            {
                let trimmed = name_text.trim();
                *name = (!trimmed.is_empty()).then(|| trimmed.to_string());
                changed = true;
            }
            changed |= match_combo(ui, &format!("{}_match", id), name_match);
        }
        Condition::RtActive | Condition::CtActive => {}
        Condition::CrtrStatus { id: flag, active } => {
            // Creature-scoped, hand-authored: display honestly. It can never
            // fire on a player-scoped surface (no creature in scope).
            ui.weak(format!(
                "creature status \"{flag}\" {} — creature cards only, never \
                 matches here",
                if *active { "active" } else { "inactive" }
            ));
        }
        Condition::All { .. } | Condition::Any { .. } => {}
    }
    changed
}

fn hand_slot_combo(ui: &mut egui::Ui, id: &str, hand: &mut creature_field::condition::HandSlot) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id.to_string())
        .selected_text(hand.label())
        .width(110.0)
        .show_ui(ui, |ui| {
            for candidate in creature_field::condition::HandSlot::ALL {
                if ui
                    .selectable_label(*hand == candidate, candidate.label())
                    .clicked()
                {
                    *hand = candidate;
                    changed = true;
                }
            }
        });
    changed
}

fn category_combo(ui: &mut egui::Ui, id: &str, category: &mut EffectCategory) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id.to_string())
        .selected_text(category_name(*category))
        .width(110.0)
        .show_ui(ui, |ui| {
            for candidate in EffectCategory::ALL {
                if ui
                    .selectable_label(*category == candidate, category_name(candidate))
                    .clicked()
                {
                    *category = candidate;
                    changed = true;
                }
            }
        });
    changed
}

fn effect_name_field(
    ui: &mut egui::Ui,
    id: &str,
    name: &mut String,
    category: &EffectCategory,
    suggestions: &std::collections::HashMap<&'static str, Vec<String>>,
) -> bool {
    let mut changed = ui
        .add(egui::TextEdit::singleline(name).desired_width(150.0))
        .changed();
    let known = suggestions
        .get(category.state_key())
        .map(|v| v.as_slice())
        .unwrap_or(&[]);
    if !known.is_empty() {
        egui::ComboBox::from_id_salt(format!("{}_suggest", id))
            .selected_text("...")
            .width(30.0)
            .show_ui(ui, |ui| {
                for candidate in known {
                    if ui.selectable_label(false, candidate).clicked() {
                        *name = candidate.clone();
                        changed = true;
                    }
                }
            });
    }
    changed
}

fn match_combo(ui: &mut egui::Ui, id: &str, name_match: &mut NameMatch) -> bool {
    let mut changed = false;
    let label = match name_match {
        NameMatch::Exact => "exact",
        NameMatch::Contains => "contains",
    };
    egui::ComboBox::from_id_salt(id.to_string())
        .selected_text(label)
        .width(90.0)
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(matches!(name_match, NameMatch::Exact), "exact")
                .clicked()
            {
                *name_match = NameMatch::Exact;
                changed = true;
            }
            if ui
                .selectable_label(matches!(name_match, NameMatch::Contains), "contains")
                .clicked()
            {
                *name_match = NameMatch::Contains;
                changed = true;
            }
        });
    changed
}

fn cmp_combo(ui: &mut egui::Ui, id: &str, cmp: &mut Cmp) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id.to_string())
        .selected_text(cmp.symbol())
        .width(50.0)
        .show_ui(ui, |ui| {
            for candidate in CMPS {
                if ui
                    .selectable_label(cmp == candidate, candidate.symbol())
                    .clicked()
                {
                    *cmp = *candidate;
                    changed = true;
                }
            }
        });
    changed
}
