//! Room creatures as the wire describes them: [`Creature`] and its
//! structured `<crtrStatus>` snapshot [`CreatureFlags`].
//!
//! Ported from VellumFE `src/core/state.rs` (commit 60c84a79). Only the
//! creature types moved; `GameState` and everything else stays in the host.

/// A creature in the target list
#[derive(Clone, Debug)]
pub struct Creature {
    /// Creature display name
    pub name: String,
    /// Creature noun (short identifier, e.g., "hog" from "muddy hog")
    pub noun: Option<String>,
    /// Creature ID (e.g., "#146101714")
    pub id: String,
    /// Creature status parsed from the "(stunned)" text after the bold name.
    /// Legacy single-status fallback; `flags` is authoritative when present.
    pub status: Option<String>,
    /// Structured status snapshot from the `<crtrStatus>` XML tag.
    /// None when the feed hasn't sent one for this creature.
    pub flags: Option<CreatureFlags>,
}

impl Creature {
    /// Statuses to display, most reliable source first: the structured
    /// `<crtrStatus>` snapshot when present, else the legacy text-parsed
    /// status as a single entry.
    pub fn display_statuses(&self) -> Vec<String> {
        if let Some(flags) = &self.flags {
            let mut out = Vec::new();
            if flags.dead {
                out.push("dead".to_string());
            }
            // Extended feed extras lead: per-creature health percentage,
            // then the condition string, then the status flags.
            if let Some(pct) = flags.health_percent() {
                out.push(format!("{pct}%"));
            }
            if let Some(cond) = &flags.condition {
                out.push(cond.clone());
            }
            out.extend(flags.statuses.iter().cloned());
            out
        } else {
            self.status.clone().into_iter().collect()
        }
    }

    /// Appendage "creatures" — limbs that erupt from the ground and attack,
    /// as summoned by Grasp of the Dead (709) and similar. They are
    /// targetable but cannot be damaged, so target lists filter them out
    /// Lich-style to keep the list actionable. The kraken tentacles are real
    /// creatures despite matching the noun pattern.
    pub fn is_body_part(&self) -> bool {
        static BODY_PART_REGEX: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let regex = BODY_PART_REGEX.get_or_init(|| {
            regex::Regex::new(
                r"(?i)^(?:arm|appendage|claw|limb|pincer|tentacle)s?$|^(?:palpus|palpi)$",
            )
            .unwrap()
        });
        static KRAKEN_EXCEPTION: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let kraken = KRAKEN_EXCEPTION.get_or_init(|| {
            // Real creatures whose noun looks like an appendage; Lich excepts
            // all four kraken-tentacle variants (gameobj.rb / creature.rb).
            regex::Regex::new(r"(?i)(?:amaranthine|ghostly|grizzled|ancient) kraken tentacle")
                .unwrap()
        });
        self.noun
            .as_deref()
            .is_some_and(|noun| regex.is_match(noun) && !kraken.is_match(&self.name))
    }

    /// Whether this creature should be shown as a target — Lich's
    /// `valid_target?` (dead/animated/appendage exclusions), plus the
    /// user-configured `excluded_nouns`. Hostility is a *separate* gate and
    /// is deliberately not checked here. The single source of truth for the
    /// TUI, GUI, and web targets lists; keep those callers routed through it.
    pub fn is_valid_target(&self, excluded_nouns: &[String]) -> bool {
        // Dead/gone: structured <crtrStatus> flag when present, legacy text
        // status otherwise.
        if self.is_dead() {
            return false;
        }
        // Animated decoys, except "animated slush".
        let name_lower = self.name.to_ascii_lowercase();
        if name_lower.starts_with("animated") && !name_lower.starts_with("animated slush") {
            return false;
        }
        // Severed appendages (arm, tentacle, …), except the kraken variants.
        if self.is_body_part() {
            return false;
        }
        // User-configured noun exclusions (case-insensitive).
        if let Some(noun) = self.noun.as_deref() {
            let noun_lower = noun.to_ascii_lowercase();
            if excluded_nouns
                .iter()
                .any(|e| e.eq_ignore_ascii_case(&noun_lower))
            {
                return false;
            }
        }
        true
    }

    /// Dead by the structured flag, or by the legacy text status
    /// ("dead"/"gone") when no snapshot has been seen.
    pub fn is_dead(&self) -> bool {
        if let Some(flags) = &self.flags {
            return flags.dead;
        }
        self.status.as_deref().is_some_and(|s| {
            let lower = s.to_lowercase();
            lower.contains("dead") || lower.contains("gone")
        })
    }

