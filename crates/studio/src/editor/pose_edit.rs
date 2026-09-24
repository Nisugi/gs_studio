//! The pose editor, without the drawing: one body type's pose file open for
//! editing, the rows a puppet offers, bone picking and undo.
//!
//! The file is edited IN PLACE (`toml_edit`): comments, key order and
//! spacing survive, and only the entries touched are rewritten, as
//! `key = { rot = [x, y, z], bend = [..] }` in whole degrees and halves.
//! What is edited is either the SHARED pose (`[pose.<name>]`, every family
//! of the body type) or this family's sparse correction
//! (`[family.<family>.<name>]`, see `gs_puppet::pose`): in the family scope
//! a key the family does not name shows the shared value, and editing it
//! adds the key to the family's section.
//!
//! A mirror pair (`leg_l` / `leg_r`) is one row, the stem `leg`, posing
//! both sides mirrored -- until *Split* writes the sides out separately.

use std::path::{Path, PathBuf};

use gs_puppet::math::{mul_mv, rot_y, V3};
use gs_puppet::{PoseSet, Posed, Puppet};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value};

/// Which section an edit goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// `[pose.<name>]`: every family of the body type.
    Shared,
    /// `[family.<family>.<name>]`: just this family's corrections.
    Family,
}

/// One bone's (or pair's) angles, degrees.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Entry {
    pub rot: [f32; 3],
    pub bend: Vec<f32>,
}

/// One key's joint limits as the file writes them, `[low, high]` degrees:
/// all three Euler angles or none, and the first `bend.len()` bends.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Limits {
    pub rot: Option<[[f32; 2]; 3]>,
    pub bend: Vec<[f32; 2]>,
}

/// The widest range an angle can have.
pub const FREE: [f32; 2] = [-180.0, 180.0];

/// Where a row's value comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Named in the section being edited.
    Here,
    /// The family scope showing the shared pose's value.
    Shared,
    /// Named nowhere: the rest angle.
    Rest,
}

/// One editable row: the root, a bone, or a mirror pair's stem.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// The pose-file key (`root`, `neck`, `leg`, `leg_l`).
    pub key: String,
    /// The puppet bones this row moves (both sides for a stem).
    pub bones: Vec<String>,
    /// How many `bend` angles the chain takes.
    pub bends: usize,
    /// A pair's stem that could be split into sides, or a side that could
    /// be joined back.
    pub pair: Option<PairState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairState {
    /// The stem row: both sides mirrored.
    Joined,
    /// One side of a split pair.
    Split,
}

pub const ROOT: &str = "root";

pub struct PoseDoc {
    body_type: String,
    path: PathBuf,
    doc: DocumentMut,
    /// The text as last saved (or opened), as `toml_edit` writes it.
    saved: String,
    /// The file uses CRLF line endings; saving keeps them.
    crlf: bool,
    undo: Vec<String>,
    redo: Vec<String>,
    /// The last edit's (scope, family, pose, key): the next edit to the same
    /// one merges into its undo step (a slider drag is one step).
    last_edit: Option<(Scope, String, String, String)>,
    revision: u64,
    parsed: Option<(u64, String, Result<PoseSet, String>)>,
}

fn round_half(v: f32) -> f64 {
    ((v as f64) * 2.0).round() / 2.0
}

fn num(v: &Value) -> Option<f32> {
    v.as_float().map(|f| f as f32).or_else(|| v.as_integer().map(|i| i as f32))
}

fn floats(v: Option<&Value>) -> Vec<f32> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(num).collect()).unwrap_or_default()
}

fn array(values: impl IntoIterator<Item = f32>) -> Array {
    let mut a: Array = values.into_iter().map(round_half).collect();
    a.fmt();
    a
}

fn entry_of(item: &Item) -> Option<Entry> {
    let get = |k: &str| -> Option<Value> {
        match item {
            Item::Value(Value::InlineTable(t)) => t.get(k).cloned(),
            Item::Table(t) => t.get(k).and_then(Item::as_value).cloned(),
            _ => None,
        }
    };
    if !item.is_inline_table() && !item.is_table() {
        return None;
    }
    let rot = floats(get("rot").as_ref());
    Some(Entry { rot: [0, 1, 2].map(|i| rot.get(i).copied().unwrap_or(0.0)), bend: floats(get("bend").as_ref()) })
}

fn entry_value(e: &Entry) -> Value {
    let mut t = InlineTable::new();
    let has_rot = e.rot.iter().any(|v| round_half(*v) != 0.0);
    if has_rot || e.bend.is_empty() {
        t.insert("rot", Value::Array(array(e.rot)));
    }
    if !e.bend.is_empty() {
        t.insert("bend", Value::Array(array(e.bend.iter().copied())));
    }
    t.fmt();
    Value::InlineTable(t)
}

