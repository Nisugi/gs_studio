//! The cast list: what can be spawned, from the creature pool's base art,
//! grouped by bestiary area. Ported from VellumFE
//! `src/frontend/gui/studio.rs` (`CastEntry`, `build_cast`) at 60c84a79.

use std::collections::{BTreeMap, BTreeSet};

/// One castable pool-art entry: a base image's token, humanized.
#[derive(Debug, Clone, PartialEq)]
pub struct CastEntry {
    /// Display name, '_' -> ' ' (name_token slugs it back to the art).
    pub display: String,
    /// Last word of the display name.
    pub noun: String,
    /// Bestiary areas (empty = unknown; grouped under "Other").
    pub areas: Vec<String>,
}

/// Distinct castable base tokens from `(stem, set folder)` pairs: a BASE
/// image is flat `creatures/<stem>.png`, or one whose stem equals its parent
/// folder (noun or variant folder). `default`/`status` folders are reserved;
/// `{token}_`-suffixed extras (prone/wounds) fail the stem==folder test.
pub fn castable_tokens<'a>(images: impl IntoIterator<Item = (&'a str, Option<&'a str>)>) -> BTreeSet<String> {
    let mut tokens = BTreeSet::new();
    for (stem, set) in images {
        if let Some(set) = set {
            if set.split('/').any(|seg| seg == "default" || seg == "status") {
                continue;
            }
            let parent = set.rsplit('/').next().unwrap_or(set);
            if stem != parent {
                continue;
            }
        }
        if !stem.is_empty() {
            tokens.insert(stem.to_string());
        }
    }
    // Tier extras beside a flat base ({token}_prone, {token}_chest2, ...)
    // are pose/wound art, not castable creatures: drop any stem that
    // extends another stem with an underscore suffix. (Spawning "coyote
    // prone" from coyote_prone.png cast the pose image as its own
    // full-height creature.)
    let extras: Vec<String> =
        tokens.iter().filter(|t| t.rmatch_indices('_').any(|(i, _)| tokens.contains(&t[..i]))).cloned().collect();
    for extra in extras {
        tokens.remove(&extra);
    }
    tokens
}

/// A token as a cast entry; `areas_for` looks the display name up.
pub fn entry_for(token: &str, areas_for: impl Fn(&str) -> Vec<String>) -> CastEntry {
    let display = token.replace('_', " ");
    let noun = display.rsplit(' ').next().unwrap_or(&display).to_string();
    let areas = areas_for(&display);
    CastEntry { display, noun, areas }
}

/// Scan the creature pool (under gs_field's base dir) into the cast list.
pub fn build_cast() -> Vec<CastEntry> {
    let images = gs_field::pool::list_creature_images();
    let tokens = castable_tokens(images.iter().map(|i| (i.stem(), i.set.as_deref())));
    let db = gs_field::bestiary::shared();
    tokens.iter().map(|token| entry_for(token, |name| db.areas_for_name(name))).collect()
}

// ------------------------------------------------- spawning without art

/// One bestiary creature offered by the spawn-by-name box: a real creature
/// the pool may have no art for. Sprite art beats the family puppet, so the
/// only way to SEE a canine / elephantid / suine puppet in the app -- short
/// of emptying the pool -- is to spawn something that has none.
#[derive(Debug, Clone, PartialEq)]
pub struct BestiaryPick {
    pub name: String,
    pub noun: String,
    /// Bestiary family ("canine", "humanoid"), for the hint text.
    pub family: Option<String>,
}

/// Bestiary entries whose name or noun contains `query` (case-insensitive),
/// nearest first: exact name, then name prefix, then the rest, capped at
/// `limit`. An empty query offers nothing (the list is thousands long).
pub fn bestiary_matches(db: &gs_field::bestiary::BestiaryDb, query: &str, limit: usize) -> Vec<BestiaryPick> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut ranked: Vec<(u8, &gs_field::bestiary::CreatureEntry)> = db
        .entries
        .iter()
        .filter_map(|e| {
            let name = e.name.to_ascii_lowercase();
            let rank = if name == q {
                0
            } else if name.starts_with(&q) {
                1
            } else if name.contains(&q) || e.noun.to_ascii_lowercase().contains(&q) {
                2
            } else {
                return None;
            };
            Some((rank, e))
        })
        .collect();
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
    ranked.dedup_by(|a, b| a.1.name == b.1.name);
    ranked
        .into_iter()
        .take(limit)
        .map(|(_, e)| BestiaryPick {
            name: e.name.clone(),
            noun: if e.noun.trim().is_empty() { last_word(&e.name) } else { e.noun.clone() },
            family: e.family.clone().filter(|f| !f.trim().is_empty()),
        })
        .collect()
}

