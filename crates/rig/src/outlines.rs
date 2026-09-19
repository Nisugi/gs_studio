//! Loader for `data/outlines.json` (written by `tools/extract_outlines.py`).

use std::collections::BTreeMap;
use std::path::Path;

use rig_bake::{Outline, Pt};
use serde::Deserialize;

use crate::model::{outline_key, RigError, View};

#[derive(Debug, Clone, Deserialize)]
pub struct OutlineEntry {
    pub ref_ft: f32,
    pub subpaths: Vec<Vec<Pt>>,
}

impl OutlineEntry {
    pub fn outline(&self) -> Outline {
        Outline { subpaths: self.subpaths.clone() }
    }

    /// (min_x, min_y, max_x, max_y) in feet; None for an empty outline.
    pub fn bounds(&self) -> Option<[f32; 4]> {
        let mut pts = self.subpaths.iter().flatten();
        let first = pts.next()?;
        Some(pts.fold([first[0], first[1], first[0], first[1]], |b, p| {
            [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]
        }))
    }
}

/// Every silhouette, keyed `<family>` (side) or `<family>-front`.
#[derive(Debug, Clone, Default)]
pub struct Outlines {
    pub entries: BTreeMap<String, OutlineEntry>,
}

impl Outlines {
    pub fn from_json(text: &str) -> Result<Outlines, RigError> {
        let entries = serde_json::from_str(text).map_err(|e| RigError::Parse(e.to_string()))?;
        Ok(Outlines { entries })
    }

    pub fn load(path: &Path) -> Result<Outlines, RigError> {
        let text = std::fs::read_to_string(path).map_err(|e| RigError::Io(path.to_owned(), e))?;
        Outlines::from_json(&text)
    }

    pub fn get(&self, family: &str, view: View) -> Option<&OutlineEntry> {
        self.entries.get(&outline_key(family, view))
    }

    /// Family names that have at least one view, sorted.
    pub fn families(&self) -> Vec<String> {
        let mut out: Vec<String> =
            self.entries.keys().map(|k| k.strip_suffix("-front").unwrap_or(k).to_owned()).collect();
        out.dedup();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_split_into_family_and_view() {
        let json = r#"{"canine":{"ref_ft":3,"subpaths":[[[0,0],[1,0],[1,2]]]},
                       "canine-front":{"ref_ft":3,"subpaths":[]},
                       "worm":{"ref_ft":2,"subpaths":[]}}"#;
        let o = Outlines::from_json(json).unwrap();
        assert_eq!(o.families(), ["canine", "worm"]);
        assert_eq!(o.get("canine", View::Side).unwrap().bounds(), Some([0.0, 0.0, 1.0, 2.0]));
        assert!(o.get("canine", View::Front).unwrap().bounds().is_none());
        assert!(o.get("worm", View::Front).is_none());
    }
}