fn ranges(v: Option<&Value>) -> Vec<[f32; 2]> {
    v.and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|r| r.as_array().map(|r| r.iter().filter_map(num).collect::<Vec<_>>())).filter(|r| r.len() == 2).map(|r| [r[0], r[1]]).collect())
        .unwrap_or_default()
}

fn limits_of(item: &Item) -> Option<Limits> {
    let get = |k: &str| -> Option<Value> {
        match item {
            Item::Value(Value::InlineTable(t)) => t.get(k).cloned(),
            Item::Table(t) => t.get(k).and_then(Item::as_value).cloned(),
            _ => None,
        }
    };
    if !item.is_inline_table() && !item.is_table() {
        return None;
    }
    let rot = ranges(get("rot").as_ref());
    Some(Limits { rot: (rot.len() == 3).then(|| [rot[0], rot[1], rot[2]]), bend: ranges(get("bend").as_ref()) })
}

fn limits_value(l: &Limits) -> Value {
    let pairs = |rs: &[[f32; 2]]| {
        let mut a: Array = rs
            .iter()
            .map(|[lo, hi]| {
                let (lo, hi) = (lo.min(*hi), hi.max(*lo));
                Value::Array(array([lo, hi]))
            })
            .collect();
        a.fmt();
        Value::Array(a)
    };
    let mut t = InlineTable::new();
    if let Some(rot) = &l.rot {
        t.insert("rot", pairs(rot));
    }
    if !l.bend.is_empty() {
        t.insert("bend", pairs(&l.bend));
    }
    t.fmt();
    Value::InlineTable(t)
}

/// A pair's mirror: `leg_r` from `leg` (y and z negated, as `quat_mirror`).
fn mirrored(e: &Entry) -> Entry {
    Entry { rot: [e.rot[0], -e.rot[1], -e.rot[2]], bend: e.bend.clone() }
}

impl PoseDoc {
    /// `<data>/poses/<body_type>.toml`, or the shipped copy when the data
    /// root has none (saving then creates the file).
    pub fn open(data_root: &Path, body_type: &str) -> Result<PoseDoc, String> {
        let path = data_root.join("poses").join(format!("{body_type}.toml"));
        let text = if path.is_file() {
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?
        } else {
            gs_puppet::SHIPPED_POSES
                .iter()
                .find(|(b, _)| *b == body_type)
                .map(|(_, t)| t.to_string())
                .ok_or_else(|| format!("no pose file for body type '{body_type}'"))?
        };
        let doc: DocumentMut = text.parse().map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(PoseDoc {
            body_type: body_type.to_owned(),
            path,
            saved: doc.to_string(),
            crlf: text.contains("\r\n"),
            doc,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: None,
            revision: 0,
            parsed: None,
        })
    }

