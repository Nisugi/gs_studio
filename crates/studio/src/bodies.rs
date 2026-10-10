//! Modelled bodies in the data root: `<data>/meshes/<form>/[<size>/]`,
//! each a folder `PuppetCache::set_body_dir` (`gs_field_egui::puppet_cache`) reads
//! (`body.obj`, `color.png`, `weights.txt`, `splat.spz`...). A form with a
//! body may bring its own puppet -- its rings measured from the mesh
//! (`gs_puppet::mesh::BoundMesh::measured`) -- as `<data>/meshes/<form>/
//! puppet.toml`, which is installed into the pool's puppets as that form.
//! The body itself (its largest size) is installed into the pool's
//! `global/bodies/<form>/` ([`install_bodies`]), where `gs_field_egui`
//! finds it by itself, so a game client draws the form as its body with no
//! studio code.
//!
//! No egui here.

use std::path::{Path, PathBuf};

/// A form's bodies: each size's name ("" for one in the form's own folder)
/// and folder, largest first as named (master, then by number).
#[derive(Debug, Clone, PartialEq)]
pub struct FormBodies {
    pub form: String,
    pub sizes: Vec<(String, PathBuf)>,
    /// The form is this body's own: it brings its puppet (`puppet.toml`).
    pub own_puppet: bool,
}

/// The bodies under `<data>/meshes`, by form.
pub fn scan(data: &Path) -> Vec<FormBodies> {
    let mut out = Vec::new();
    let Ok(forms) = std::fs::read_dir(data.join("meshes")) else { return out };
    for form in forms.flatten().filter(|e| e.path().is_dir()) {
        let dir = form.path();
        let mut sizes: Vec<(String, PathBuf)> = Vec::new();
        if dir.join("body.obj").exists() {
            sizes.push((String::new(), dir.clone()));
        }
        for sub in std::fs::read_dir(&dir).into_iter().flatten().flatten().filter(|e| e.path().join("body.obj").exists()) {
            sizes.push((sub.file_name().to_string_lossy().into_owned(), sub.path()));
        }
        // master first, then the larger counts ("150k" before "40k")
        let rank = |name: &str| if name == "master" { u32::MAX } else { name.trim_end_matches('k').parse::<u32>().unwrap_or(0) };
        sizes.sort_by(|a, b| rank(&b.0).cmp(&rank(&a.0)).then(a.0.cmp(&b.0)));
        if !sizes.is_empty() {
            out.push(FormBodies { form: form.file_name().to_string_lossy().into_owned(), sizes, own_puppet: dir.join("puppet.toml").exists() });
        }
    }
    out.sort_by(|a, b| a.form.cmp(&b.form));
    out
}

/// Each form's own puppet (`<data>/meshes/<form>/puppet.toml`) copied into
/// `puppets_dir` as `<form>.puppet.toml` where it is missing or differs.
/// Returns the forms installed: the caller then drops the pool's caches.
pub fn install_puppets(data: &Path, puppets_dir: &Path) -> std::io::Result<Vec<String>> {
    install_puppets_of(data, puppets_dir, &|_| true)
}

/// [`install_puppets`] of the forms `keep` takes.
fn install_puppets_of(data: &Path, puppets_dir: &Path, keep: &dyn Fn(&str) -> bool) -> std::io::Result<Vec<String>> {
    let mut installed = Vec::new();
    for body in scan(data).into_iter().filter(|b| keep(&b.form)) {
        let src = data.join("meshes").join(&body.form).join("puppet.toml");
        let Ok(text) = std::fs::read_to_string(&src) else { continue };
        let dst = puppets_dir.join(format!("{}.puppet.toml", body.form));
        if std::fs::read_to_string(&dst).is_ok_and(|have| have == text) {
            continue;
        }
        std::fs::create_dir_all(puppets_dir)?;
        std::fs::write(&dst, text)?;
        installed.push(body.form);
    }
    Ok(installed)
}

