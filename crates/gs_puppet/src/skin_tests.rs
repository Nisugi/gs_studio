//! The smooth skin against the shipped humanoid.

use crate::draw::{Look, BOSS_OUTLINE, WOUND_COLORS};
use crate::math::{mul_mv, rot_y};
use crate::raster::Canvas;
use crate::{Camera, Pose, Puppet, Skin};

const SIZE: f32 = 300.0;

fn humanoid() -> Puppet {
    Puppet::from_baked_toml(include_str!("../../gs_field/data/puppets/humanoid.puppet.toml")).expect("shipped humanoid")
}

fn camera(p: &Puppet) -> Camera {
    Camera::framing(p.height, [SIZE, SIZE])
}

fn pixel(skin: &Skin, at: [f32; 2]) -> Option<[u8; 4]> {
    let (x, y) = ((at[0] - skin.origin[0]).floor(), (at[1] - skin.origin[1]).floor());
    if x < 0.0 || y < 0.0 || x as usize >= skin.w || y as usize >= skin.h {
        return None;
    }
    let i = (y as usize * skin.w + x as usize) * 4;
    Some([skin.rgba[i], skin.rgba[i + 1], skin.rgba[i + 2], skin.rgba[i + 3]])
}

#[test]
fn the_skin_covers_what_the_shapes_cover() {
    let p = humanoid();
    let posed = p.pose(&Pose::default());
    let cam = camera(&p);
    let project = |q| cam.project(q);
    let look = Look { outline_px: 0.0, min_station_px: 0.0, ..Look::default() };
    let skin = posed.skin(&project, &look);
    assert!(!skin.is_empty());
    let skin_px = skin.rgba.chunks_exact(4).filter(|c| c[3] >= 128).count();

    // The shapes, all white on black, in the same pixel space.
    let mut c = Canvas::new(SIZE as usize, SIZE as usize, 1, [0, 0, 0]);
    let mut prims = posed.draw_list(&project, &look);
    for prim in &mut prims {
        prim.color = [255, 255, 255, 255];
    }
    c.draw(&prims);
    let shape_px = c.to_rgba8().chunks_exact(4).filter(|c| c[0] >= 128).count();
    let ratio = skin_px as f32 / shape_px as f32;
    assert!((0.9..1.1).contains(&ratio), "skin {skin_px} px vs shapes {shape_px} px");
}

#[test]
fn a_near_arm_hides_the_trunk_and_a_far_one_is_hidden_by_it() {
    let p = humanoid();
    let posed = p.pose(&Pose::default());
    let cam = camera(&p);
    let project = |q| cam.project(q);
    let arm = p.segments.iter().position(|s| s.part.as_deref() == Some("rightArm")).expect("an arm");
    let st = p.segments[arm].stations[p.segments[arm].stations.len() / 2];
    let wounds = [("rightArm", 2u8)];
    // Orange is much less green for its red than the body colour.
    let orange = |c: [u8; 4]| (c[1] as f32 / c[0].max(1) as f32) < 0.72;
    let [w, body] = [WOUND_COLORS[1], crate::BODY_DEFAULT].map(|c| c[1] as f32 / c[0] as f32);
    assert!(w < 0.72 && body > 0.72, "the test's colour split no longer separates them");

    let shows = |yaw: f32| {
        let look = Look { yaw, wounds: &wounds, ..Look::default() };
        let skin = posed.skin(&project, &look);
        let (at, _) = project(mul_mv(&rot_y(yaw), posed.point(arm, st.c)));
        let c = pixel(&skin, at).expect("the arm is in frame");
        assert!(c[3] > 200, "yaw {yaw}: the arm's middle is covered");
        orange(c)
    };
    // Yaw 90 turns the figure's right side (-x) toward the camera.
    assert!(shows(90.0), "near arm shows its wound");
    assert!(!shows(-90.0), "far arm is behind the trunk");
}

#[test]
fn a_boss_gets_its_gold_contour_and_the_dead_lose_their_eyes() {
    let p = humanoid();
    let posed = p.pose(&Pose::default());
    let cam = camera(&p);
    let project = |q| cam.project(q);
    let count = |skin: &Skin, want: [u8; 3], tol: i32| {
        skin.rgba
            .chunks_exact(4)
            .filter(|c| c[3] > 200 && (0..3).all(|k| (c[k] as i32 - want[k] as i32).abs() <= tol))
            .count()
    };
    let plain = posed.skin(&project, &Look::default());
    let boss = posed.skin(&project, &Look { boss_outline: true, ..Look::default() });
    assert_eq!(count(&plain, BOSS_OUTLINE, 8), 0);
    assert!(count(&boss, BOSS_OUTLINE, 8) > 50);

    let eye = [24, 18, 14];
    let dead = posed.skin(&project, &Look { dead: true, ..Look::default() });
    assert!(count(&plain, eye, 6) > 0, "the living look out");
    assert_eq!(count(&dead, eye, 6), 0);
}

