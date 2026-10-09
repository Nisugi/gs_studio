//! What the injury doll shows of a character, as the game reports it
//! (ADDITION): wounds and scars per part, and the statuses it is under --
//! the status icons (`IconSTUNNED`...), the ones the game says only in text
//! (bound, calmed, cutthroat, silenced, sleeping, thorned) and the named
//! effects of the Debuffs list ("Major Bleed", "Staggered"...), all in one
//! vocabulary ([`Status`], `status`, shared with the creature field). A host fills a [`DollState`] from its game state
//! and hands it to the doll (`gs_field_egui::doll_view::paint_state`); how
//! each status looks is the renderer's.
//!
//! Egui-free, like the rest of the crate.

use crate::body::canonical_part;
pub use crate::status::{Status, Statuses, PRECEDENCE};

/// A character as the doll draws it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DollState {
    /// `(part, rank 1..=3)` in the doll's part names (`head`, `leftArm`,
    /// `nsys`...); feet fold into legs.
    pub wounds: Vec<(String, u8)>,
    pub scars: Vec<(String, u8)>,
    pub statuses: Statuses,
}

impl DollState {
    /// From a game state's own reports: wounds and scars per part, in any of
    /// the injury feed's spellings (`leftFoot`, `nerves`...; rank 0 and
    /// unknown parts dropped, a folded part keeps its worst rank), and the
    /// names of every status and effect active -- indicators, text
    /// statuses, debuffs -- of which those the doll draws are kept.
    pub fn from_game<'a, 'b, 'c>(
        wounds: impl IntoIterator<Item = (&'a str, u8)>,
        scars: impl IntoIterator<Item = (&'b str, u8)>,
        statuses: impl IntoIterator<Item = &'c str>,
    ) -> DollState {
        let fold = |list: &mut Vec<(String, u8)>, part: &str, rank: u8| {
            let Some(part) = canonical_part(part) else { return };
            let rank = rank.min(3);
            if rank == 0 {
                return;
            }
            match list.iter_mut().find(|(p, _)| p == part) {
                Some((_, r)) => *r = (*r).max(rank),
                None => list.push((part.to_owned(), rank)),
            }
        };
        let mut state = DollState { statuses: Statuses::from_names(statuses), ..DollState::default() };
        for (part, rank) in wounds {
            fold(&mut state.wounds, part, rank);
        }
        for (part, rank) in scars {
            fold(&mut state.scars, part, rank);
        }
        state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_game_state_folds_feet_and_drops_what_the_doll_lacks() {
        let s = DollState::from_game([("leftFoot", 2), ("leftLeg", 1), ("nerves", 3), ("tail", 2), ("head", 0)], [("rightArm", 5)], ["IconBLEEDING", "joined"]);
        assert_eq!(s.wounds, vec![("leftLeg".to_owned(), 2), ("nsys".to_owned(), 3)]);
        assert_eq!(s.scars, vec![("rightArm".to_owned(), 3)]);
        assert_eq!(s.statuses.iter().collect::<Vec<_>>(), vec![Status::Bleeding]);
    }
}