/// The files a body is made of, in the pool as in the data root. The cloud
/// is `splat.spz` where there is one, else `splat.ply`.
const BODY_FILES: [&str; 4] = ["body.obj", "color.png", "normal.png", "weights.txt"];

/// Each form's largest body copied into `bodies_dir/<form>/` where a file
/// is missing or differs in size or modification time (an OBJ can be
/// hundreds of MB: it is not compared byte by byte). A `splat.ply` is
/// removed when a `splat.spz` is installed. Only a form that is its body's
/// own (it brings its `puppet.toml`) is installed: a game client draws
/// such a form as its body by itself, and a body made for another form's
/// rig (`humanoid/ranger`) stays a studio choice. Returns the forms
/// touched.
pub fn install_bodies(data: &Path, bodies_dir: &Path) -> std::io::Result<Vec<String>> {
    install_bodies_of(data, bodies_dir, &|_| true, false)
}

/// [`install_bodies`] of the forms `keep` takes. `client`: a game client's
/// pool, which draws a body as its cloud -- a body with one goes without
/// its sheets (`color.png`, `normal.png`: the thin mesh's paint does not
/// land anyway), and any left there from before are removed.
fn install_bodies_of(data: &Path, bodies_dir: &Path, keep: &dyn Fn(&str) -> bool, client: bool) -> std::io::Result<Vec<String>> {
    fn same(a: &Path, b: &Path) -> bool {
        match (std::fs::metadata(a), std::fs::metadata(b)) {
            (Ok(a), Ok(b)) => a.len() == b.len() && a.modified().ok() == b.modified().ok(),
            _ => false,
        }
    }
    fn put(src: &Path, dst: &Path) -> std::io::Result<bool> {
        if !src.is_file() || same(src, dst) {
            return Ok(false);
        }
        std::fs::copy(src, dst)?;
        // the copy keeps the source's time, so `same` holds next time
        if let Ok(m) = std::fs::metadata(src).and_then(|m| m.modified()) {
            let _ = std::fs::File::options().write(true).open(dst).and_then(|f| f.set_modified(m));
        }
        Ok(true)
    }
    let mut touched = Vec::new();
    for body in scan(data).into_iter().filter(|b| b.own_puppet && keep(&b.form)) {
        let Some((_, src_dir)) = body.sizes.first() else { continue };
        let dst_dir = bodies_dir.join(&body.form);
        let mut changed = false;
        let mut files: Vec<(PathBuf, PathBuf)> = BODY_FILES.iter().map(|f| (src_dir.join(f), dst_dir.join(f))).collect();
        // a thinned runtime mesh (body_thin.obj, weights_thin.txt) is what the
        // pool gets under the plain names: the doll draws splats, and the
        // mesh only binds them -- the full one stays in the data root
        if src_dir.join("body_thin.obj").is_file() {
            for (from, to) in [("body_thin.obj", "body.obj"), ("weights_thin.txt", "weights.txt")] {
                if let Some(pair) = files.iter_mut().find(|(_, d)| d.ends_with(to)) {
                    pair.0 = src_dir.join(from);
                }
            }
        }
        let cloud = gs_puppet::splat::cloud_file(src_dir, "splat");
        if cloud.is_file() {
            files.push((cloud.clone(), dst_dir.join(cloud.file_name().expect("a file name"))));
        }
        let sheets = ["color.png", "normal.png"];
        if client && cloud.is_file() {
            files.retain(|(_, d)| !sheets.iter().any(|f| d.ends_with(f)));
            for f in sheets {
                if dst_dir.join(f).is_file() {
                    std::fs::remove_file(dst_dir.join(f))?;
                    changed = true;
                }
            }
        }
        // how the injury doll shows it (`injury_doll::roster`), and its
        // puppet, kept with the form: the folder is the whole figure
        // (`injury_doll::source`; the puppet goes to `global/puppets` too,
        // for a client from before the split)
        files.push((data.join("meshes").join(&body.form).join("doll.toml"), dst_dir.join("doll.toml")));
        files.push((data.join("meshes").join(&body.form).join("puppet.toml"), dst_dir.join("puppet.toml")));
        if files.iter().any(|(s, d)| s.is_file() && !same(s, d)) {
            std::fs::create_dir_all(&dst_dir)?;
        }
        for (src, dst) in &files {
            changed |= put(src, dst)?;
        }
        // the pool keeps the source's one cloud: an spz the source gave up
        // for a ply (a retrain) must not stay to be loaded first, nor a ply
        // the source replaced with an spz
        if cloud.is_file() {
            let other = if cloud.extension().is_some_and(|e| e == "spz") { "splat.ply" } else { "splat.spz" };
            if dst_dir.join(other).is_file() {
                std::fs::remove_file(dst_dir.join(other))?;
                changed = true;
            }
        }
        if changed {
            touched.push(body.form);
        }
    }
    Ok(touched)
}

