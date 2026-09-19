//! The sandbox's condition evaluator.
//!
//! VellumFE's Stage evaluated skin `when` conditions with the client's full
//! evaluator (`core::conditions::eval_condition_for_creature`) against an
//! EMPTY `GameState`. There is no game state here at all, so this is the
//! small evaluator that gives the same answers:
//!
//! * `All` / `Any` recurse (empty `All` = true, empty `Any` = false);
//! * `CrtrStatus` tests the creature in scope: `has_flag(id) == active`;
//! * every leaf that reads game state is `false`.
//!
//! Against an empty `GameState` the original answered `true` for a few
//! game-state leaves (`EffectInactive`, `HandEmpty`, `Indicator` with
//! `active = false`, `Injury` / percent `Vital` when the comparison holds
//! for a fresh player, and `TimeOfDay` by the wall clock). Those
//! say something about a player who does not exist in a sandbox; they are
//! `false` here, by decision. The match below has no wildcard arm, so the
//! implemented / always-false split is exactly what it spells out.

use gs_field::{Condition, CreatureFlags};

/// Does `cond` hold for a creature with these flags?
pub fn eval_condition(cond: &Condition, flags: &CreatureFlags) -> bool {
    match cond {
        Condition::All { conditions } => conditions.iter().all(|c| eval_condition(c, flags)),
        Condition::Any { conditions } => conditions.iter().any(|c| eval_condition(c, flags)),
        Condition::CrtrStatus { id, active } => flags.has_flag(id) == *active,
        // Everything below reads the player's game state. No wildcard arm:
        // a new Condition variant must be sorted into one side or the other.
        Condition::EffectActive { .. }
        | Condition::EffectInactive { .. }
        | Condition::EffectTime { .. }
        | Condition::RtActive
        | Condition::CtActive
        | Condition::Indicator { .. }
        | Condition::Vital { .. }
        | Condition::Injury { .. }
        | Condition::SpellAffordable { .. }
        | Condition::TimeOfDay { .. }
        | Condition::HandEmpty { .. }
        | Condition::HandHolds { .. }
        | Condition::SpellPrepared { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(id: &str, active: bool) -> Condition {
        Condition::CrtrStatus { id: id.into(), active }
    }

    fn stunned_boss() -> CreatureFlags {
        CreatureFlags { hostile: true, ascension_boss: true, statuses: vec!["stunned".into()], ..Default::default() }
    }

    #[test]
    fn crtr_status_reads_statuses_and_class_flags_in_both_senses() {
        let flags = stunned_boss();
        assert!(eval_condition(&status("stunned", true), &flags));
        assert!(eval_condition(&status("STUNNED", true), &flags), "case-insensitive, like has_flag");
        assert!(eval_condition(&status("hostile", true), &flags));
        assert!(!eval_condition(&status("webbed", true), &flags));
        assert!(eval_condition(&status("webbed", false), &flags));
        assert!(!eval_condition(&status("stunned", false), &flags));
        assert!(eval_condition(&status("dead", false), &flags));
    }

    #[test]
    fn combinators_recurse_and_follow_the_empty_rules() {
        let flags = stunned_boss();
        let all = |conditions| Condition::All { conditions };
        let any = |conditions| Condition::Any { conditions };
        assert!(eval_condition(&all(vec![]), &flags));
        assert!(!eval_condition(&any(vec![]), &flags));
        assert!(eval_condition(&all(vec![status("stunned", true), status("dead", false)]), &flags));
        assert!(!eval_condition(&all(vec![status("stunned", true), status("dead", true)]), &flags));
        assert!(eval_condition(&any(vec![status("dead", true), any(vec![status("stunned", true)])]), &flags));
    }

    #[test]
    fn game_state_leaves_are_false_and_poison_an_all() {
        let flags = stunned_boss();
        for leaf in [
            Condition::RtActive,
            Condition::CtActive,
            Condition::Indicator { id: "stunned".into(), active: true },
            Condition::Indicator { id: "stunned".into(), active: false },
            Condition::SpellAffordable { number: 101 },
            Condition::HandEmpty { hand: Default::default() },
            Condition::SpellPrepared { name: None, name_match: Default::default() },
        ] {
            assert!(!eval_condition(&leaf, &flags), "{leaf:?}");
            let mixed = Condition::All { conditions: vec![status("stunned", true), leaf.clone()] };
            assert!(!eval_condition(&mixed, &flags), "{leaf:?}");
            let either = Condition::Any { conditions: vec![status("stunned", true), leaf] };
            assert!(eval_condition(&either, &flags));
        }
    }
}
