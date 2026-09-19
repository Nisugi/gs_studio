//! The sandbox's condition evaluator.
//!
//! VellumFE's Stage evaluated skin `when` conditions with the client's REAL
//! evaluator, `core::conditions::eval_condition_for_creature(cond,
//! &GameState::new(), now_server = Utc::now(), gameobj = None, flags)`
//! (`frontend/gui/studio.rs:1117-1126`). There is no `GameState` here, so
//! this evaluator returns, leaf by leaf, exactly what that call returns for
//! a freshly constructed game state. Every answer below is derived from
//! VellumFE @60c84a79 -- `src/core/conditions.rs` ("c.rs") and the
//! `GameState::new()` defaults in `src/core/state.rs` ("state.rs"):
//!
//! | Leaf | Empty-state answer | Derived from |
//! |---|---|---|
//! | `All` / `Any` | recurse (empty `All` true, empty `Any` false) | c.rs:160-165 |
//! | `CrtrStatus` | `flags.has_flag(id) == active` | c.rs:166-168 |
//! | `EffectActive` | false: `effects` is empty, the lookup finds nothing | c.rs:173-174, 348; state.rs:2038 |
//! | `EffectInactive` | TRUE: the negation of the above | c.rs:179-180 |
//! | `EffectTime` | false: no effect, no expiry | c.rs:187-190 |
//! | `RtActive` / `CtActive` | false: `roundtime_end` / `casttime_end` are `None` | c.rs:191-192; state.rs:2028-2029 |
//! | `Indicator` | `!active`: a never-reported id reads false, so `active = false` MATCHES | c.rs:193-195, 393-395; state.rs:361-363 |
//! | `Vital`, percent | `cmp(100, value)`: a fresh player's bars are FULL (100), not zero | c.rs:201-203, 401-406; state.rs:2189-2197 |
//! | `Vital`, absolute | false: minivitals have `max == 0` until data arrives | c.rs:407-414; state.rs:2083 |
//! | `Injury` | `cmp(0, level)`: an absent part is level 0 | c.rs:204-209; state.rs:2041 |
//! | `SpellAffordable` | false -- see below | c.rs:323-338 |
//! | `TimeOfDay` | the wall clock's phase == `phase` | c.rs:211-219 |
//! | `HandEmpty` | TRUE for every slot: no held item, no prepared spell | c.rs:220-227, 252-268; state.rs:2030, 2035-2036 |
//! | `HandHolds` | false: nothing is held (and `gameobj` is `None`) | c.rs:291-294, 309-313 |
//! | `SpellPrepared` | false: `spell` is `None` | c.rs:241-244, 265-268 |
//!
//! ONE residual difference, `SpellAffordable`: the original looks the number
//! up in VellumFE's bundled spell table (`effect-list.xml`), which this
//! workspace does not have. With an empty game state the original is false
//! for every spell that costs anything (minivitals `max == 0`), for formula
//! costs and for unknown numbers -- but TRUE for a known spell that lists no
//! static cost at all (c.rs:330-337: `None => true` three times). Without the
//! table every number is "unknown" here, so this is always false.
//!
//! `TimeOfDay` reads the clock, so the pure form takes the timestamp:
//! [`eval_condition_at`]. [`eval_condition`] is that at "now", like the
//! original's `chrono::Utc::now().timestamp()`.
//!
//! The match has no wildcard arm: a new `Condition` variant does not compile
//! until it is given an answer here.

use gs_field::condition::{HandSlot, VitalUnit};
use gs_field::{Condition, CreatureFlags};

use super::elanthian_time::{phase_at, PhaseBoundaries};

/// Percent vitals of `GameState::new()`: `Vitals::default()` is 100 across
/// the board (state.rs:2189-2197).
const FRESH_VITAL_PERCENT: i64 = 100;

/// Does `cond` hold, right now, for a creature with these flags and a
/// player with an empty game state?
pub fn eval_condition(cond: &Condition, flags: &CreatureFlags) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    eval_condition_at(cond, flags, now)
}