/// [`bestiary_matches`] against the shared database.
pub fn suggest(query: &str, limit: usize) -> Vec<BestiaryPick> {
    bestiary_matches(gs_field::bestiary::shared(), query, limit)
}

/// The noun a free-typed name spawns with: its last word.
pub fn last_word(name: &str) -> String {
    name.trim().rsplit(|c: char| c.is_whitespace()).find(|w| !w.is_empty()).unwrap_or("").to_string()
}

/// Does this creature draw as SPRITE art? The pool must hold base art for
/// it -- the same resolution the renderer's art store runs
/// (`resolve_tier_art` against the pool root and the default card skin) --
/// and no puppet may come first, by the renderer's order
/// (`CreatureArtCache::prepare_creature_art`): a puppet in a skin of its
/// own (or its named form's sheet) beats any sprite, and any puppet beats
/// the pool's `creatures/default` stand-in. So a "drawing as a puppet" hint
/// in the UI does not disagree with the card. (The renderer also refuses a
/// skin not laid out for its figure, and draws the sprite; this does not
/// decode skins to know.)
pub fn has_art(name: &str, noun: Option<&str>) -> bool {
    let Ok(root) = gs_field::data_dirs::global_images_dir() else {
        return false;
    };
    let family = noun.and_then(gs_field::cards::family_for_noun);
    let skin = gs_field::skins::CreatureCardSkin::default();
    let Some(tier) = gs_field::cards::resolve_tier_art(&root, &skin, Some(name), noun, family.as_deref()) else {
        return false;
    };
    let set = gs_field::puppets::puppets();
    let Some(puppet) = set.puppet_for(name, noun) else {
        return true;
    };
    let own_skin = set.skin(&puppet.family, &creature_token(name)).is_some();
    let named_form = puppet.tier == gs_field::dolls::DollTier::Form && set.get(&puppet.family).is_some_and(|rig| rig.texture.is_some());
    tier.token != "default" && !own_skin && !named_form
}

/// A creature's bestiary name token, the key skins are filed under: its
/// bestiary entry's (exact name, else a lone noun), else its own -- as the
/// renderer keys them (`creature_puppet::creature_key`).
fn creature_token(name: &str) -> String {
    let canonical = gs_field::naming::canonical_name(name);
    match gs_field::bestiary::shared().entry_for_name(&canonical) {
        Some(entry) => gs_field::naming::name_token(&entry.name),
        None => gs_field::naming::name_token(&canonical),
    }
}

/// The family puppet a creature with no art falls back to, if any.
pub fn puppet_family(name: &str, noun: Option<&str>) -> Option<String> {
    gs_field::puppets::puppet_for(name, noun).map(|d| d.family)
}

/// A creature to show `family`'s puppet with: any bestiary creature whose
/// puppet resolves to that family. The bestiary's family names are not the
/// rig families (birds are "Bird" for the `avian` rig, bears "Bear" for
/// `ursine`, and snakes reach `serpent` only by body type), so the puppet
/// lookup itself is the test. Preferred, in order: a creature whose bestiary
/// family names the rig (directly or through `FAMILY_ALIASES`), one with NO
/// art in the pool (so it needs no forcing anywhere), then by name, so the
/// pick is stable. Err says why none will do.
#[cfg(test)]
pub fn puppet_showcase_in(
    db: &gs_field::bestiary::BestiaryDb,
    family: &str,
    has_art: impl Fn(&str, &str) -> bool,
    puppet_of: impl Fn(&str, &str) -> Option<String>,
) -> Result<BestiaryPick, String> {
    puppet_showcase_bare_in(db, family, has_art, |_, _| false, puppet_of)
}

