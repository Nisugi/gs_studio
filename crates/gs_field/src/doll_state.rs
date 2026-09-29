//! What the injury doll shows of a character, as the game reports it
//! (ADDITION): wounds and scars per part, and the statuses it is under --
//! the status icons (`IconSTUNNED`...), the ones the game says only in text
//! (bound, calmed, cutthroat, silenced, sleeping, thorned) and the named
//! effects of the Debuffs list ("Major Bleed", "Staggered"...), all in one
//! vocabulary ([`Status`]). A host fills a [`DollState`] from its game state
//! and hands it to the doll (`gs_field_egui::doll_view::paint_state`); how
//! each status looks is the renderer's.
//!
//! Egui-free, like the rest of the crate.

use crate::cards::canonical_part;

/// A status the doll draws. Several game names fold into one
/// ([`Status::from_name`]): what they look like is the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Status {
    // --- how it stands (a pose of its own)
    Kneeling,
    Sitting,
    Prone,
    Stunned,
    Sleeping,
    /// Unconscious: lying still, no dreams over it.
    Unconscious,
    Webbed,
    Dead,
    // --- on the body
    /// Frozen: the ice look, as it has always been drawn.
    Frozen,
    /// Immobile: held where it stands (no breath, no sway).
    Immobile,
    Poisoned,
    Diseased,
    Bleeding,
    Rooted,
    /// Bound by an unseen force (Bind): bands of light.
    Bound,
    /// Hidden or shrouded: in shadow.
    Hidden,
    Invisible,
    Possessed,
    Blinded,
    /// Throat cut: a cut across it, and no voice.
    Cutthroat,
    Silenced,
    // --- how it holds itself
    Calmed,
    /// Terrified, in sheer fear, horrified: cowering.
    Terrified,
    Demoralized,
    /// Feeble or weakened: sagging.
    Feeble,
    Crippled,
    Staggered,
    /// Dazed: reeling.
    Dazed,
    /// Confused or disoriented: reeling, not knowing where it is.
    Confused,
    // --- how it breathes
    Frenzied,
    Overexerted,
    Slowed,
    Vulnerable,
}

impl Status {
    /// Every one, in order.
    pub const ALL: [Status; 33] = [
        Status::Kneeling,
        Status::Sitting,
        Status::Prone,
        Status::Stunned,
        Status::Sleeping,
        Status::Unconscious,
        Status::Webbed,
        Status::Dead,
        Status::Frozen,
        Status::Immobile,
        Status::Poisoned,
        Status::Diseased,
        Status::Bleeding,
        Status::Rooted,
        Status::Bound,
        Status::Hidden,
        Status::Invisible,
        Status::Possessed,
        Status::Blinded,
        Status::Cutthroat,
        Status::Silenced,
        Status::Calmed,
        Status::Terrified,
        Status::Demoralized,
        Status::Feeble,
        Status::Crippled,
        Status::Staggered,
        Status::Dazed,
        Status::Confused,
        Status::Frenzied,
        Status::Overexerted,
        Status::Slowed,
        Status::Vulnerable,
    ];

    /// Its id: lowercase, one word.
    pub const fn id(self) -> &'static str {
        match self {
            Status::Kneeling => "kneeling",
            Status::Sitting => "sitting",
            Status::Prone => "prone",
            Status::Stunned => "stunned",
            Status::Sleeping => "sleeping",
            Status::Unconscious => "unconscious",
            Status::Webbed => "webbed",
            Status::Dead => "dead",
            Status::Frozen => "frozen",
            Status::Immobile => "immobile",
            Status::Poisoned => "poisoned",
            Status::Diseased => "diseased",
            Status::Bleeding => "bleeding",
            Status::Rooted => "rooted",
            Status::Bound => "bound",
            Status::Hidden => "hidden",
            Status::Invisible => "invisible",
            Status::Possessed => "possessed",
            Status::Blinded => "blinded",
            Status::Cutthroat => "cutthroat",
            Status::Silenced => "silenced",
            Status::Calmed => "calmed",
            Status::Terrified => "terrified",
            Status::Demoralized => "demoralized",
            Status::Feeble => "feeble",
            Status::Crippled => "crippled",
            Status::Staggered => "staggered",
            Status::Dazed => "dazed",
            Status::Confused => "confused",
            Status::Frenzied => "frenzied",
            Status::Overexerted => "overexerted",
            Status::Slowed => "slowed",
            Status::Vulnerable => "vulnerable",
        }
    }

    /// The status a game name means, if the doll draws it: an indicator id
    /// (`IconPOISONED`, `poisoned`), a text status (`thorned`), a creature
    /// flag (`immobile`, `disoriented`) or an effect's name (`Major Bleed`,
    /// `Sheer Fear`, `Disoriented (status)`), any case. None for what has
    /// no look (standing, joined, roundtime, a parry...).
    pub fn from_name(name: &str) -> Option<Status> {
        let n = name.trim();
        let n = n.strip_prefix("Icon").unwrap_or(n);
        let n = n.to_ascii_lowercase();
        let n = n.trim_end_matches("(status)").trim().replace(['_', '-'], " ");
        Some(match n.as_str() {
            "kneeling" | "kneel" => Status::Kneeling,
            "sitting" | "sit" => Status::Sitting,
            "prone" | "lying down" => Status::Prone,
            "stunned" | "stun" => Status::Stunned,
            "sleeping" | "asleep" | "sleep" => Status::Sleeping,
            "unconscious" => Status::Unconscious,
            "webbed" | "web" => Status::Webbed,
            "dead" => Status::Dead,
            "frozen" => Status::Frozen,
            "immobile" | "immobilized" => Status::Immobile,
            "poisoned" | "poison" | "major poison" | "thorned" | "thorn poison" => Status::Poisoned,
            "diseased" | "disease" | "mind rot" => Status::Diseased,
            "bleeding" | "bleed" | "major bleed" => Status::Bleeding,
            "rooted" => Status::Rooted,
            "bound" | "bind" => Status::Bound,
            "hidden" | "shrouded" => Status::Hidden,
            "invisible" => Status::Invisible,
            "possessed" => Status::Possessed,
            "blinded" | "blind" => Status::Blinded,
            "cutthroat" => Status::Cutthroat,
            "silenced" => Status::Silenced,
            "calmed" | "calm" => Status::Calmed,
            "terrified" | "sheer fear" | "horrify" | "horrified" | "frightened" => Status::Terrified,
            "demoralized" => Status::Demoralized,
            "feeble" | "weakened" => Status::Feeble,
            "crippled" => Status::Crippled,
            "staggered" => Status::Staggered,
            "dazed" => Status::Dazed,
            "confused" | "disoriented" => Status::Confused,
            "frenzied" | "frenzy" => Status::Frenzied,
            "overexerted" => Status::Overexerted,
            "slowed" => Status::Slowed,
            "vulnerable" => Status::Vulnerable,
            _ => return None,
        })
    }
}

