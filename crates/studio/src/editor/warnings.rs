//! What is wrong (or not finished) with a rig, in words.

use rig::{Baked, Pt, Rig, PARTS};
use rig_bake::Mask;

/// A part smaller than this share of the silhouette is suspicious.
pub const MIN_SHARE: f32 = 0.005;

/// True when `p` is on (or within one pixel of) the filled silhouette. The
/// slack keeps a joint clicked right on the edge from flickering.
pub fn inside_silhouette(mask: &Mask, p: Pt) -> bool {
    let (gx, gy) = mask.grid.to_grid(p);
    let (cx, cy) = (gx.floor() as i64, gy.floor() as i64);
    (-1..=1).any(|dy| {
        (-1..=1).any(|dx| {
            let (x, y) = (cx + dx, cy + dy);
            x >= 0
                && y >= 0
                && (x as usize) < mask.grid.w
                && (y as usize) < mask.grid.h
                && mask.inside[y as usize * mask.grid.w + x as usize]
        })
    })
}

/// Joints placed off the silhouette, in name order.
pub fn joints_outside(rig: &Rig, mask: &Mask) -> Vec<String> {
    rig.joints.iter().filter(|(_, p)| !inside_silhouette(mask, **p)).map(|(n, _)| n.clone()).collect()
}

/// Joints no bone's chain uses. Eye joints are not expected on a bone.
pub fn unused_joints(rig: &Rig) -> Vec<String> {
    rig.joint_order
        .iter()
        .filter(|j| *j != rig.eyes.joint(true) && *j != rig.eyes.joint(false))
        .filter(|j| !rig.bones.iter().any(|b| b.joints.contains(j)))
        .cloned()
        .collect()
}

