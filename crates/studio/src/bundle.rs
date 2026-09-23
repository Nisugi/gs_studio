//! Art and rig data packed into the binary by `build.rs` (the
//! `bundled-art` feature), and the seeding that unpacks it.
//!
//! Seeding only ever ADDS: a file already at the destination is left as it
//! is, so calibrations and rigs edited since are never overwritten. The app
//! seeds only gs-studio's own folders -- never a pool or data root the user
//! pointed elsewhere, and never `~/.vellum-fe`.

use std::path::Path;

mod generated {
    include!(concat!(env!("OUT_DIR"), "/bundle_index.rs"));
}

static BLOB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/bundle.bin"));

/// Which folder a packed file belongs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Root {
    Pool,
    Data,
}

impl Root {
    fn tag(self) -> &'static str {
        match self {
            Root::Pool => "pool",
            Root::Data => "data",
        }
    }
}

/// How many files the binary carries for `root`, and their total size.
pub fn contents(root: Root) -> (usize, usize) {
    let of_root = generated::ENTRIES.iter().filter(|e| e.0 == root.tag());
    of_root.fold((0, 0), |(n, bytes), e| (n + 1, bytes + e.3))
}

/// What a seed did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SeedReport {
    pub written: usize,
    pub bytes: usize,
    /// Already there, left alone.
    pub kept: usize,
    pub errors: Vec<String>,
}

impl SeedReport {
    pub fn summary(&self, what: &str, dest: &Path) -> String {
        let mut s = format!(
            "bundled {what}: {} files written ({} MB) into {}, {} already there",
            self.written,
            self.bytes / (1024 * 1024),
            dest.display(),
            self.kept
        );
        if !self.errors.is_empty() {
            s += &format!(", {} failed (first: {})", self.errors.len(), self.errors[0]);
        }
        s
    }
}

/// Unpack the binary's `root` files into `dest`, adding only what is missing.
pub fn seed(root: Root, dest: &Path) -> SeedReport {
    seed_from(generated::ENTRIES, BLOB, root, dest)
}

fn seed_from(entries: &[(&str, &str, usize, usize)], blob: &[u8], root: Root, dest: &Path) -> SeedReport {
    let mut report = SeedReport::default();
    for &(tag, rel, offset, len) in entries {
        if tag != root.tag() {
            continue;
        }
        // Paths come from build.rs, but never let one climb out of `dest`.
        if rel.split('/').any(|seg| seg == ".." || seg.is_empty()) {
            report.errors.push(format!("{rel}: not a plain relative path"));
            continue;
        }
        let path = rel.split('/').fold(dest.to_path_buf(), |p, seg| p.join(seg));
        if path.exists() {
            report.kept += 1;
            continue;
        }
        let Some(bytes) = blob.get(offset..offset + len) else {
            report.errors.push(format!("{rel}: outside the bundle"));
            continue;
        };
        let written = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&path, bytes));
        match written {
            Ok(()) => {
                report.written += 1;
                report.bytes += len;
            }
            Err(e) => report.errors.push(format!("{}: {e}", path.display())),
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOB: &[u8] = b"PNGDATAtomlOUTLINES";
    const ENTRIES: &[(&str, &str, usize, usize)] = &[
        ("pool", "global/images/creatures/coyote/coyote.png", 0, 7),
        ("pool", "global/images/creatures/coyote/coyote.toml", 7, 4),
        ("data", "outlines.json", 11, 8),
        ("pool", "../escape.txt", 0, 1),
    ];

    #[test]
    fn seeding_adds_what_is_missing_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let pool = dir.path().join("pool");
        std::fs::create_dir_all(pool.join("global/images/creatures/coyote")).unwrap();
        std::fs::write(pool.join("global/images/creatures/coyote/coyote.toml"), "edited").unwrap();

        let report = seed_from(ENTRIES, BLOB, Root::Pool, &pool);
        assert_eq!((report.written, report.bytes, report.kept), (1, 7, 1));
        assert_eq!(report.errors.len(), 1, "the escaping path is refused: {:?}", report.errors);
        assert_eq!(std::fs::read(pool.join("global/images/creatures/coyote/coyote.png")).unwrap(), b"PNGDATA");
        assert_eq!(std::fs::read_to_string(pool.join("global/images/creatures/coyote/coyote.toml")).unwrap(), "edited");
        assert!(!dir.path().join("escape.txt").exists());
        assert!(!pool.join("outlines.json").exists(), "data files do not land in the pool");

        let data = dir.path().join("data");
        let report = seed_from(ENTRIES, BLOB, Root::Data, &data);
        assert_eq!(report.written, 1);
        assert_eq!(std::fs::read(data.join("outlines.json")).unwrap(), b"OUTLINES");
        // Seeding again changes nothing.
        let again = seed_from(ENTRIES, BLOB, Root::Data, &data);
        assert_eq!((again.written, again.kept), (0, 1));
    }

    /// A normal (featureless) build carries nothing and seeds nothing.
    #[cfg(not(feature = "bundled-art"))]
    #[test]
    fn a_normal_build_carries_no_bundle() {
        assert_eq!(contents(Root::Pool), (0, 0));
        assert_eq!(contents(Root::Data), (0, 0));
    }
}
