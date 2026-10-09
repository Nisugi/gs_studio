//! The statuses a character or a creature is under, in one vocabulary
//! (ADDITION; out of `doll_state`, which re-exports it): the status icons,
//! the ones the game says only in text and the named effects of the Debuffs
//! list fold into [`Status`] ([`Status::from_name`]); a [`Statuses`] set is
//! small and hashable. Shared by the injury doll and the creature field
//! (docs/crate-split-plan.md).

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

/// `(winner, loser)`: when both are on, the loser is not drawn
/// ([`Statuses::resolved`]).
pub const PRECEDENCE: [(Status, Status); 1] = [(Status::Staggered, Status::Calmed)];

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

    /// The set as the doll draws it: where two statuses would fight over
    /// the same look, the one that wins, the other dropped -- staggered
    /// over calmed (off balance is not at ease).
    pub fn resolved(self) -> Self {
        let mut out = self;
        for (winner, loser) in PRECEDENCE {
            if out.has(winner) {
                out.set(loser, false);
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
    fn staggered_wins_over_calmed() {
        let both = Statuses::NONE.with(Status::Staggered).with(Status::Calmed);
        assert_eq!(both.resolved(), Statuses::NONE.with(Status::Staggered));
        let calm = Statuses::NONE.with(Status::Calmed);
        assert_eq!(calm.resolved(), calm);
    }
}