/// [`puppet_showcase_in`] that also steers clear of creatures with a skin of
/// their own (`has_skin(name, noun)`), so the pick wears the form's bare
/// sheet: the Puppets list's per-texture rows spawn the skinned creatures
/// by name, and this is the row for the sheet itself.
pub fn puppet_showcase_bare_in(
    db: &gs_field::bestiary::BestiaryDb,
    family: &str,
    has_art: impl Fn(&str, &str) -> bool,
    has_skin: impl Fn(&str, &str) -> bool,
    puppet_of: impl Fn(&str, &str) -> Option<String>,
) -> Result<BestiaryPick, String> {
    let names_rig = |f: &str| {
        let f = f.trim();
        f.eq_ignore_ascii_case(family)
            || gs_field::dolls::rig_family_for(&f.to_ascii_lowercase()).is_some_and(|r| r.eq_ignore_ascii_case(family))
    };
    let mut picks: Vec<(bool, bool, bool, BestiaryPick)> = db
        .entries
        .iter()
        .map(|e| BestiaryPick {
            name: e.name.clone(),
            noun: if e.noun.trim().is_empty() { last_word(&e.name) } else { e.noun.clone() },
            family: e.family.clone(),
        })
        .filter(|p| puppet_of(&p.name, &p.noun).is_some_and(|f| f.eq_ignore_ascii_case(family)))
        .map(|p| (has_skin(&p.name, &p.noun), !p.family.as_deref().is_some_and(names_rig), has_art(&p.name, &p.noun), p))
        .collect();
    picks.sort_by(|a, b| (a.0, a.1, a.2, &a.3.name).cmp(&(b.0, b.1, b.2, &b.3.name)));
    picks
        .into_iter()
        .next()
        .map(|(_, _, _, p)| p)
        .ok_or_else(|| format!("no creature in the bestiary resolves to the {family} puppet"))
}

/// [`puppet_showcase_in`] against the shared bestiary, the pool and the
/// published puppets.
pub fn puppet_showcase(family: &str) -> Result<BestiaryPick, String> {
    let set = gs_field::puppets::puppets();
    puppet_showcase_bare_in(
        gs_field::bestiary::shared(),
        family,
        |n, noun| has_art(n, Some(noun)),
        |n, _| set.skin(family, &gs_field::naming::name_token(n)).is_some(),
        |n, noun| puppet_family(n, Some(noun)),
    )
}

/// One texture a form's puppet can be shown in: the form's own sheet
/// (`creature` = None: spawn via [`puppet_showcase`]) or a creature skin
/// (`creature` = the bestiary creature the skin belongs to, name and noun;
/// a skin the bestiary does not know spawns under its token, spaced).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PuppetTexture {
    pub label: String,
    pub creature: Option<(String, String)>,
}

/// One published form (`<form>.rig.toml`, `family` in the code) with the
/// textures it can wear.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PuppetForm {
    pub form: String,
    pub body_type: String,
    /// From the pool's `global/puppets/` rather than built in.
    pub pool: bool,
    /// The form ships a sheet of its own (`<form>.png`).
    pub sheet: bool,
    pub textures: Vec<PuppetTexture>,
}

/// The published forms grouped by bestiary family: the family most of the
/// creatures drawing with that form belong to ("Other" when none does).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PuppetFamily {
    pub family: String,
    pub forms: Vec<PuppetForm>,
}