/// A set of [`Status`]es: small, `Copy`, hashable (a puppet key holds it).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Statuses(u64);

impl Statuses {
    pub const NONE: Statuses = Statuses(0);

    pub fn has(self, s: Status) -> bool {
        self.0 & (1 << s as u8) != 0
    }

    pub fn with(mut self, s: Status) -> Self {
        self.insert(s);
        self
    }

    pub fn insert(&mut self, s: Status) {
        self.0 |= 1 << s as u8;
    }

    pub fn set(&mut self, s: Status, on: bool) {
        if on {
            self.0 |= 1 << s as u8;
        } else {
            self.0 &= !(1 << s as u8);
        }
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn iter(self) -> impl Iterator<Item = Status> {
        Status::ALL.into_iter().filter(move |s| self.has(*s))
    }

    /// The statuses these game names mean ([`Status::from_name`]); names
    /// with no look are skipped.
    pub fn from_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Self {
        let mut out = Statuses::NONE;
        for n in names {
            if let Some(s) = Status::from_name(n) {
                out.insert(s);
            }
        }
        out
    }

    /// Whether a game name's status is in the set (an unknown name: no).
    pub fn has_name(self, name: &str) -> bool {
        Status::from_name(name).is_some_and(|s| self.has(s))
    }
}

impl FromIterator<Status> for Statuses {
    fn from_iter<I: IntoIterator<Item = Status>>(iter: I) -> Self {
        let mut out = Statuses::NONE;
        for s in iter {
            out.insert(s);
        }
        out
    }
}

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
    fn game_names_fold_into_the_doll_s_statuses() {
        assert_eq!(Status::from_name("IconPOISONED"), Some(Status::Poisoned));
        assert_eq!(Status::from_name("thorned"), Some(Status::Poisoned));
        assert_eq!(Status::from_name("Major Bleed"), Some(Status::Bleeding));
        assert_eq!(Status::from_name("Sheer Fear"), Some(Status::Terrified));
        assert_eq!(Status::from_name("Disoriented (status)"), Some(Status::Confused));
        assert_eq!(Status::from_name("immobile"), Some(Status::Immobile));
        assert_eq!(Status::from_name("frozen"), Some(Status::Frozen));
        assert_eq!(Status::from_name("Mind Rot"), Some(Status::Diseased));
        assert_eq!(Status::from_name("IconSTANDING"), None);
        assert_eq!(Status::from_name("Recent Parry"), None);
        for s in Status::ALL {
            assert_eq!(Status::from_name(s.id()), Some(s), "{s:?} names itself");
        }
    }

    #[test]
    fn a_set_holds_what_it_is_given() {
        let set = Statuses::from_names(["IconWEBBED", "calmed", "Roundtime"]);
        assert!(set.has(Status::Webbed) && set.has(Status::Calmed) && !set.has(Status::Stunned));
        assert_eq!(set.iter().count(), 2);
        let mut set = set;
        set.set(Status::Webbed, false);
        assert_eq!(set.iter().collect::<Vec<_>>(), vec![Status::Calmed]);
    }

    #[test]
    fn a_game_state_folds_feet_and_drops_what_the_doll_lacks() {
        let s = DollState::from_game([("leftFoot", 2), ("leftLeg", 1), ("nerves", 3), ("tail", 2), ("head", 0)], [("rightArm", 5)], ["IconBLEEDING", "joined"]);
        assert_eq!(s.wounds, vec![("leftLeg".to_owned(), 2), ("nsys".to_owned(), 3)]);
        assert_eq!(s.scars, vec![("rightArm".to_owned(), 3)]);
        assert_eq!(s.statuses.iter().collect::<Vec<_>>(), vec![Status::Bleeding]);
    }
}