#[test]
fn every_surface_point_lands_in_its_own_patch() {
    let p = humanoid();
    let layout = crate::UvLayout::new(&p, 60.0);
    assert_eq!(layout.patches.len(), p.segments.len());
    for (i, s) in p.segments.iter().enumerate() {
        let q = &layout.patches[i];
        for k in 0..s.stations.len() {
            for j in 0..16 {
                let theta = j as f32 / 16.0 * std::f32::consts::TAU;
                let at = layout.locate(i, k as f32, theta).expect("addressable");
                let inside = |v: f32, lo: f32, size: f32| v >= lo - 0.5 && v <= lo + size + 0.5;
                assert!(
                    inside(at[0], q.origin[0], q.size[0]) && inside(at[1], q.origin[1], q.size[1]),
                    "{} station {k} angle {theta}: {at:?} outside its patch",
                    s.name
                );
            }
        }
    }
    // Patches never overlap.
    for (a, qa) in layout.patches.iter().enumerate() {
        for qb in &layout.patches[a + 1..] {
            let apart = qa.origin[0] + qa.size[0] <= qb.origin[0]
                || qb.origin[0] + qb.size[0] <= qa.origin[0]
                || qa.origin[1] + qa.size[1] <= qb.origin[1]
                || qb.origin[1] + qb.size[1] <= qa.origin[1];
            assert!(apart, "patches {} and {} overlap", qa.segment, qb.segment);
        }
    }
}

#[test]
fn a_limbs_seam_runs_down_its_inside() {
    let p = humanoid();
    let layout = crate::UvLayout::new(&p, 60.0);
    for (i, s) in p.segments.iter().enumerate() {
        let side = (s.head[0] + s.tail[0]) * 0.5;
        if !matches!(s.part.as_deref(), Some("rightArm" | "leftArm" | "rightLeg" | "leftLeg")) || side.abs() < 0.1 {
            continue;
        }
        let seam = layout.patches[i].seam;
        let dir = crate::math::add(crate::math::scale(s.axis_w, seam.cos()), crate::math::scale(s.axis_d, seam.sin()));
        assert!(dir[0] * side < 0.0, "{}: seam faces {dir:?}, not toward the midline", s.name);
    }
}

#[test]
fn paint_on_a_patch_shows_on_its_segment_only() {
    let p = humanoid();
    let posed = p.pose(&Pose::default());
    let cam = camera(&p);
    let project = |q| cam.project(q);
    let layout = crate::UvLayout::new(&p, 40.0);
    let chest = p.segments.iter().position(|s| s.part.as_deref() == Some("chest")).expect("a chest");
    let head = p.segments.iter().position(|s| s.part.as_deref() == Some("head")).expect("a head");
    // Paint the chest patch pure blue; leave the rest of the sheet blank.
    let mut rgba = vec![0u8; layout.w * layout.h * 4];
    let q = &layout.patches[chest];
    for y in q.origin[1] as usize..(q.origin[1] + q.size[1]) as usize {
        for x in q.origin[0] as usize..(q.origin[0] + q.size[0]) as usize {
            rgba[(y * layout.w + x) * 4..][..4].copy_from_slice(&[0, 0, 255, 255]);
        }
    }
    let tex = crate::Texture { layout, rgba };
    let look = Look { outline_px: 0.0, ..Look::default() };
    let skin = posed.skin_textured(&project, &look, Some(&tex));
    let at = |seg: usize| {
        let s = &p.segments[seg];
        let st = s.stations[s.stations.len() / 2];
        // The front of the ring: toward the camera.
        project(posed.point(seg, crate::math::add(st.c, [0.0, 0.0, st.d.max(st.w) * 0.5])))
    };
    let blue = |c: [u8; 4]| c[2] as i32 > c[0] as i32 + 60;
    assert!(blue(pixel(&skin, at(chest).0).unwrap()), "the chest wears its paint");
    assert!(!blue(pixel(&skin, at(head).0).unwrap()), "the head does not");
}