/// The Puppets list: family -> form -> texture. `forms` is every published
/// form as (form, body type, from the pool, ships a sheet); `skins` every
/// creature skin as (form, creature token); `puppet_of` resolves a bestiary
/// creature (name, noun) to its form.
pub fn puppet_tree_in(
    db: &gs_field::bestiary::BestiaryDb,
    forms: &[(String, String, bool, bool)],
    skins: &[(String, String)],
    puppet_of: impl Fn(&str, &str) -> Option<String>,
) -> Vec<PuppetFamily> {
    // Who draws with which form, and the family they say they are.
    let mut families: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let mut by_token: BTreeMap<String, (String, String)> = BTreeMap::new();
    for e in &db.entries {
        let noun = if e.noun.trim().is_empty() { last_word(&e.name) } else { e.noun.clone() };
        by_token.entry(gs_field::naming::name_token(&e.name)).or_insert_with(|| (e.name.clone(), noun.clone()));
        if let Some(form) = puppet_of(&e.name, &noun) {
            let family = e.family.as_deref().map(str::trim).filter(|f| !f.is_empty()).unwrap_or("Other");
            *families.entry(form.to_ascii_lowercase()).or_default().entry(family.to_string()).or_default() += 1;
        }
    }
    let mut tree: BTreeMap<String, Vec<PuppetForm>> = BTreeMap::new();
    for (form, body_type, pool, sheet) in forms {
        let family = families
            .get(&form.to_ascii_lowercase())
            .and_then(|counts| counts.iter().max_by_key(|(name, n)| (**n, std::cmp::Reverse((*name).clone()))))
            .map(|(name, _)| name.clone())
            .unwrap_or_else(|| "Other".to_string());
        let mut textures = Vec::new();
        if *sheet {
            textures.push(PuppetTexture { label: format!("{form} sheet"), creature: None });
        }
        for (_, token) in skins.iter().filter(|(f, _)| f.eq_ignore_ascii_case(form)) {
            let (name, noun) = by_token
                .get(token)
                .cloned()
                .unwrap_or_else(|| (token.replace('_', " "), last_word(&token.replace('_', " "))));
            textures.push(PuppetTexture { label: name.clone(), creature: Some((name, noun)) });
        }
        tree.entry(family).or_default().push(PuppetForm {
            form: form.clone(),
            body_type: body_type.clone(),
            pool: *pool,
            sheet: *sheet,
            textures,
        });
    }
    // "Other" last, the rest by name.
    let mut out: Vec<PuppetFamily> = tree.into_iter().map(|(family, forms)| PuppetFamily { family, forms }).collect();
    out.sort_by_key(|f| (f.family == "Other", f.family.to_ascii_lowercase()));
    out
}

/// [`puppet_tree_in`] against the shared bestiary and the published puppets.
pub fn puppet_tree() -> Vec<PuppetFamily> {
    let set = gs_field::puppets::puppets();
    let forms: Vec<(String, String, bool, bool)> = set
        .families()
        .filter_map(|f| {
            let rig = set.get(f)?;
            Some((f.to_string(), rig.body_type.clone(), matches!(rig.source, gs_field::puppets::PuppetSource::Override), rig.texture.is_some()))
        })
        .collect();
    let skins: Vec<(String, String)> = set.skins().map(|(f, c)| (f.to_string(), c.to_string())).collect();
    puppet_tree_in(gs_field::bestiary::shared(), &forms, &skins, |n, noun| puppet_family(n, Some(noun)))
}

/// Everything the spawn-by-name box shows for what is typed: whether the
/// name itself has art (else which puppet stands in), and the bestiary
/// suggestions, each with its own art answer. Every one of those is a trip
/// to the filesystem, so the tool computes this once per typed name
/// (`fs_cache`), not per frame.
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnHints {
    pub has_art: bool,
    pub puppet: Option<String>,
    pub picks: Vec<(BestiaryPick, bool)>,
}

pub fn spawn_hints(typed: &str, limit: usize) -> SpawnHints {
    let noun = last_word(typed);
    let noun = (!noun.is_empty()).then_some(noun);
    let has_art = has_art(typed, noun.as_deref());
    SpawnHints {
        has_art,
        puppet: (!has_art).then(|| puppet_family(typed, noun.as_deref())).flatten(),
        picks: suggest(typed, limit)
            .into_iter()
            .map(|p| {
                let art = has_art_for(&p);
                (p, art)
            })
            .collect(),
    }
}

fn has_art_for(pick: &BestiaryPick) -> bool {
    has_art(&pick.name, Some(&pick.noun))
}