    /// Fingerprint for widget change detection: id plus everything that
    /// affects how the entry renders.
    pub fn cache_key(&self) -> String {
        let boss_bits = self.flags.as_ref().map_or(0u8, |f| {
            (f.ascension_boss as u8) | ((f.mini_boss as u8) << 1) | ((f.challenging as u8) << 2)
        });
        format!(
            "{}:{}:{}",
            self.id,
            self.display_statuses().join("+"),
            boss_bits
        )
    }
}

/// Structured creature status from the `<crtrStatus>` XML tag (a full
/// snapshot: absent or "0" flags mean inactive, not unknown).
///
/// Two vocabularies, mirroring lich-5's split: transient combat statuses
/// (collected into `statuses` under the same canonical names the legacy
/// text parse produces) and classification flags (dedicated bools).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CreatureFlags {
    /// Active transient statuses in feed order ("stunned", "prone", ...).
    /// Open vocabulary: any unrecognized `="1"` attribute lands here too
    /// (Saga's rule), so new server effects surface without a code change.
    pub statuses: Vec<String>,
    pub hostile: bool,
    pub disengaged: bool,
    pub dead: bool,
    pub sympathetic: bool,
    pub ascended: bool,
    pub inferior: bool,
    pub ascension_boss: bool,
    pub mini_boss: bool,
    pub challenging: bool,
    pub rider: bool,
    pub mount: bool,
    /// Current hit points, when the extended feed reports them
    pub health: Option<u32>,
    /// Maximum hit points
    pub max_health: Option<u32>,
    /// True when the reporter flagged max HP as an estimate (`hpest="1"`,
    /// no bestiary template behind it) — bars render dimmed.
    pub hp_estimated: bool,
    /// Per-part wound ranks from the extended feed's `injuries` attribute
    /// (`"head:2,rightLeg:3"`). Part names are canonicalized to the doll
    /// vocabulary at parse; ranks are CritRanks R1-R3 (rank 3 on a limb is
    /// a severance — same vocabulary, no extra states).
    pub injuries: Vec<(String, u8)>,
    /// Free-text condition string, appended to the effect list by Saga
    pub condition: Option<String>,
}

/// Maps `<crtrStatus>` transient-status attribute names to the canonical
/// status names used by the text parse and the status_abbrev config
/// (e.g. the feed says "immobile", everything else says "immobilized").
pub const CRTR_STATUS_FLAGS: [(&str, &str); 12] = [
    ("immobile", "immobilized"),
    ("webbed", "webbed"),
    ("sleeping", "sleeping"),
    ("disoriented", "disoriented"),
    ("stunned", "stunned"),
    ("rooted", "rooted"),
    ("calmed", "calmed"),
    ("kneeling", "kneeling"),
    ("prone", "prone"),
    ("sitting", "sitting"),
    ("flying", "flying"),
    ("hovering", "hovering"),
];