pub fn warnings(rig: &Rig, baked: &Baked, mask: &Mask) -> Vec<String> {
    let mut out = Vec::new();

    let mut unplaced = Vec::new();
    for (i, bone) in rig.bones.iter().enumerate() {
        if bone.joints.len() < 2 {
            out.push(format!(
                "bone '{}' has {} joint(s) in its chain; it needs at least two",
                bone.name,
                bone.joints.len()
            ));
        } else if rig.bone_points(bone).len() < 2 {
            unplaced.push(bone.name.as_str());
        } else if bone.part.is_none() {
            out.push(format!("bone '{}' has no part, so it labels no pixels", bone.name));
        } else if baked.bone_pixels.get(i).copied().unwrap_or(0) == 0 {
            out.push(format!("bone '{}' owns no pixels (is it on the silhouette?)", bone.name));
        }
    }
    if !unplaced.is_empty() {
        out.push(format!("{} bone(s) not placed yet: {}", unplaced.len(), unplaced.join(", ")));
    }
    out.extend(rig.hierarchy_issues().iter().map(ToString::to_string));

    let unused = unused_joints(rig);
    if !unused.is_empty() {
        out.push(format!("joint(s) used by no bone: {}", unused.join(", ")));
    }
    for (slot, joint) in [("head", &rig.anchors.head), ("mouth", &rig.anchors.mouth), ("saddle", &rig.anchors.saddle)] {
        if !rig.joint_order.contains(joint) {
            out.push(format!("anchor '{slot}' names no joint"));
        }
    }
    for (eye, pixels) in &baked.eye_pixels {
        if *pixels == 0 {
            out.push(format!("eye '{eye}' covers no head pixels (is it on the head?)"));
        }
    }

    for part in PARTS {
        let baked_bone = rig.bones.iter().enumerate().any(|(i, b)| {
            b.part.as_deref() == Some(part) && baked.bone_radii_ft.get(i).is_some_and(|r| r.is_some())
        });
        let share = baked.share(part);
        if baked_bone && share < MIN_SHARE {
            out.push(format!("part '{part}' covers only {:.2}% of the silhouette", share * 100.0));
        }
    }

    let outside = joints_outside(rig, mask);
    if !outside.is_empty() {
        out.push(format!("joint(s) outside the silhouette: {}", outside.join(", ")));
    }
    if baked.map.orphans > 0 {
        out.push(format!(
            "{} orphan pixel(s): detached islands no bone reaches; they took the nearest bone's part",
            baked.map.orphans
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::{bake_mask, template, Template, View};
    use rig_bake::{rasterize, Outline};

    /// A 2 x 6 ft block standing on the ground, centred on x = 0.
    fn block() -> Mask {
        rasterize(&Outline { subpaths: vec![vec![[-1.0, 0.0], [1.0, 0.0], [1.0, 6.0], [-1.0, 6.0]]] }, 120)
    }

    fn serpent() -> (Rig, Template) {
        let t = template("serpentine").unwrap();
        (t.new_rig("serpent", View::Side, 1.0, "diameter"), t)
    }

    fn place_chain(rig: &mut Rig, t: &Template) {
        // Head at the top, tail at the bottom, straight down the middle.
        let chain: Vec<&String> = t.joints.iter().filter(|j| *j != "eye_r").collect();
        for (k, j) in chain.iter().enumerate() {
            let y = 5.8 - 5.6 * k as f32 / (chain.len() - 1) as f32;
            rig.joints.insert((*j).clone(), [0.0, y]);
        }
    }

    #[test]
    fn a_fresh_rig_only_reports_unplaced_bones() {
        let (rig, _) = serpent();
        let mask = block();
        let w = warnings(&rig, &bake_mask(&rig, &mask), &mask);
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].starts_with("5 bone(s) not placed yet: skull, neck"), "{w:?}");
    }

    #[test]
    fn a_complete_rig_is_clean() {
        let (mut rig, t) = serpent();
        place_chain(&mut rig, &t);
        let mask = block();
        let baked = bake_mask(&rig, &mask);
        let w = warnings(&rig, &baked, &mask);
        assert!(w.is_empty(), "{w:?}");
        // The real per-bone counts: every bone owns something, nothing is shared.
        assert!(baked.bone_pixels.iter().all(|&n| n > 0), "{:?}", baked.bone_pixels);
        assert_eq!(baked.bone_pixels.iter().sum::<usize>(), baked.filled());
    }

    #[test]
    fn outside_joints_and_pixel_less_bones_are_named() {
        let (mut rig, t) = serpent();
        place_chain(&mut rig, &t);
        // Drag the whole skull off the block.
        rig.joints.insert("snout".into(), [3.0, 5.8]);
        rig.joints.insert("skull_base".into(), [2.0, 5.5]);
        let mask = block();
        let w = warnings(&rig, &bake_mask(&rig, &mask), &mask);
        assert!(w.iter().any(|m| m.starts_with("bone 'skull' owns no pixels")), "{w:?}");
        assert!(w.iter().any(|m| m == "joint(s) outside the silhouette: skull_base, snout"), "{w:?}");
    }

    #[test]
    fn tiny_parts_and_partless_bones_are_flagged() {
        let (mut rig, t) = serpent();
        place_chain(&mut rig, &t);
        // Squash the neck to a sliver with a tiny explicit radius.
        let neck_end = rig.joints["neck_end"];
        rig.joints.insert("skull_base".into(), [neck_end[0], neck_end[1] + 0.02]);
        let neck = rig.bones.iter().position(|b| b.name == "neck").unwrap();
        rig.bones[neck].radius_ft = Some(0.01);
        let mask = block();
        let w = warnings(&rig, &bake_mask(&rig, &mask), &mask);
        assert!(w.iter().any(|m| m.starts_with("part 'neck' covers only")), "{w:?}");

        let chest = rig.bones.iter().position(|b| b.name == "fore_body").unwrap();
        rig.bones[chest].part = None;
        let w = warnings(&rig, &bake_mask(&rig, &mask), &mask);
        assert!(w.iter().any(|m| m.starts_with("bone 'fore_body' has no part")), "{w:?}");
    }

    #[test]
    fn structure_problems_are_reported() {
        let (mut rig, t) = serpent();
        place_chain(&mut rig, &t);
        rig.joints.insert("eye_r".into(), [3.0, 3.0]); // off the block, off the head
        rig.joint_order.push("spare".into());
        rig.anchors.saddle = String::new();
        let neck = rig.bone_index("neck").unwrap();
        rig.bones[neck].parent = Some("ghost".into());
        let skull = rig.bone_index("skull").unwrap();
        rig.bones[skull].parent = None;
        let hind = rig.bone_index("hind_body").unwrap();
        rig.bones[hind].joints.truncate(1);
        let mask = block();
        let w = warnings(&rig, &bake_mask(&rig, &mask), &mask);
        for expected in [
            "bone 'hind_body' has 1 joint(s) in its chain; it needs at least two",
            "bone 'neck' has a parent 'ghost' that does not exist",
            "2 root bones (no parent): skull, mid_body",
            "joint(s) used by no bone: back_1, back_2, tail_mid, tail_tip, spare",
            "anchor 'saddle' names no joint",
            "eye 'rightEye' covers no head pixels (is it on the head?)",
        ] {
            assert!(w.iter().any(|m| m == expected), "missing {expected:?} in {w:#?}");
        }
    }

    #[test]
    fn the_edge_has_a_pixel_of_slack() {
        let mask = block();
        assert!(inside_silhouette(&mask, [0.0, 3.0]));
        assert!(inside_silhouette(&mask, [1.0, 6.0]));
        assert!(!inside_silhouette(&mask, [1.2, 3.0]));
        assert!(!inside_silhouette(&mask, [0.0, -0.5]));
    }
}
