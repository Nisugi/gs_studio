//! Where the studio's Doll window gets its figures (docs/crate-split-plan.md,
//! S6): the pool's installed figures first, as a game client's doll reads
//! them (`injury_doll::source::DollSource`), then any form of the creature
//! field's (`GS_STUDIO_DOLL_FIGURE` names any, a dev's way past the
//! roster). Poses, sheets, props and outfits are the field's -- the pose
//! sets as the pose editor last published them, what a figure holds. A
//! body folder the window names is the cache's own
//! (`PuppetCache::set_body_dir`). No egui here.

use std::any::Any;
use std::path::PathBuf;
use std::sync::Arc;

use creature_field_egui::field_puppets::FieldSource;
use creature_field_egui::puppet_cache::{PuppetSource, Sheets};
use gs_puppet::Puppet;
use injury_doll::source::DollSource;

/// The Doll window's [`PuppetSource`]: the doll's figures, then the field's.
#[derive(Default)]
pub struct StudioDollSource {
    doll: DollSource,
    field: FieldSource,
}

impl PuppetSource for StudioDollSource {
    fn refresh(&mut self) -> u64 {
        self.doll.refresh() ^ self.field.refresh().rotate_left(32)
    }

    fn generation_now(&self) -> u64 {
        self.doll.generation_now() ^ self.field.generation_now().rotate_left(32)
    }

    fn baked(&self, form: &str) -> Option<(Arc<str>, String)> {
        self.doll.baked(form).or_else(|| self.field.baked(form))
    }

    fn pose_file(&self, body_type: &str) -> Option<Arc<str>> {
        self.field.pose_file(body_type).or_else(|| self.doll.pose_file(body_type))
    }

    fn sheets(&mut self, form: &str, creature: Option<&str>) -> Sheets {
        self.field.sheets(form, creature)
    }

    fn dress(&mut self, puppet: &Puppet, creature: &str) -> Option<Puppet> {
        self.field.dress(puppet, creature)
    }

    fn arm(&mut self, puppet: &Puppet, arms: &str) -> Puppet {
        self.field.arm(puppet, arms)
    }

    fn body_dir(&self, form: &str) -> Option<PathBuf> {
        self.doll.body_dir(form).or_else(|| self.field.body_dir(form))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