    pub fn body_type(&self) -> &str {
        &self.body_type
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn text(&self) -> String {
        self.doc.to_string()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn is_dirty(&self) -> bool {
        self.text() != self.saved
    }

    /// The poses as `family` sees them, parsed once per edit.
    pub fn set_for(&mut self, family: &str) -> &Result<PoseSet, String> {
        let fresh = self.parsed.as_ref().is_some_and(|(r, f, _)| *r == self.revision && f == family);
        if !fresh {
            let set = PoseSet::from_toml(&self.text()).map(|s| s.for_family(family).into_owned());
            self.parsed = Some((self.revision, family.to_owned(), set));
        }
        &self.parsed.as_ref().expect("just parsed").2
    }

    fn section(&self, scope: Scope, family: &str, pose: &str) -> Option<&Table> {
        let top = match scope {
            Scope::Shared => self.doc.get("pose")?.as_table()?,
            Scope::Family => self.doc.get("family")?.as_table()?.get(&family.to_ascii_lowercase())?.as_table()?,
        };
        top.get(pose)?.as_table()
    }

    /// The section, made if missing (implicit parents, so the file keeps
    /// its `[pose.x]` / `[family.f.x]` headers).
    fn section_mut(&mut self, scope: Scope, family: &str, pose: &str) -> &mut Table {
        let implicit = |item: &mut Item| {
            if !item.is_table() {
                let mut t = Table::new();
                t.set_implicit(true);
                *item = Item::Table(t);
            }
        };
        let root = self.doc.as_table_mut();
        let top = match scope {
            Scope::Shared => {
                let p = root.entry("pose").or_insert(Item::None);
                implicit(p);
                p.as_table_mut().expect("table")
            }
            Scope::Family => {
                let f = root.entry("family").or_insert(Item::None);
                implicit(f);
                let fam = f.as_table_mut().expect("table").entry(&family.to_ascii_lowercase()).or_insert(Item::None);
                implicit(fam);
                fam.as_table_mut().expect("table")
            }
        };
        let s = top.entry(pose).or_insert(Item::None);
        if !s.is_table() {
            *s = Item::Table(Table::new());
        }
        s.as_table_mut().expect("table")
    }

    /// Every pose name: the shared ones, then any this family alone has.
    pub fn pose_names(&self, family: &str) -> Vec<String> {
        let mut names: Vec<String> =
            self.doc.get("pose").and_then(Item::as_table).map(|t| t.iter().map(|(k, _)| k.to_owned()).collect()).unwrap_or_default();
        if let Some(fam) = self.doc.get("family").and_then(Item::as_table).and_then(|f| f.get(&family.to_ascii_lowercase())).and_then(Item::as_table) {
            for (k, _) in fam.iter() {
                if !names.iter().any(|n| n == k) {
                    names.push(k.to_owned());
                }
            }
        }
        if !names.iter().any(|n| n == "stand") {
            names.insert(0, "stand".into());
        }
        names
    }

    /// This family has its own correction of `pose`.
    pub fn family_has(&self, family: &str, pose: &str) -> bool {
        self.section(Scope::Family, family, pose).is_some_and(|t| !t.is_empty())
    }

    fn raw(&self, scope: Scope, family: &str, pose: &str, key: &str) -> Option<Entry> {
        let item = self.section(scope, family, pose)?.get(key)?;
        if key == ROOT {
            let r = floats(item.as_value());
            return Some(Entry { rot: [0, 1, 2].map(|i| r.get(i).copied().unwrap_or(0.0)), bend: Vec::new() });
        }
        entry_of(item)
    }

    /// What `key` resolves to in one section: exact, else a side read from
    /// its stem (mirrored for `_r`).
    fn resolve(&self, scope: Scope, family: &str, pose: &str, key: &str) -> Option<Entry> {
        if let Some(e) = self.raw(scope, family, pose, key) {
            return Some(e);
        }
        let (stem, right) = match (key.strip_suffix("_l"), key.strip_suffix("_r")) {
            (Some(s), _) => (s, false),
            (_, Some(s)) => (s, true),
            _ => return None,
        };
        let e = self.raw(scope, family, pose, stem)?;
        Some(if right { mirrored(&e) } else { e })
    }

    /// The row's value in `scope`, and where it came from.
    pub fn value(&self, scope: Scope, family: &str, pose: &str, key: &str) -> (Entry, Origin) {
        if let Some(e) = self.resolve(scope, family, pose, key) {
            return (e, Origin::Here);
        }
        if scope == Scope::Family {
            if let Some(e) = self.resolve(Scope::Shared, family, pose, key) {
                return (e, Origin::Shared);
            }
        }
        (Entry::default(), Origin::Rest)
    }

    fn checkpoint(&mut self, edit: Option<(Scope, String, String, String)>) {
        if edit.is_none() || edit != self.last_edit {
            self.undo.push(self.text());
            if self.undo.len() > 200 {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last_edit = edit;
        self.revision += 1;
    }

    /// Write `key` (None removes it) in the section; an emptied family
    /// section is removed, so the file only says what differs.
    pub fn set(&mut self, scope: Scope, family: &str, pose: &str, key: &str, entry: Option<&Entry>) {
        let now = self.raw(scope, family, pose, key);
        if now.as_ref() == entry && entry.is_some() {
            return;
        }
        if entry.is_none() && self.section(scope, family, pose).is_none_or(|t| !t.contains_key(key)) {
            return;
        }
        self.checkpoint(Some((scope, family.to_owned(), pose.to_owned(), key.to_owned())));
        let section = self.section_mut(scope, family, pose);
        match entry {
            Some(e) => {
                let mut v = if key == ROOT { Value::Array(array(e.rot)) } else { entry_value(e) };
                match section.get(key).and_then(Item::as_value) {
                    Some(old) => *v.decor_mut() = old.decor().clone(),
                    None => v.decor_mut().set_prefix(" "),
                }
                section.insert(key, Item::Value(v));
            }
            None => {
                section.remove(key);
            }
        }
        if scope == Scope::Family {
            self.prune_family(family, pose);
        }
    }

    fn prune_family(&mut self, family: &str, pose: &str) {
        let fam = family.to_ascii_lowercase();
        let Some(families) = self.doc.get_mut("family").and_then(Item::as_table_mut) else { return };
        if let Some(f) = families.get_mut(&fam).and_then(Item::as_table_mut) {
            if f.get(pose).and_then(Item::as_table).is_some_and(Table::is_empty) {
                f.remove(pose);
            }
            if f.is_empty() {
                families.remove(&fam);
            }
        }
        if families.is_empty() {
            self.doc.remove("family");
        }
    }

    fn limits_table(&self, scope: Scope, family: &str) -> Option<&Table> {
        match scope {
            Scope::Shared => self.doc.get("limits")?.as_table(),
            Scope::Family => self.doc.get("family_limits")?.as_table()?.get(&family.to_ascii_lowercase())?.as_table(),
        }
    }

    /// `key`'s joint limits in `scope` (a pair's limits are its stem's, for
    /// the left side), and where they come from: the family scope shows the
    /// shared limits until it names the key. None = no limit.
    pub fn limits(&self, scope: Scope, family: &str, key: &str) -> (Option<Limits>, Origin) {
        let get = |s: Scope| self.limits_table(s, family).and_then(|t| t.get(key)).and_then(limits_of);
        if let Some(l) = get(scope) {
            return (Some(l), Origin::Here);
        }
        if scope == Scope::Family {
            if let Some(l) = get(Scope::Shared) {
                return (Some(l), Origin::Shared);
            }
        }
        (None, Origin::Rest)
    }

    /// Write `key`'s limits (None removes them) under `[limits]`, or this
    /// family's `[family_limits.<family>]` (which replaces the shared key
    /// whole). A drag on one key is one undo step.
    pub fn set_limits(&mut self, scope: Scope, family: &str, key: &str, limits: Option<&Limits>) {
        let now = self.limits_table(scope, family).and_then(|t| t.get(key)).and_then(limits_of);
        if now.as_ref() == limits {
            return;
        }
        self.checkpoint(Some((scope, family.to_owned(), "\0limits".to_owned(), key.to_owned())));
        let fam = family.to_ascii_lowercase();
        let root = self.doc.as_table_mut();
        let table = match scope {
            Scope::Shared => {
                let item = root.entry("limits").or_insert(Item::None);
                if !item.is_table() {
                    *item = Item::Table(Table::new());
                }
                item.as_table_mut().expect("table")
            }
            Scope::Family => {
                let top = root.entry("family_limits").or_insert(Item::None);
                if !top.is_table() {
                    let mut t = Table::new();
                    t.set_implicit(true);
                    *top = Item::Table(t);
                }
                let item = top.as_table_mut().expect("table").entry(&fam).or_insert(Item::None);
                if !item.is_table() {
                    *item = Item::Table(Table::new());
                }
                item.as_table_mut().expect("table")
            }
        };
        match limits {
            Some(l) => {
                let mut v = limits_value(l);
                match table.get(key).and_then(Item::as_value) {
                    Some(old) => *v.decor_mut() = old.decor().clone(),
                    None => v.decor_mut().set_prefix(" "),
                }
                table.insert(key, Item::Value(v));
            }
            None => {
                table.remove(key);
            }
        }
        // An emptied family table goes, so the file only says what differs.
        if scope == Scope::Family {
            if let Some(top) = self.doc.get_mut("family_limits").and_then(Item::as_table_mut) {
                if top.get(&fam).and_then(Item::as_table).is_some_and(Table::is_empty) {
                    top.remove(&fam);
                }
                if top.is_empty() {
                    self.doc.remove("family_limits");
                }
            }
        }
    }

    /// Replace a joined pair with its two sides (the right mirrored), or
    /// join the sides back into the stem (the left side's angles win).
    pub fn split_pair(&mut self, scope: Scope, family: &str, pose: &str, stem: &str) {
        let (e, _) = self.value(scope, family, pose, stem);
        self.checkpoint(None);
        let section = self.section_mut(scope, family, pose);
        let decor = section.get(stem).and_then(Item::as_value).map(|v| v.decor().clone());
        section.remove(stem);
        for (side, e) in [("_l", e.clone()), ("_r", mirrored(&e))] {
            let mut v = entry_value(&e);
            match &decor {
                Some(d) => *v.decor_mut() = d.clone(),
                None => v.decor_mut().set_prefix(" "),
            }
            section.insert(&format!("{stem}{side}"), Item::Value(v));
        }
    }

    pub fn join_pair(&mut self, scope: Scope, family: &str, pose: &str, stem: &str) {
        let (e, _) = self.value(scope, family, pose, &format!("{stem}_l"));
        self.checkpoint(None);
        let section = self.section_mut(scope, family, pose);
        let decor = section.get(&format!("{stem}_l")).and_then(Item::as_value).map(|v| v.decor().clone());
        section.remove(&format!("{stem}_l"));
        section.remove(&format!("{stem}_r"));
        let mut v = entry_value(&e);
        match decor {
            Some(d) => *v.decor_mut() = d,
            None => v.decor_mut().set_prefix(" "),
        }
        section.insert(stem, Item::Value(v));
    }

    /// Does the section (or, in the family scope, the shared pose under it)
    /// spell this pair out per side?
    pub fn pair_is_split(&self, scope: Scope, family: &str, pose: &str, stem: &str) -> bool {
        let has = |s: Scope| {
            self.section(s, family, pose).is_some_and(|t| t.contains_key(&format!("{stem}_l")) || t.contains_key(&format!("{stem}_r")))
        };
        let names_stem = |s: Scope| self.section(s, family, pose).is_some_and(|t| t.contains_key(stem));
        match scope {
            Scope::Shared => has(Scope::Shared),
            // A family naming the stem replaces the shared sides.
            Scope::Family => has(Scope::Family) || (!names_stem(Scope::Family) && has(Scope::Shared)),
        }
    }

    /// A new shared pose, a copy of `from` (or empty). False when the name
    /// is taken or not a valid key.
    pub fn new_pose(&mut self, name: &str, from: Option<&str>) -> bool {
        let name = name.trim();
        let valid = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !valid || self.section(Scope::Shared, "", name).is_some() {
            return false;
        }
        let copy = from.and_then(|f| self.section(Scope::Shared, "", f)).cloned();
        self.checkpoint(None);
        let section = self.section_mut(Scope::Shared, "", name);
        if let Some(copy) = copy {
            for (k, v) in copy.iter() {
                section.insert(k, v.clone());
            }
        }
        true
    }

    /// Remove a pose: the shared one and every family's correction of it
    /// (`stand` stays: the rest pose always exists).
    pub fn delete_pose(&mut self, name: &str) -> bool {
        if name == "stand" {
            return false;
        }
        self.checkpoint(None);
        let mut removed = self.doc.get_mut("pose").and_then(Item::as_table_mut).and_then(|t| t.remove(name)).is_some();
        let families: Vec<String> =
            self.doc.get("family").and_then(Item::as_table).map(|t| t.iter().map(|(k, _)| k.to_owned()).collect()).unwrap_or_default();
        for f in families {
            removed |= self.doc["family"][&f].as_table_mut().and_then(|t| t.remove(name)).is_some();
            self.prune_family(&f, name);
        }
        removed
    }

    /// Drop this family's whole correction of `pose`.
    pub fn clear_family(&mut self, family: &str, pose: &str) {
        if !self.family_has(family, pose) {
            return;
        }
        self.checkpoint(None);
        if let Some(f) = self.doc.get_mut("family").and_then(|f| f.get_mut(&family.to_ascii_lowercase())).and_then(Item::as_table_mut) {
            f.remove(pose);
        }
        self.prune_family(family, pose);
    }

    fn restore(&mut self, text: String) {
        if let Ok(doc) = text.parse() {
            self.doc = doc;
            self.revision += 1;
            self.last_edit = None;
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) {
        if let Some(t) = self.undo.pop() {
            self.redo.push(self.text());
            self.restore(t);
        }
    }

    pub fn redo(&mut self) {
        if let Some(t) = self.redo.pop() {
            self.undo.push(self.text());
            self.restore(t);
        }
    }

    /// Write the file (checked first: a file that would not load is not
    /// written).
    pub fn save(&mut self) -> Result<&Path, String> {
        let text = self.text();
        PoseSet::from_toml(&text)?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let on_disk = if self.crlf { text.replace("\r\n", "\n").replace('\n', "\r\n") } else { text.clone() };
        std::fs::write(&self.path, on_disk).map_err(|e| format!("{}: {e}", self.path.display()))?;
        self.saved = text;
        self.last_edit = None;
        Ok(&self.path)
    }
}

/// The rows a puppet offers, root first, in the puppet's own bone order: a
/// bone whose mirror exists is one stem row, unless `split(stem)` says the
/// pose spells the sides out.
pub fn rows(puppet: &Puppet, split: &dyn Fn(&str) -> bool) -> Vec<Row> {
    let mut bones: Vec<(String, usize)> = Vec::new();
    // Held props are not bones a pose moves.
    for s in puppet.segments.iter().filter(|s| !s.kind.is_prop()) {
        match bones.iter_mut().find(|(b, _)| *b == s.bone) {
            Some((_, n)) => *n = (*n).max(s.chain_index),
            None => bones.push((s.bone.clone(), s.chain_index)),
        }
    }
    let has = |b: &str| bones.iter().any(|(x, _)| x == b);
    let mut out = vec![Row { key: ROOT.into(), bones: Vec::new(), bends: 0, pair: None }];
    let mut seen = std::collections::HashSet::new();
    for (bone, bends) in &bones {
        let stem = bone.strip_suffix("_l").or_else(|| bone.strip_suffix("_r")).filter(|s| has(&format!("{s}_l")) && has(&format!("{s}_r")));
        match stem {
            Some(stem) if !split(stem) => {
                if seen.insert(stem.to_owned()) {
                    out.push(Row {
                        key: stem.to_owned(),
                        bones: vec![format!("{stem}_l"), format!("{stem}_r")],
                        bends: *bends,
                        pair: Some(PairState::Joined),
                    });
                }
            }
            Some(_) => out.push(Row { key: bone.clone(), bones: vec![bone.clone()], bends: *bends, pair: Some(PairState::Split) }),
            None => out.push(Row { key: bone.clone(), bones: vec![bone.clone()], bends: *bends, pair: None }),
        }
    }
    out
}

/// The row a bone belongs to.
pub fn row_of<'a>(rows: &'a [Row], bone: &str) -> Option<&'a Row> {
    rows.iter().find(|r| r.bones.iter().any(|b| b == bone))
}

/// The bone under a screen point: the segment whose projected bone line
/// passes nearest (within `reach_px`), the nearer one on a tie. `project`
/// takes a world point (after `yaw` turns the figure) to screen.
pub fn pick_bone(posed: &Posed, yaw: f32, project: &dyn Fn(V3) -> ([f32; 2], f32), at: [f32; 2], reach_px: f32) -> Option<String> {
    let turn = rot_y(yaw);
    let mut best: Option<(f32, f32, &str)> = None;
    for (i, s) in posed.puppet.segments.iter().enumerate() {
        let (a, da) = project(mul_mv(&turn, posed.point(i, s.head)));
        let (b, db) = project(mul_mv(&turn, posed.point(i, s.tail)));
        let (ab, ap) = ([b[0] - a[0], b[1] - a[1]], [at[0] - a[0], at[1] - a[1]]);
        let len2 = (ab[0] * ab[0] + ab[1] * ab[1]).max(1e-6);
        let t = ((ap[0] * ab[0] + ap[1] * ab[1]) / len2).clamp(0.0, 1.0);
        let d = (ap[0] - ab[0] * t).hypot(ap[1] - ab[1] * t);
        // Thick parts are hit further from their line.
        let fat = s.stations.iter().map(|st| st.w.max(st.d)).fold(0.0f32, f32::max);
        let (sa, _) = project(mul_mv(&turn, posed.point(i, [s.head[0] + fat, s.head[1], s.head[2]])));
        let d = (d - ((sa[0] - a[0]).hypot(sa[1] - a[1]) * 0.6)).max(0.0);
        if d > reach_px {
            continue;
        }
        let depth = da + (db - da) * t;
        if best.is_none_or(|(bd, bz, _)| d < bd - 0.5 || (d <= bd + 0.5 && depth < bz)) {
            best = Some((d, depth, &s.bone));
        }
    }
    best.map(|(_, _, b)| b.to_owned())
}

/// The screen lines of `bones` in this pose (for highlighting them).
pub fn bone_lines(posed: &Posed, yaw: f32, project: &dyn Fn(V3) -> ([f32; 2], f32), bones: &[String]) -> Vec<[[f32; 2]; 2]> {
    let turn = rot_y(yaw);
    posed
        .puppet
        .segments
        .iter()
        .enumerate()
        .filter(|(_, s)| bones.contains(&s.bone))
        .map(|(i, s)| [project(mul_mv(&turn, posed.point(i, s.head))).0, project(mul_mv(&turn, posed.point(i, s.tail))).0])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "# Poses for bipeds.\nbody_type = \"biped\"\n\n[pose.stand]\n\n# Knees bent.\n[pose.kneel]\nleg = { bend = [92.0, 62.0] }   # both\narm = { rot = [-6.0, 0.0, 0.0], bend = [-16.0] }\n\n[family.ogre.kneel]\nroot = [5.0, 0.0, 0.0]\n";

    fn doc(dir: &Path) -> PoseDoc {
        std::fs::create_dir_all(dir.join("poses")).unwrap();
        std::fs::write(dir.join("poses/biped.toml"), FILE).unwrap();
        PoseDoc::open(dir, "biped").unwrap()
    }

    #[test]
    fn edits_keep_the_file_and_undo_merges_a_drag() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = doc(dir.path());
        assert!(!d.is_dirty());
        let (arm, origin) = d.value(Scope::Shared, "troll", "kneel", "arm");
        assert_eq!((arm.rot, origin), ([-6.0, 0.0, 0.0], Origin::Here));
        // A side reads its stem, mirrored.
        let (right, _) = d.value(Scope::Shared, "troll", "kneel", "arm_r");
        assert_eq!(right.rot, [-6.0, 0.0, 0.0]);
        for x in [-10.0, -20.0, -30.25] {
            d.set(Scope::Shared, "troll", "kneel", "arm", Some(&Entry { rot: [x, 0.0, 12.0], bend: vec![-16.0] }));
        }
        let text = d.text();
        assert!(text.contains("# Knees bent.") && text.contains("# both"), "comments survive:\n{text}");
        assert!(text.contains("arm = { rot = [-30.5, 0.0, 12.0], bend = [-16.0] }"), "{text}");
        assert!(d.is_dirty());
        d.undo();
        assert_eq!(d.text(), FILE, "one drag, one undo step");
        d.redo();
        assert!(d.text().contains("-30.5"));
        let set = d.set_for("troll").as_ref().unwrap().clone();
        assert!(set.get("kneel").unwrap().bones.contains_key("arm"));
        d.save().unwrap();
        assert!(!d.is_dirty());
        assert!(std::fs::read_to_string(dir.path().join("poses/biped.toml")).unwrap().contains("-30.5"));
    }

    #[test]
    fn a_crlf_file_opens_clean_and_saves_crlf() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("poses")).unwrap();
        std::fs::write(dir.path().join("poses/biped.toml"), FILE.replace('\n', "\r\n")).unwrap();
        let mut d = PoseDoc::open(dir.path(), "biped").unwrap();
        assert!(!d.is_dirty(), "opening is not an edit");
        d.set(Scope::Shared, "", "kneel", "arm", Some(&Entry { rot: [1.0, 0.0, 0.0], bend: vec![] }));
        d.save().unwrap();
        let text = std::fs::read_to_string(dir.path().join("poses/biped.toml")).unwrap();
        assert!(!text.replace("\r\n", "").contains('\n'), "every line ends CRLF");
    }