/// [`install_puppets`] and [`install_bodies`] into the pool's own folders
/// (`gs_field::puppets::{puppets_dir, bodies_dir}`): the forms touched.
pub fn install_all(data: &Path) -> std::io::Result<Vec<String>> {
    let err = |e| std::io::Error::other(format!("{e}"));
    let mut forms = install_puppets(data, &gs_field::puppets::puppets_dir().map_err(err)?)?;
    forms.extend(install_bodies(data, &gs_field::data_dirs::bodies_dir().map_err(err)?)?);
    forms.sort();
    forms.dedup();
    Ok(forms)
}

/// The forms whose `<data>/meshes/<form>/doll.toml` says the playable race
/// they are a figure of (`injury_doll::roster`): what a game client's
/// doll offers.
pub fn race_forms(data: &Path) -> Vec<String> {
    scan(data)
        .into_iter()
        .filter(|b| b.own_puppet)
        .filter(|b| injury_doll::roster::read_figure(&b.form, &data.join("meshes").join(&b.form)).race.is_some())
        .map(|b| b.form)
        .collect()
}

/// A game client's doll, into the pool at `pool` (its `global/puppets`
/// and `global/bodies`): the playable races' figures alone ([`race_forms`]),
/// each its puppet and its body -- the thin mesh where there is one, and
/// its one cloud (`splat.spz` over `splat.ply`), without its sheets. The
/// forms touched.
pub fn install_doll(data: &Path, pool: &Path) -> std::io::Result<Vec<String>> {
    let races = race_forms(data);
    let keep = |form: &str| races.iter().any(|r| r == form);
    let global = pool.join("global");
    std::fs::create_dir_all(global.join("puppets"))?;
    let mut forms = install_puppets_of(data, &global.join("puppets"), &keep)?;
    forms.extend(install_bodies_of(data, &global.join("bodies"), &keep, true)?);
    forms.sort();
    forms.dedup();
    Ok(forms)
}

/// The figures pack (docs/crate-split-plan.md, S8): the playable races'
/// figures ([`race_forms`]) as a client's doll draws them -- each its own
/// folder, `global/bodies/<form>/`: puppet, thin mesh, weights, one cloud,
/// `doll.toml`, no sheets ([`install_doll`]'s) -- with its `pack.toml`,
/// into `out` (a figure no longer a race is taken out). The forms written.
pub fn figures_pack(data: &Path, out: &Path) -> std::io::Result<Vec<String>> {
    let races = race_forms(data);
    let keep = |form: &str| races.iter().any(|r| r == form);
    let bodies = out.join("global").join("bodies");
    std::fs::create_dir_all(&bodies)?;
    for dir in std::fs::read_dir(&bodies)?.flatten() {
        if dir.path().is_dir() && !keep(&dir.file_name().to_string_lossy()) {
            std::fs::remove_dir_all(dir.path())?;
        }
    }
    std::fs::write(out.join("pack.toml"), FIGURES_PACK_TOML)?;
    install_bodies_of(data, &bodies, &keep, true)
}