/// Grouped by bestiary area (multi-area creatures list under each;
/// unknowns under "Other").
pub fn by_area(cast: &[CastEntry]) -> BTreeMap<&str, Vec<&CastEntry>> {
    let mut by_area: BTreeMap<&str, Vec<&CastEntry>> = BTreeMap::new();
    for entry in cast {
        if entry.areas.is_empty() {
            by_area.entry("Other").or_default().push(entry);
        } else {
            for area in &entry.areas {
                by_area.entry(area.as_str()).or_default().push(entry);
            }
        }
    }
    by_area
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bases_are_cast_and_pose_wound_status_art_is_not() {
        let tokens = castable_tokens([
            ("coyote", None),
            ("coyote_prone", None),
            ("coyote_chest2", None),
            ("forest_troll", None),
            ("kobold", Some("kobold")),
            ("kobold_prone", Some("kobold")),
            ("shaman", Some("kobold/shaman")),
            ("stunned", Some("status")),
            ("kobold", Some("default")),
            ("webbed", Some("kobold/status")),
            ("", None),
        ]);
        let tokens: Vec<&str> = tokens.iter().map(String::as_str).collect();
        assert_eq!(tokens, ["coyote", "forest_troll", "kobold", "shaman"]);
    }

    #[test]
    fn an_underscore_name_survives_unless_its_prefix_is_a_base() {
        // forest_troll is only an "extra" if a base called "forest" exists.
        assert!(castable_tokens([("forest_troll", None)]).contains("forest_troll"));
        assert!(!castable_tokens([("forest_troll", None), ("forest", None)]).contains("forest_troll"));
    }

    #[test]
    fn entries_humanize_and_group_by_area() {
        let areas = |name: &str| {
            if name == "forest troll" {
                vec!["Glatoph".to_string(), "Zeltoph".to_string()]
            } else {
                vec![]
            }
        };
        let cast = vec![entry_for("forest_troll", areas), entry_for("kobold", areas)];
        assert_eq!((cast[0].display.as_str(), cast[0].noun.as_str()), ("forest troll", "troll"));
        assert_eq!((cast[1].display.as_str(), cast[1].noun.as_str()), ("kobold", "kobold"));
        let grouped = by_area(&cast);
        assert_eq!(grouped.keys().copied().collect::<Vec<_>>(), ["Glatoph", "Other", "Zeltoph"]);
        assert_eq!(grouped["Other"][0].display, "kobold");
    }

    #[test]
    fn suggestions_rank_exact_then_prefix_then_substring() {
        use gs_field::bestiary::{BestiaryDb, CreatureEntry};
        let e = |name: &str, noun: &str, family: &str| CreatureEntry {
            name: name.into(),
            noun: noun.into(),
            family: (!family.is_empty()).then(|| family.to_string()),
            ..Default::default()
        };
        let db = BestiaryDb::from_entries(vec![
            e("timber wolf", "wolf", "canine"),
            e("wolf", "wolf", "canine"),
            e("wolf spider", "spider", "arachnid"),
            e("cave troll", "troll", "humanoid"),
        ]);
        let names: Vec<String> = bestiary_matches(&db, " WOLF ", 10).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["wolf", "wolf spider", "timber wolf"]);
        assert_eq!(bestiary_matches(&db, "wolf", 2).len(), 2, "capped");
        assert!(bestiary_matches(&db, "   ", 10).is_empty(), "an empty query offers nothing");
        assert!(bestiary_matches(&db, "zzz", 10).is_empty());
        let pick = &bestiary_matches(&db, "timber", 10)[0];
        assert_eq!((pick.noun.as_str(), pick.family.as_deref()), ("wolf", Some("canine")));

        // A blank noun in the file falls back to the name's last word.
        let db = BestiaryDb::from_entries(vec![e("gnarled hinterboar", "", "")]);
        let pick = &bestiary_matches(&db, "hinter", 10)[0];
        assert_eq!((pick.noun.as_str(), pick.family.as_deref()), ("hinterboar", None));
    }

    #[test]
    fn a_puppet_showcase_is_an_artless_creature_whose_puppet_is_that_family() {
        use gs_field::bestiary::{BestiaryDb, CreatureEntry};
        let e = |name: &str, family: &str| CreatureEntry {
            name: name.into(),
            noun: last_word(name),
            family: Some(family.into()),
            ..Default::default()
        };
        let db = BestiaryDb::from_entries(vec![
            e("wolf", "canine"),
            e("coyote", "canine"),
            e("timber wolf", "canine"),
            e("mastodon", "Elephantid"),
        ]);
        let canine = |_: &str, _: &str| Some("canine".to_string());
        // Sorted by name; the coyote has art, so the next one shows the puppet.
        let pick = puppet_showcase_in(&db, "canine", |n, _| n == "coyote", canine).unwrap();
        assert_eq!((pick.name.as_str(), pick.noun.as_str()), ("timber wolf", "wolf"));
        // Family names match in any case.
        let pick = puppet_showcase_in(&db, "elephantid", |_, _| false, |_, _| Some("elephantid".into())).unwrap();
        assert_eq!(pick.name, "mastodon");
        // Every one has art: still a pick (the stage forces the puppet).
        assert_eq!(puppet_showcase_in(&db, "canine", |_, _| true, canine).unwrap().name, "coyote");
        assert!(puppet_showcase_in(&db, "suine", |_, _| false, canine).unwrap_err().contains("no creature"));
        // A creature whose puppet resolves elsewhere does not show this one.
        assert!(puppet_showcase_in(&db, "canine", |_, _| false, |_, _| Some("humanoid".into())).is_err());

        // The bestiary spells rig families its own way: "Bird" is the avian
        // rig, and a snake reaches the serpent puppet only by body type.
        let db = BestiaryDb::from_entries(vec![e("blood eagle", "Bird"), e("viper", "Reptilian"), e("cave lizard", "Reptilian")]);
        let by_name = |n: &str, _: &str| {
            Some(match n {
                "blood eagle" => "avian",
                "viper" => "serpent",
                _ => "reptilian",
            }
            .to_string())
        };
        assert_eq!(puppet_showcase_in(&db, "avian", |_, _| false, by_name).unwrap().name, "blood eagle");
        assert_eq!(puppet_showcase_in(&db, "serpent", |_, _| false, by_name).unwrap().name, "viper");
        // A creature of the rig's own family beats an art-less stranger.
        let db = BestiaryDb::from_entries(vec![e("aaa hawk", "Chimeric"), e("zzz hawk", "Bird")]);
        let both_avian = |_: &str, _: &str| Some("avian".to_string());
        let pick = puppet_showcase_in(&db, "avian", |n, _| n == "zzz hawk", both_avian).unwrap();
        assert_eq!(pick.name, "zzz hawk");
    }

    #[test]
    fn a_free_typed_name_spawns_with_its_last_word_as_the_noun() {
        assert_eq!(last_word("  a large timber wolf "), "wolf");
        assert_eq!(last_word("kobold"), "kobold");
        assert_eq!(last_word("   "), "");
    }

    /// The hint the spawn box shows must match what actually draws: with an
    /// empty pool nothing has art, and the family puppet is what stands in.
    #[test]
    fn with_an_empty_pool_nothing_has_art_and_a_puppet_stands_in() {
        let _base = crate::stage::test_support::TempBase::new();
        assert!(!has_art("timber wolf", Some("wolf")));
        assert!(!has_art("training dummy", Some("dummy")));
        // The doll cascade run with "has a puppet" as the test: the
        // humanoid puppet is the generic fallback, so SOMETHING always
        // stands in for a creature with no sprite.
        assert_eq!(puppet_family("training dummy", Some("dummy")).as_deref(), Some("humanoid"));
    }

    /// ... and art in the pool wins, which is why the owner never saw a
    /// puppet: every Cast entry is built FROM the art pool.
    #[test]
    fn art_in_the_pool_is_found_by_the_same_resolver_the_renderer_uses() {
        let _base = crate::stage::test_support::TempBase::new();
        let creatures = gs_field::data_dirs::global_image_category_dir("creatures").unwrap();
        std::fs::create_dir_all(&creatures).unwrap();
        std::fs::write(creatures.join("timber_wolf.png"), b"not really a png").unwrap();
        gs_field::pool::invalidate_cache();
        assert!(has_art("timber wolf", Some("wolf")));
        assert!(!has_art("cave troll", Some("troll")));
    }

    /// ... unless a puppet comes first, by the renderer's order: one in a
    /// skin of its own beats the creature's sprite, and any puppet beats the
    /// pool's `creatures/default` stand-in.
    #[test]
    fn a_puppet_in_its_own_skin_or_over_the_default_art_is_not_sprite_art() {
        let _base = crate::stage::test_support::TempBase::new();
        let creatures = gs_field::data_dirs::global_image_category_dir("creatures").unwrap();
        std::fs::create_dir_all(creatures.join("default")).unwrap();
        std::fs::write(creatures.join("default/default.png"), b"not really a png").unwrap();
        std::fs::write(creatures.join("hill_troll.png"), b"not really a png").unwrap();
        std::fs::write(creatures.join("cave_troll.png"), b"not really a png").unwrap();
        let skins = gs_field::puppets::puppets_dir().unwrap().join("skins/troll");
        std::fs::create_dir_all(&skins).unwrap();
        std::fs::write(skins.join(format!("{}.png", creature_token("cave troll"))), b"not really a png").unwrap();
        gs_field::pool::invalidate_cache();
        gs_field::puppets::invalidate_cache();
        assert!(has_art("hill troll", Some("troll")), "its own sprite, no skin of its own");
        assert!(!has_art("cave troll", Some("troll")), "a skin of its own beats its sprite");
        assert!(!has_art("forest troll", Some("troll")), "only the default stand-in: the puppet draws");
        gs_field::puppets::invalidate_cache();
    }

    /// The Puppets tree: forms under the family most of their creatures
    /// claim, a sheet row when the form ships one, a row per creature skin
    /// (named after the bestiary creature, else the token spaced), and
    /// "Other" last.
    #[test]
    fn the_puppet_tree_groups_forms_by_family_and_lists_their_textures() {
        use gs_field::bestiary::{BestiaryDb, CreatureEntry};
        let e = |name: &str, family: &str| CreatureEntry {
            name: name.into(),
            noun: last_word(name),
            family: Some(family.into()),
            ..Default::default()
        };
        let db = BestiaryDb::from_entries(vec![e("coyote", "Canine"), e("timber wolf", "Canine"), e("giant rat", "Rodent"), e("blood eagle", "Bird")]);
        let of = |n: &str, _: &str| match n {
            "coyote" | "timber wolf" | "giant rat" => Some("canine".to_string()),
            "blood eagle" => Some("avian".to_string()),
            _ => None,
        };
        let forms = vec![
            ("canine".to_string(), "quadruped".to_string(), true, true),
            ("avian".to_string(), "avian".to_string(), false, false),
            ("worm".to_string(), "serpentine".to_string(), false, true),
        ];
        let skins = vec![("canine".to_string(), "timber_wolf".to_string()), ("canine".to_string(), "ice_hound".to_string())];
        let tree = puppet_tree_in(&db, &forms, &skins, of);
        let families: Vec<&str> = tree.iter().map(|f| f.family.as_str()).collect();
        assert_eq!(families, ["Bird", "Canine", "Other"]);
        let canine = &tree[1].forms[0];
        assert_eq!((canine.form.as_str(), canine.pool, canine.sheet), ("canine", true, true));
        let labels: Vec<&str> = canine.textures.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(labels, ["canine sheet", "timber wolf", "ice hound"]);
        assert_eq!(canine.textures[0].creature, None);
        assert_eq!(canine.textures[1].creature, Some(("timber wolf".to_string(), "wolf".to_string())));
        assert_eq!(canine.textures[2].creature, Some(("ice hound".to_string(), "hound".to_string())));
        assert!(tree[0].forms[0].textures.is_empty(), "no sheet, no skins: nothing to wear");
        assert_eq!(tree[2].forms[0].form, "worm");
    }

    /// The bare-sheet showcase passes over creatures with a skin of their own.
    #[test]
    fn the_bare_showcase_avoids_skinned_creatures() {
        use gs_field::bestiary::{BestiaryDb, CreatureEntry};
        let e = |name: &str| CreatureEntry { name: name.into(), noun: last_word(name), family: Some("Canine".into()), ..Default::default() };
        let db = BestiaryDb::from_entries(vec![e("coyote"), e("timber wolf")]);
        let canine = |_: &str, _: &str| Some("canine".to_string());
        let pick = puppet_showcase_bare_in(&db, "canine", |_, _| false, |n, _| n == "coyote", canine).unwrap();
        assert_eq!(pick.name, "timber wolf");
    }
}