    #[test]
    fn a_family_inherits_until_it_names_a_key_and_empties_away() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = doc(dir.path());
        let (leg, origin) = d.value(Scope::Family, "ogre", "kneel", "leg");
        assert_eq!((leg.bend.clone(), origin), (vec![92.0, 62.0], Origin::Shared));
        assert_eq!(d.value(Scope::Family, "ogre", "kneel", ROOT).1, Origin::Here);
        d.set(Scope::Family, "ogre", "kneel", "leg", Some(&Entry { rot: [0.0; 3], bend: vec![40.0, 20.0] }));
        assert!(d.text().contains("[family.ogre.kneel]") && d.text().contains("leg = { bend = [40.0, 20.0] }"), "{}", d.text());
        // A new family section keeps the dotted header.
        d.set(Scope::Family, "troll", "kneel", "arm", Some(&Entry { rot: [1.0, 0.0, 0.0], bend: vec![] }));
        assert!(d.text().contains("[family.troll.kneel]"), "{}", d.text());
        d.set(Scope::Family, "troll", "kneel", "arm", None);
        assert!(!d.text().contains("troll"), "an emptied correction is removed:\n{}", d.text());
        d.clear_family("ogre", "kneel");
        assert!(!d.text().contains("family"), "{}", d.text());
        assert!(PoseSet::from_toml(&d.text()).is_ok());
    }

    #[test]
    fn pairs_split_and_join_and_poses_come_and_go() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = doc(dir.path());
        assert!(!d.pair_is_split(Scope::Shared, "", "kneel", "arm"));
        d.split_pair(Scope::Shared, "", "kneel", "arm");
        assert!(d.pair_is_split(Scope::Shared, "", "kneel", "arm"));
        assert!(d.text().contains("arm_l = { rot = [-6.0, 0.0, 0.0], bend = [-16.0] }"), "{}", d.text());
        d.set(Scope::Shared, "", "kneel", "arm_r", Some(&Entry { rot: [-6.0, 0.0, -30.0], bend: vec![-16.0] }));
        let set = PoseSet::from_toml(&d.text()).unwrap();
        assert_ne!(set.get("kneel").unwrap().bone("arm_l"), set.get("kneel").unwrap().bone("arm_r"));
        d.join_pair(Scope::Shared, "", "kneel", "arm");
        assert!(!d.pair_is_split(Scope::Shared, "", "kneel", "arm"));

        assert!(d.new_pose("crouch", Some("kneel")));
        assert!(!d.new_pose("crouch", None), "taken");
        assert!(!d.new_pose("bad name", None));
        let set = PoseSet::from_toml(&d.text()).unwrap();
        assert_eq!(set.get("crouch"), set.get("kneel"));
        assert!(d.pose_names("ogre").contains(&"crouch".to_owned()));
        assert!(d.delete_pose("kneel"));
        assert!(!d.text().contains("kneel") && !d.delete_pose("stand"), "{}", d.text());
    }

    #[test]
    fn limits_edit_in_place_per_scope_and_bound_the_set() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = doc(dir.path());
        assert_eq!(d.limits(Scope::Shared, "", "leg"), (None, Origin::Rest));
        let leg = Limits { rot: Some([[-120.0, 40.0], [-40.0, 40.0], [-30.0, 70.0]]), bend: vec![[0.0, 150.0]] };
        for hi in [30.0, 35.0, 40.0] {
            let mut l = leg.clone();
            l.rot.as_mut().unwrap()[0][1] = hi;
            d.set_limits(Scope::Shared, "", "leg", Some(&l));
        }
        assert!(d.text().contains("[limits]\nleg = { rot = [[-120.0, 40.0], [-40.0, 40.0], [-30.0, 70.0]], bend = [[0.0, 150.0]] }"), "{}", d.text());
        assert!(d.text().contains("# Knees bent."), "comments survive");
        let set = PoseSet::from_toml(&d.text()).unwrap();
        assert_eq!(set.limit("leg_l").unwrap().rot[0], Some([-120.0, 40.0]));
        d.undo();
        assert_eq!(d.text(), FILE, "one drag, one undo step");
        d.redo();

        // The family inherits, then replaces the key whole.
        assert_eq!(d.limits(Scope::Family, "ogre", "leg"), (Some(leg.clone()), Origin::Shared));
        let ogre = Limits { rot: None, bend: vec![[0.0, 90.0]] };
        d.set_limits(Scope::Family, "ogre", "leg", Some(&ogre));
        assert!(d.text().contains("[family_limits.ogre]\nleg = { bend = [[0.0, 90.0]] }"), "{}", d.text());
        let ogres = PoseSet::from_toml(&d.text()).unwrap().for_family("ogre").into_owned();
        assert_eq!(ogres.limit("leg").unwrap().rot, [None; 3]);
        d.set_limits(Scope::Family, "ogre", "leg", None);
        assert!(!d.text().contains("family_limits"), "{}", d.text());
        // A range typed high-low is written low-high.
        d.set_limits(Scope::Shared, "", "arm", Some(&Limits { rot: None, bend: vec![[10.0, -150.0]] }));
        assert!(d.text().contains("arm = { bend = [[-150.0, 10.0]] }"), "{}", d.text());
        d.set_limits(Scope::Shared, "", "leg", None);
        assert!(PoseSet::from_toml(&d.text()).unwrap().limit("leg").is_none());
    }

    #[test]
    fn rows_group_mirror_pairs_and_picking_finds_the_bone() {
        let baked = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/rigs/humanoid.puppet.toml")).unwrap();
        let puppet = Puppet::from_baked_toml(&baked).unwrap();
        let rows = rows(&puppet, &|_| false);
        assert_eq!(rows[0].key, ROOT);
        let leg = rows.iter().find(|r| r.key == "leg").expect("a leg pair");
        assert_eq!(leg.bones, vec!["leg_l".to_owned(), "leg_r".to_owned()]);
        assert!(leg.bends >= 1);
        let split = super::rows(&puppet, &|s| s == "leg");
        assert!(split.iter().any(|r| r.key == "leg_r" && r.pair == Some(PairState::Split)));
        assert_eq!(row_of(&rows, "leg_r").unwrap().key, "leg");

        // Front view, the camera the preview uses: the middle of a shin.
        let posed = puppet.pose(&gs_puppet::Pose::default());
        let cam = gs_puppet::Camera::framing(puppet.height, [400.0, 600.0]);
        let project = |p: V3| cam.project(p);
        let lines = bone_lines(&posed, 0.0, &project, &["leg_l".to_owned()]);
        let [a, b] = *lines.last().unwrap();
        let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        assert_eq!(pick_bone(&posed, 0.0, &project, mid, 10.0).as_deref(), Some("leg_l"));
        assert_eq!(pick_bone(&posed, 0.0, &project, [-500.0, -500.0], 10.0), None);
    }
}