/// The figures pack's `pack.toml`.
const FIGURES_PACK_TOML: &str = r#"# The injury doll's figures: the playable races' modelled bodies, each
# its own folder (gs-studio --pack figures builds it from the data root).
id = "figures"
title = "Injury doll figures"
kind = "figures"
version = "1"
format = 1
"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// A client's doll gets the playable races' figures and nothing else:
    /// not a form whose doll.toml says no race, nor one with none.
    #[test]
    fn a_clients_doll_gets_the_races_alone() {
        let root = std::env::temp_dir().join(format!("gs_studio_doll_pool_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let data = root.join("data");
        for (form, doll) in [("half_elf_male", Some("race = \"Half-elf\"
variant = \"Male\"
")), ("grid_figure", Some("kind = \"extra\"
race = \"Half-elf\"
")), ("dwarf_male", None), ("odd_one", Some("kind = \"extra\"
"))] {
            let dir = data.join("meshes").join(form);
            std::fs::create_dir_all(dir.join("master")).unwrap();
            std::fs::write(dir.join("puppet.toml"), format!("family = \"{form}\"
")).unwrap();
            std::fs::write(dir.join("master/body.obj"), "v 0 0 0
").unwrap();
            std::fs::write(dir.join("master/splat.spz"), "spz").unwrap();
            if let Some(d) = doll {
                std::fs::write(dir.join("doll.toml"), d).unwrap();
            }
        }
        let pool = root.join("hydra_data");
        assert_eq!(install_doll(&data, &pool).unwrap(), ["grid_figure", "half_elf_male"]);
        let mut there: Vec<String> = std::fs::read_dir(pool.join("global/bodies")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        there.sort();
        assert_eq!(there, ["grid_figure", "half_elf_male"]);
        assert!(pool.join("global/puppets/half_elf_male.puppet.toml").is_file() && !pool.join("global/puppets/dwarf_male.puppet.toml").exists());
        assert!(pool.join("global/bodies/half_elf_male/splat.spz").is_file() && pool.join("global/bodies/half_elf_male/doll.toml").is_file());
        // the figure's folder is the whole figure: its puppet with it
        assert_eq!(std::fs::read_to_string(pool.join("global/bodies/half_elf_male/puppet.toml")).unwrap(), "family = \"half_elf_male\"\n");
        assert_eq!(injury_doll::source::DollSource::at(pool.join("global")).baked_toml("half_elf_male").as_deref(), Some("family = \"half_elf_male\"\n"));
        // its cloud is how a client draws it: no sheets, and old ones go
        std::fs::write(data.join("meshes/half_elf_male/master/color.png"), "png").unwrap();
        std::fs::write(pool.join("global/bodies/half_elf_male/normal.png"), "old").unwrap();
        install_doll(&data, &pool).unwrap();
        assert!(!pool.join("global/bodies/half_elf_male/color.png").exists() && !pool.join("global/bodies/half_elf_male/normal.png").exists());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_largest_body_is_installed_once_with_only_its_newest_cloud() {
        let root = std::env::temp_dir().join(format!("gs_studio_bodies_pool_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let data = root.join("data");
        for dir in ["meshes/half_elf/40k", "meshes/half_elf/master", "meshes/humanoid/ranger"] {
            std::fs::create_dir_all(data.join(dir)).unwrap();
        }
        for (f, text) in [("half_elf/puppet.toml", "family = \"half_elf\"\n"), ("half_elf/doll.toml", "label = \"Half-elf\"\n"), ("half_elf/master/body.obj", "v 0 0 0\n"), ("half_elf/master/color.png", "png"), ("half_elf/master/weights.txt", "w"), ("half_elf/master/splat.ply", "ply"), ("half_elf/40k/body.obj", "small"), ("humanoid/ranger/body.obj", "ranger")] {
            std::fs::write(data.join("meshes").join(f), text).unwrap();
        }
        let pool = root.join("pool/global/bodies");
        assert_eq!(install_bodies(&data, &pool).unwrap(), ["half_elf"], "the humanoid's ranger is not its own body");
        assert!(!pool.join("humanoid").exists());
        let dst = pool.join("half_elf");
        assert_eq!(std::fs::read_to_string(dst.join("body.obj")).unwrap(), "v 0 0 0\n", "the largest size");
        assert!(dst.join("splat.ply").is_file() && !dst.join("normal.png").exists());
        assert!(dst.join("doll.toml").is_file(), "how the doll shows it goes with it");
        assert!(install_bodies(&data, &pool).unwrap().is_empty(), "nothing changed");
        // an spz arrives: it is installed and the ply goes
        std::fs::write(data.join("meshes/half_elf/master/splat.spz"), "spz").unwrap();
        assert_eq!(install_bodies(&data, &pool).unwrap(), ["half_elf"]);
        assert!(dst.join("splat.spz").is_file() && !dst.join("splat.ply").exists());
        // retrained to a ply alone: the spz it had goes, so the new cloud is the one loaded
        std::fs::remove_file(data.join("meshes/half_elf/master/splat.spz")).unwrap();
        std::fs::write(data.join("meshes/half_elf/master/splat.ply"), "new ply").unwrap();
        assert_eq!(install_bodies(&data, &pool).unwrap(), ["half_elf"]);
        assert!(dst.join("splat.ply").is_file() && !dst.join("splat.spz").exists(), "no stale spz");
        // a thinned runtime mesh goes in under the plain name
        std::fs::write(data.join("meshes/half_elf/master/body_thin.obj"), "thin
").unwrap();
        std::fs::write(data.join("meshes/half_elf/master/weights_thin.txt"), "wt").unwrap();
        assert_eq!(install_bodies(&data, &pool).unwrap(), ["half_elf"]);
        assert_eq!(std::fs::read_to_string(dst.join("body.obj")).unwrap(), "thin
");
        assert_eq!(std::fs::read_to_string(dst.join("weights.txt")).unwrap(), "wt");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn bodies_are_found_by_form_and_size_and_their_puppets_installed() {
        let root = std::env::temp_dir().join(format!("gs_studio_bodies_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let data = root.join("data");
        for dir in ["meshes/half_elf/40k", "meshes/half_elf/master", "meshes/half_elf/150k", "meshes/ranger", "meshes/empty"] {
            std::fs::create_dir_all(data.join(dir)).unwrap();
        }
        for obj in ["meshes/half_elf/40k/body.obj", "meshes/half_elf/master/body.obj", "meshes/half_elf/150k/body.obj", "meshes/ranger/body.obj"] {
            std::fs::write(data.join(obj), "").unwrap();
        }
        std::fs::write(data.join("meshes/half_elf/puppet.toml"), "family = \"half_elf\"\n").unwrap();
        let found = scan(&data);
        assert_eq!(found.iter().map(|f| f.form.as_str()).collect::<Vec<_>>(), ["half_elf", "ranger"]);
        assert_eq!(found[0].sizes.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), ["master", "150k", "40k"]);
        assert_eq!(found[1].sizes, [(String::new(), data.join("meshes").join("ranger"))]);
        assert!(found[0].own_puppet && !found[1].own_puppet);
        let pool = root.join("pool");
        assert_eq!(install_puppets(&data, &pool).unwrap(), ["half_elf"]);
        assert_eq!(std::fs::read_to_string(pool.join("half_elf.puppet.toml")).unwrap(), "family = \"half_elf\"\n");
        // unchanged: nothing to install again
        assert!(install_puppets(&data, &pool).unwrap().is_empty());
        std::fs::remove_dir_all(&root).ok();
    }
}