/// [`eval_condition`] at a given UTC unix time (only `TimeOfDay` reads it).
pub fn eval_condition_at(cond: &Condition, flags: &CreatureFlags, now_unix: i64) -> bool {
    match cond {
        Condition::All { conditions } => conditions.iter().all(|c| eval_condition_at(c, flags, now_unix)),
        Condition::Any { conditions } => conditions.iter().any(|c| eval_condition_at(c, flags, now_unix)),
        Condition::CrtrStatus { id, active } => flags.has_flag(id) == *active,
        // No effects exist, so none is active ...
        Condition::EffectActive { .. } => false,
        // ... which is exactly what "inactive" asks for.
        Condition::EffectInactive { .. } => true,
        Condition::EffectTime { .. } => false,
        Condition::RtActive | Condition::CtActive => false,
        // A never-reported indicator reads false; the condition compares.
        Condition::Indicator { id: _, active } => !*active,
        Condition::Vital { vital: _, cmp, value, unit } => match unit {
            VitalUnit::Percent => cmp.eval(FRESH_VITAL_PERCENT, *value as i64),
            VitalUnit::Absolute => false,
        },
        // Absent = healthy = level 0.
        Condition::Injury { area: _, cmp, level } => cmp.eval(0, *level as i64),
        // No spell table here: every number is unknown (module docs).
        Condition::SpellAffordable { .. } => false,
        Condition::TimeOfDay { phase } => phase_at(now_unix, PhaseBoundaries::default()) == *phase,
        // Spelled per slot, as the original is: all four are empty.
        Condition::HandEmpty { hand } => match hand {
            HandSlot::Right | HandSlot::Left | HandSlot::Either | HandSlot::Spell => true,
        },
        Condition::HandHolds { .. } => false,
        Condition::SpellPrepared { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gs_field::condition::{Cmp, DayPhase, EffectCategory, NameMatch, VitalKind};

    /// 2026-08-10 20:44 UTC = 16:44 Eastern: Day.
    const A_DAY_INSTANT: i64 = 1_786_394_661;

    fn status(id: &str, active: bool) -> Condition {
        Condition::CrtrStatus { id: id.into(), active }
    }

    fn stunned_boss() -> CreatureFlags {
        CreatureFlags { hostile: true, ascension_boss: true, statuses: vec!["stunned".into()], ..Default::default() }
    }

    fn holds(cond: &Condition) -> bool {
        eval_condition_at(cond, &stunned_boss(), A_DAY_INSTANT)
    }

    // conditions.rs:166-168 -- `flags.has_flag(id) == *active`.
    #[test]
    fn crtr_status_reads_statuses_and_class_flags_in_both_senses() {
        assert!(holds(&status("stunned", true)));
        assert!(holds(&status("STUNNED", true)), "case-insensitive, like has_flag");
        assert!(holds(&status("hostile", true)));
        assert!(!holds(&status("webbed", true)));
        assert!(holds(&status("webbed", false)));
        assert!(!holds(&status("stunned", false)));
        assert!(holds(&status("dead", false)));
    }

    // conditions.rs:160-165 -- `.all(..)` / `.any(..)` over the children.
    #[test]
    fn combinators_recurse_and_follow_the_empty_rules() {
        let all = |conditions| Condition::All { conditions };
        let any = |conditions| Condition::Any { conditions };
        assert!(holds(&all(vec![])));
        assert!(!holds(&any(vec![])));
        assert!(holds(&all(vec![status("stunned", true), status("dead", false)])));
        assert!(!holds(&all(vec![status("stunned", true), status("dead", true)])));
        assert!(holds(&any(vec![status("dead", true), any(vec![status("stunned", true)])])));
        // Game-state leaves compose like any other: a false one poisons an
        // All, a true one carries an Any.
        assert!(!holds(&all(vec![status("stunned", true), Condition::RtActive])));
        assert!(holds(&any(vec![status("dead", true), Condition::HandEmpty { hand: HandSlot::Right }])));
    }

    fn effect_fields() -> (EffectCategory, String, NameMatch) {
        (EffectCategory::Buffs, "Spirit Shield".to_string(), NameMatch::Exact)
    }

    // conditions.rs:173-174 -- `effect_lookup(..).is_some_and(..)`; the
    // lookup (c.rs:348) reads `gs.effects`, empty in GameState::new()
    // (state.rs:2038) -> None -> false.
    #[test]
    fn effect_active_is_false() {
        let (category, name, name_match) = effect_fields();
        assert!(!holds(&Condition::EffectActive { category, name, name_match }));
    }

    // conditions.rs:179-180 -- `!effect_lookup(..).is_some_and(..)` -> !false.
    #[test]
    fn effect_inactive_is_true() {
        let (category, name, name_match) = effect_fields();
        assert!(holds(&Condition::EffectInactive { category, name, name_match }));
    }

    // conditions.rs:187-190 -- no effect -> `.unwrap_or(false)`, whatever
    // the comparison.
    #[test]
    fn effect_time_is_false_for_every_comparison() {
        for cmp in [Cmp::Lt, Cmp::Le, Cmp::Gt, Cmp::Ge] {
            let (category, name, name_match) = effect_fields();
            assert!(!holds(&Condition::EffectTime { category, name, name_match, cmp, seconds: 0 }), "{cmp:?}");
        }
    }

    // conditions.rs:191 -- `gs.roundtime_end.is_some_and(..)`; None at
    // state.rs:2028.
    #[test]
    fn rt_active_is_false() {
        assert!(!holds(&Condition::RtActive));
    }

    // conditions.rs:192 -- `gs.casttime_end.is_some_and(..)`; None at
    // state.rs:2029.
    #[test]
    fn ct_active_is_false() {
        assert!(!holds(&Condition::CtActive));
    }

    // conditions.rs:193-195 -- `indicator_value(gs, id).map(|v| v == *active)`;
    // indicator_value (c.rs:393-395) is `Some(gs.status.get(id))`, and
    // StatusInfo::get (state.rs:361-363) reads false for an unreported id.
    #[test]
    fn indicator_matches_only_when_it_asks_for_inactive() {
        for id in ["stunned", "STANDING", "no_such_indicator"] {
            assert!(!holds(&Condition::Indicator { id: id.into(), active: true }), "{id}");
            assert!(holds(&Condition::Indicator { id: id.into(), active: false }), "{id}");
        }
    }

    // conditions.rs:201-203 with vital_value c.rs:401-406 -- percent reads
    // `gs.vitals.*`, and Vitals::default() (state.rs:2189-2197) is 100, not
    // zero: `cmp.eval(100, value)`.
    #[test]
    fn percent_vitals_compare_against_a_full_bar() {
        let vital = |vital, cmp, value| Condition::Vital { vital, cmp, value, unit: VitalUnit::Percent };
        for kind in [VitalKind::Health, VitalKind::Mana, VitalKind::Stamina, VitalKind::Spirit] {
            assert!(!holds(&vital(kind, Cmp::Lt, 50)), "a full bar is not below 50");
            assert!(!holds(&vital(kind, Cmp::Lt, 100)));
            assert!(holds(&vital(kind, Cmp::Le, 100)));
            assert!(holds(&vital(kind, Cmp::Ge, 100)));
            assert!(holds(&vital(kind, Cmp::Gt, 99)));
            assert!(!holds(&vital(kind, Cmp::Gt, 100)));
        }
    }

    // conditions.rs:407-414 -- absolute reads minivitals and is
    // `(entry.max > 0).then_some(..)`; MiniVitalsState::default()
    // (state.rs:2083) has max 0 -> None -> `.unwrap_or(false)` (c.rs:203).
    #[test]
    fn absolute_vitals_are_false_for_every_comparison() {
        for cmp in [Cmp::Lt, Cmp::Le, Cmp::Gt, Cmp::Ge] {
            let cond = Condition::Vital { vital: VitalKind::Mana, cmp, value: 0, unit: VitalUnit::Absolute };
            assert!(!holds(&cond), "{cmp:?}");
        }
    }

    // conditions.rs:204-209 -- `gs.injuries.get(area).copied().unwrap_or(0)`
    // (empty map, state.rs:2041), then `cmp.eval(0, level)`.
    #[test]
    fn injuries_compare_against_a_healthy_level_zero() {
        let injury = |cmp, level| Condition::Injury { area: "head".into(), cmp, level };
        assert!(!holds(&injury(Cmp::Ge, 2)), "the common rule: no rank-2 wound");
        assert!(!holds(&injury(Cmp::Gt, 0)));
        assert!(holds(&injury(Cmp::Lt, 1)), "healthy IS below rank 1");
        assert!(holds(&injury(Cmp::Le, 0)));
        assert!(holds(&injury(Cmp::Ge, 0)));
    }

    // conditions.rs:210 -> spell_affordable c.rs:323-338. No spell table in
    // this workspace, so c.rs:324-326 (`let Some(spell) = .. else { return
    // false }`) is the only path. (The original is true for a known spell
    // with no static cost; see the module docs.)
    #[test]
    fn spell_affordable_is_false() {
        assert!(!holds(&Condition::SpellAffordable { number: 101 }));
        assert!(!holds(&Condition::SpellAffordable { number: 0 }));
    }

    // conditions.rs:211-219 -- `phase_at(now_server, PhaseBoundaries::default())
    // == *phase`, with now_server = Utc::now() (studio.rs:1118).
    #[test]
    fn time_of_day_follows_the_clock() {
        let at = |phase, now| eval_condition_at(&Condition::TimeOfDay { phase }, &stunned_boss(), now);
        assert!(at(DayPhase::Day, A_DAY_INSTANT));
        for other in [DayPhase::Dawn, DayPhase::Dusk, DayPhase::Night] {
            assert!(!at(other, A_DAY_INSTANT), "{other:?}");
        }
        // Eight hours on: 00:44 Eastern.
        assert!(at(DayPhase::Night, A_DAY_INSTANT + 8 * 3600));
        // The wall-clock form matches exactly one phase.
        let now_true = [DayPhase::Dawn, DayPhase::Day, DayPhase::Dusk, DayPhase::Night]
            .into_iter()
            .filter(|phase| eval_condition(&Condition::TimeOfDay { phase: *phase }, &stunned_boss()))
            .count();
        assert!(now_true <= 1, "at most one (zero only across a phase boundary mid-test)");
    }

    // conditions.rs:220-227 -- Right/Left/Either: `hand_item(..).is_none()`;
    // hand_item (c.rs:252-262) finds no GameObjects hand (default registry,
    // state.rs:2067) and `left_hand` / `right_hand` are None
    // (state.rs:2035-2036). Spell: `prepared_spell(gs).is_none()`
    // (c.rs:265-268; `spell: None`, state.rs:2030).
    #[test]
    fn hand_empty_is_true_for_every_slot() {
        for hand in [HandSlot::Right, HandSlot::Left, HandSlot::Either, HandSlot::Spell] {
            assert!(holds(&Condition::HandEmpty { hand }), "{hand:?}");
        }
    }

    // conditions.rs:228-240 -> hand_holds: `check` returns false when
    // hand_item is None (c.rs:292-294); the Spell slot needs a prepared
    // spell (c.rs:309-313). Even "neither test set = any item" is false.
    #[test]
    fn hand_holds_is_false_for_every_slot() {
        for hand in [HandSlot::Right, HandSlot::Left, HandSlot::Either, HandSlot::Spell] {
            let any_item = Condition::HandHolds { hand, item_type: None, name: None, name_match: NameMatch::Exact };
            assert!(!holds(&any_item), "{hand:?}");
            let typed = Condition::HandHolds {
                hand,
                item_type: Some("weapon".into()),
                name: Some("sword".into()),
                name_match: NameMatch::Contains,
            };
            assert!(!holds(&typed), "{hand:?}");
        }
    }

    // conditions.rs:241-244 -- `prepared_spell(gs).is_some_and(..)`; None.
    #[test]
    fn spell_prepared_is_false() {
        assert!(!holds(&Condition::SpellPrepared { name: None, name_match: NameMatch::Exact }));
        assert!(!holds(&Condition::SpellPrepared { name: Some("Spirit".into()), name_match: NameMatch::Contains }));
    }
}
