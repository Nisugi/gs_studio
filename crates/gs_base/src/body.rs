//! The body's parts as the game names them (ADDITION; out of `skins`,
//! `dolls`, `cards` and `condition`, which re-export them): the fourteen a
//! wound lands on, in the injury doll's label order, the injury feed's ids,
//! an outside name folded onto them, and the limbs a rank-3 wound severs.
//! Shared by the creature field and the injury doll (docs/crate-split-plan.md).

/// Every body part the game wounds, in the injury doll's label order (a
/// part's label value -- a label map's R channel -- is its place here plus
/// one), and its name in words. `skins::DOLL_PARTS` gives each its anchor on
/// a 2D doll, in this order.
pub const PARTS: [(&str, &str); 14] = [
    ("head", "head"),
    ("leftEye", "left eye"),
    ("rightEye", "right eye"),
    ("neck", "neck"),
    ("chest", "chest"),
    ("abdomen", "abdomen"),
    ("back", "back"),
    ("leftArm", "left arm"),
    ("rightArm", "right arm"),
    ("leftHand", "left hand"),
    ("rightHand", "right hand"),
    ("leftLeg", "left leg"),
    ("rightLeg", "right leg"),
    ("nsys", "nervous system"),
];

/// Body-part ids the injury feed uses (same keys as the injury doll). The
/// canonical list the parser emits on a full clear; editors offer these in a
/// dropdown for `Condition::Injury.area`.
pub const INJURY_AREAS: &[&str] = &[
    "head",
    "neck",
    "chest",
    "abdomen",
    "back",
    "leftArm",
    "rightArm",
    "leftHand",
    "rightHand",
    "leftLeg",
    "rightLeg",
    "leftEye",
    "rightEye",
    "nsys",
];

/// 1-based label value (the PNG's R channel) for a doll part key,
/// case-insensitive. `None` for names outside [`PARTS`].
pub fn part_label(part: &str) -> Option<u8> {
    PARTS.iter().position(|(key, _)| key.eq_ignore_ascii_case(part))
        .map(|i| i as u8 + 1)
}

/// The doll part key for a label value (1-based); `None` for 0 / out of range.
pub fn label_part(label: u8) -> Option<&'static str> {
    (label as usize)
        .checked_sub(1)
        .and_then(|i| PARTS.get(i))
        .map(|(key, _)| *key)
}

/// Map an external creature body-part name (CreatureBar vocabulary) onto
/// the canonical doll part key used everywhere in Vellum. Differences:
/// `nerves` -> `nsys`, and foot wounds fold into the matching leg. Canonical
/// names pass through unchanged (case-insensitively); unknown parts return
/// None and are dropped, same as the doll loader does.
pub fn canonical_part(name: &str) -> Option<&'static str> {
    let folded = match name.to_ascii_lowercase().as_str() {
        "nerves" => "nsys",
        "leftfoot" => "leftLeg",
        "rightfoot" => "rightLeg",
        other => {
            return INJURY_AREAS
                .iter()
                .find(|part| part.eq_ignore_ascii_case(other))
                .copied();
        }
    };
    Some(folded)
}

/// The limbs whose rank-3 wound is a severance (see the doc comment on
/// `CreatureFlags::injuries`), each with the parts that go with it: losing
/// an arm takes the hand along.
pub const SEVERABLE_LIMBS: &[(&str, &[&str])] = &[
    ("leftArm", &["leftArm", "leftHand"]),
    ("rightArm", &["rightArm", "rightHand"]),
    ("leftLeg", &["leftLeg"]),
    ("rightLeg", &["rightLeg"]),
];

/// The parts removed when `part` is wounded at `rank`: non-empty only for a
/// rank-3 (or worse) wound on one of [`SEVERABLE_LIMBS`].
pub fn severed_parts(part: &str, rank: u8) -> &'static [&'static str] {
    if rank < 3 {
        return &[];
    }
    SEVERABLE_LIMBS
        .iter()
        .find(|(limb, _)| limb.eq_ignore_ascii_case(part))
        .map(|(_, removed)| *removed)
        .unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The injury feed's ids are the doll's parts, in their own order.
    #[test]
    fn the_injury_areas_are_the_parts() {
        let mut a: Vec<&str> = INJURY_AREAS.to_vec();
        let mut b: Vec<&str> = PARTS.iter().map(|(key, _)| *key).collect();
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a, b);
        assert_eq!(part_label("head"), Some(1));
        assert_eq!(label_part(14), Some("nsys"));
        assert_eq!(canonical_part("LeftFoot"), Some("leftLeg"));
        assert_eq!(severed_parts("leftArm", 3), ["leftArm", "leftHand"]);
    }
}