impl CreatureFlags {
    /// Builds a snapshot from raw `<crtrStatus>` attributes (excluding
    /// `exist`). Attribute values are "1"/"0"; unknown names are ignored so
    /// new server flags degrade gracefully.
    pub fn from_xml_attrs<'a>(attrs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let mut flags = Self::default();
        for (name, value) in attrs {
            // Numeric/text extras ride alongside the "1" flags (extended
            // feed): per-creature hit points and a condition string.
            match name {
                "health" => {
                    flags.health = value.trim().parse().ok();
                    continue;
                }
                "maxhealth" => {
                    flags.max_health = value.trim().parse().ok();
                    continue;
                }
                "hpest" => {
                    flags.hp_estimated = value.trim() == "1";
                    continue;
                }
                "injuries" => {
                    // "head:2,rightLeg:3" — canonicalize parts (feet fold
                    // into legs, so keep the worse rank on collision),
                    // clamp ranks to the R1-R3 vocabulary, drop unknowns.
                    for entry in value.split(',') {
                        let Some((part, rank)) = entry.trim().split_once(':') else {
                            continue;
                        };
                        let Some(canonical) =
                            crate::cards::canonical_part(part.trim())
                        else {
                            continue;
                        };
                        let Ok(rank) = rank.trim().parse::<u8>() else {
                            continue;
                        };
                        let rank = rank.min(3);
                        if rank == 0 {
                            continue;
                        }
                        match flags.injuries.iter_mut().find(|(p, _)| p == canonical) {
                            Some((_, existing)) => *existing = (*existing).max(rank),
                            None => flags.injuries.push((canonical.to_string(), rank)),
                        }
                    }
                    continue;
                }
                "condition" => {
                    let v = value.trim();
                    if !v.is_empty() {
                        flags.condition = Some(v.to_string());
                    }
                    continue;
                }
                _ => {}
            }
            let active = value == "1";
            if !active {
                continue;
            }
            if let Some((_, canonical)) = CRTR_STATUS_FLAGS.iter().find(|(xml, _)| *xml == name) {
                flags.statuses.push(canonical.to_string());
                continue;
            }
            match name {
                "hostile" => flags.hostile = true,
                "disengaged" => flags.disengaged = true,
                "dead" => flags.dead = true,
                "sympathetic" => flags.sympathetic = true,
                "ascended" => flags.ascended = true,
                "inferior" => flags.inferior = true,
                "AscensionBoss" => flags.ascension_boss = true,
                "MiniBoss" => flags.mini_boss = true,
                "challenging" => flags.challenging = true,
                "rider" => flags.rider = true,
                "mount" => flags.mount = true,
                // Open vocabulary (Saga's rule): any other ="1" attribute
                // is an effect name - new server effects surface without a
                // client release. Logged so the creature-cards flag census
                // can enumerate what the server actually sends beyond the
                // canonical table (grep logs for "crtrStatus open-vocab").
                other => {
                    tracing::debug!("crtrStatus open-vocab flag: {other}");
                    flags.statuses.push(other.to_string());
                }
            }
        }
        flags
    }

    /// Health percentage 0..=100 when both numbers are known and sane.
    pub fn health_percent(&self) -> Option<u32> {
        let (h, m) = (self.health?, self.max_health?);
        if m == 0 {
            return None;
        }
        Some(((h * 100) / m).min(100))
    }

    /// Boss-tier creature (AscensionBoss or MiniBoss).
    pub fn is_boss(&self) -> bool {
        self.ascension_boss || self.mini_boss
    }

    /// Whether a named status/classification flag is active, for
    /// `Condition::CrtrStatus`. Accepts the canonical status names
    /// ("immobilized"), the feed's raw spellings ("immobile", "MiniBoss"),
    /// and any open-vocabulary status the server sent, case-insensitively.
    /// Unknown names read false — same fail-closed shape as indicators.
    pub fn has_flag(&self, name: &str) -> bool {
        // Feed spelling -> canonical, so authors can use either.
        let name = CRTR_STATUS_FLAGS
            .iter()
            .find(|(xml, _)| xml.eq_ignore_ascii_case(name))
            .map_or(name, |(_, canonical)| canonical);
        match name.to_ascii_lowercase().as_str() {
            "hostile" => self.hostile,
            "disengaged" => self.disengaged,
            "dead" => self.dead,
            "sympathetic" => self.sympathetic,
            "ascended" => self.ascended,
            "inferior" => self.inferior,
            "ascensionboss" => self.ascension_boss,
            "miniboss" => self.mini_boss,
            "challenging" => self.challenging,
            "rider" => self.rider,
            "mount" => self.mount,
            _ => self.statuses.iter().any(|s| s.eq_ignore_ascii_case(name)),
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    // ========== Creature::is_body_part tests ==========

    fn body_part_creature(name: &str, noun: Option<&str>) -> Creature {
        Creature {
            name: name.to_string(),
            noun: noun.map(str::to_string),
            id: "#1".to_string(),
            status: None,
            flags: None,
        }
    }

    #[test]
    fn test_is_body_part_matches_appendage_nouns() {
        for noun in [
            "arm", "arms", "tentacle", "claws", "limb", "pincer", "palpi",
        ] {
            assert!(
                body_part_creature("a severed thing", Some(noun)).is_body_part(),
                "noun '{}' should be a body part",
                noun
            );
        }
    }

    #[test]
    fn test_is_body_part_kraken_tentacle_is_a_creature() {
        // All four variants Lich excepts (gameobj.rb / creature.rb), not just
        // amaranthine — these are real creatures despite the "tentacle" noun.
        for variant in ["amaranthine", "ghostly", "grizzled", "ancient"] {
            let name = format!("a {variant} kraken tentacle");
            assert!(
                !body_part_creature(&name, Some("tentacle")).is_body_part(),
                "'{name}' should be a creature, not an appendage"
            );
        }
        // A plain tentacle with no kraken qualifier is still an appendage.
        assert!(body_part_creature("a severed tentacle", Some("tentacle")).is_body_part());
    }

    #[test]
    fn test_is_body_part_normal_creature_and_missing_noun() {
        assert!(!body_part_creature("a muddy hog", Some("hog")).is_body_part());
        assert!(!body_part_creature("a severed arm", None).is_body_part());
    }

    // ========== Creature::is_valid_target tests ==========

    #[test]
    fn test_is_valid_target_lich_valid_target_rules() {
        let excluded = vec!["coal".to_string()];

        // Creature.name holds the bold link text WITHOUT the article
        // (the feed keeps "a"/"an" outside the <a> tag), so the animated
        // check is anchored at "animated", matching Lich's /^animated\b/.

        // Live hostile-eligible creature passes.
        assert!(body_part_creature("muddy hog", Some("hog")).is_valid_target(&excluded));

        // Animated decoy fails, but "animated slush" passes.
        assert!(!body_part_creature("animated statue", Some("statue")).is_valid_target(&excluded));
        assert!(body_part_creature("animated slush", Some("slush")).is_valid_target(&excluded));

        // Appendage fails; kraken tentacle passes.
        assert!(!body_part_creature("severed arm", Some("arm")).is_valid_target(&excluded));
        assert!(
            body_part_creature("ancient kraken tentacle", Some("tentacle"))
                .is_valid_target(&excluded)
        );

        // Configured excluded noun fails (case-insensitive).
        assert!(!body_part_creature("lump of coal", Some("Coal")).is_valid_target(&excluded));

        // Dead fails.
        let dead = Creature {
            flags: Some(CreatureFlags {
                dead: true,
                ..Default::default()
            }),
            ..body_part_creature("slain orc", Some("orc"))
        };
        assert!(!dead.is_valid_target(&excluded));
    }


    // ===== from VellumFE src/parser/tests.rs (copied; originals stay with the parser) =====

    #[test]
    fn test_crtr_status_health_condition_and_open_vocab() {
        let flags = CreatureFlags::from_xml_attrs([
            ("hostile", "1"),
            ("stunned", "1"),
            ("health", "450"),
            ("maxhealth", "500"),
            ("condition", "bleeding heavily"),
            // Unknown effect name with value 1 = open vocabulary.
            ("frozen", "1"),
            // Unknown attr with a non-1 value stays ignored.
            ("mystery", "banana"),
        ]);
        assert!(flags.hostile);
        assert_eq!(flags.health, Some(450));
        assert_eq!(flags.max_health, Some(500));
        assert_eq!(flags.health_percent(), Some(90));
        assert_eq!(flags.condition.as_deref(), Some("bleeding heavily"));
        assert_eq!(
            flags.statuses,
            vec!["stunned".to_string(), "frozen".to_string()]
        );

        // maxhealth 0 or missing pieces yield no percentage.
        assert_eq!(
            CreatureFlags::from_xml_attrs([("health", "10"), ("maxhealth", "0")]).health_percent(),
            None
        );
        assert_eq!(
            CreatureFlags::from_xml_attrs([("health", "10")]).health_percent(),
            None
        );
    }

    #[test]
    fn test_crtr_status_injuries_and_hpest() {
        let flags = CreatureFlags::from_xml_attrs([
            ("health", "120"),
            ("maxhealth", "400"),
            ("hpest", "1"),
            // Feed vocabulary: nerves -> nsys, feet fold into legs (keeping
            // the worse rank), rank>3 clamps, rank 0 and unknown parts drop.
            (
                "injuries",
                "head:2,rightLeg:1,rightFoot:3,nerves:1,chest:9,tail:2,back:0",
            ),
        ]);
        assert!(flags.hp_estimated);
        assert_eq!(
            flags.injuries,
            vec![
                ("head".to_string(), 2),
                ("rightLeg".to_string(), 3),
                ("nsys".to_string(), 1),
                ("chest".to_string(), 3),
            ]
        );

        // No hpest attr = not estimated; garbage injuries parse to empty.
        let plain = CreatureFlags::from_xml_attrs([("injuries", "nonsense,also:bad:extra,:3,x:")]);
        assert!(!plain.hp_estimated);
        assert!(plain.injuries.is_empty());
    }

}
