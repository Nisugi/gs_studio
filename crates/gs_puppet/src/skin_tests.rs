//! The smooth skin against the shipped humanoid.

use crate::draw::{Look, BOSS_OUTLINE};
use crate::math::{mul_mv, rot_y};
use crate::raster::Canvas;
use crate::{Camera, Pose, Puppet, Skin};

const SIZE: f32 = 300.0;

fn humanoid() -> Puppet {
    Puppet::from_baked_toml(crate::fixtures::HUMANOID).expect("shipped humanoid")
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
    // Round the arm's middle: how many pixels its wound marks change.
    let marked = |yaw: f32| {
        let bare = posed.skin(&project, &Look { yaw, ..Look::default() });
        let hurt = posed.skin(&project, &Look { yaw, wounds: &wounds, ..Look::default() });
        assert_eq!((bare.w, bare.h, bare.origin), (hurt.w, hurt.h, hurt.origin));
        let (at, _) = project(mul_mv(&rot_y(yaw), posed.point(arm, st.c)));
        assert!(pixel(&bare, at).expect("the arm is in frame")[3] > 200, "yaw {yaw}: the arm's middle is covered");
        let (cx, cy) = (at[0] - bare.origin[0], at[1] - bare.origin[1]);
        let mut n = 0;
        for y in (cy as i64 - 25).max(0)..(cy as i64 + 25).min(bare.h as i64) {
            for x in (cx as i64 - 12).max(0)..(cx as i64 + 12).min(bare.w as i64) {
                let i = (y as usize * bare.w + x as usize) * 4;
                if (0..3).any(|k| bare.rgba[i + k].abs_diff(hurt.rgba[i + k]) > 10) {
                    n += 1;
                }
            }
        }
        n
    };
    // Yaw 90 turns the figure's right side (-x) toward the camera.
    assert!(marked(90.0) > 20, "near arm shows its wound: {}", marked(90.0));
    // (a rank-2 wound's blood reaches the arm's edge, which peeks past the
    // trunk by a pixel or two)
    assert!(marked(-90.0) <= 4, "far arm is behind the trunk: {}", marked(-90.0));
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
    let tex = crate::Texture { layout, rgba, normal: None };
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

/// A lamp lights the side of the figure that faces it, more where it is
/// nearer; one past its reach adds nothing; and its light is ADDED (alpha
/// 0), never painted over the skin.
#[test]
fn a_lamp_lights_the_side_that_faces_it() {
    let p = humanoid();
    let posed = p.pose(&Pose::default());
    let cam = camera(&p);
    let project = |q| cam.project(q);
    let look = Look { outline_px: 0.0, ..Look::default() };
    let (skin, surface) = posed.skin_surface(&project, &look, None, crate::Effects::default());
    assert_eq!((surface.w, surface.h), (skin.w, skin.h));
    // Screen x of the figure's middle, in the skin's own pixels.
    let (mid, _) = project([0.0, p.height * 0.5, 0.0]);
    let mid = mid[0] - skin.origin[0];
    let lamp = |x: f32| crate::Lamp { pos: [x, p.height * 0.5, 1.0], rgb: [255, 160, 74], reach: 8.0, strength: 1.0, beam: None };
    let sides = |light: &[u8]| {
        let (mut left, mut right) = (0u64, 0u64);
        for (i, c) in light.chunks_exact(4).enumerate() {
            assert_eq!(c[3], 0, "lamp light is added, not laid over");
            if ((i % skin.w) as f32) < mid {
                left += c[0] as u64;
            } else {
                right += c[0] as u64;
            }
        }
        (left, right)
    };
    // World -x is screen-left (the creature's right).
    let (left, right) = sides(&surface.lamp_light(&[lamp(-3.0)], &skin));
    assert!(left > right * 3 / 2 && left > 0, "a lamp on the left lights the left: {left} vs {right}");
    let (left, right) = sides(&surface.lamp_light(&[lamp(3.0)], &skin));
    assert!(right > left * 3 / 2, "and on the right, the right: {left} vs {right}");
    let (near, _) = sides(&surface.lamp_light(&[lamp(-1.5)], &skin));
    let (far, _) = sides(&surface.lamp_light(&[lamp(-5.0)], &skin));
    assert!(near > far, "nearer is brighter: {near} vs {far}");
    let gone = surface.lamp_light(&[lamp(-20.0)], &skin);
    assert!(gone.iter().all(|v| *v == 0), "out of reach, nothing");
    // A lamp behind the figure lights little of what faces the viewer.
    let behind = crate::Lamp { pos: [0.0, p.height * 0.5, -3.0], ..lamp(0.0) };
    let front = crate::Lamp { pos: [0.0, p.height * 0.5, 3.0], ..lamp(0.0) };
    let sum = |l: crate::Lamp| surface.lamp_light(&[l], &skin).iter().map(|v| *v as u64).sum::<u64>();
    assert!(sum(front) > sum(behind) * 3, "front {} vs behind {}", sum(front), sum(behind));
}

/// A lamp beside a figure throws its shadow away from it along the ground,
/// longer the lower the flame; nothing past the lamp's reach, and nothing
/// from a lamp whose beam points away.
#[test]
fn a_lamp_casts_the_figure_s_shadow_away_from_it() {
    let p = humanoid();
    let posed = p.pose(&Pose::default());
    let cam = camera(&p);
    let project = |q| cam.project(q);
    let caster = posed.shadow_caster(&Look::default());
    assert!(!caster.is_empty());
    let lamp = |x: f32, y: f32| crate::Lamp { pos: [x, y, 0.0], rgb: [255, 160, 74], reach: 20.0, strength: 1.0, beam: None };
    // The shadow's centre of mass, in pixels, and how far it spreads.
    let spread = |s: &crate::Shadow| {
        let (mut sum, mut n, mut lo, mut hi) = (0.0f32, 0.0f32, f32::MAX, f32::MIN);
        for (i, a) in s.alpha.iter().enumerate() {
            if *a > 8 {
                let x = s.origin[0] + (i % s.w) as f32;
                sum += x * *a as f32;
                n += *a as f32;
                lo = lo.min(x);
                hi = hi.max(x);
            }
        }
        (sum / n.max(1.0), hi - lo)
    };
    let (foot, _) = project([0.0, 0.0, 0.0]);
    // Flames above the head (the figure is 6 ft), one well above, one just.
    let high = caster.cast(&project, &[lamp(-4.0, 12.0)], 2.0).expect("a shadow");
    let (centre, long) = spread(&high);
    assert!(centre > foot[0], "a lamp on the left throws it right: {centre} vs the foot at {}", foot[0]);
    let low = caster.cast(&project, &[lamp(-4.0, 7.0)], 2.0).expect("a shadow");
    let (_, longer) = spread(&low);
    assert!(longer > long * 1.3, "a lower flame throws it further: {longer} vs {long}");
    assert!(caster.cast(&project, &[lamp(-40.0, 3.0)], 2.0).is_none(), "out of reach, none");
    let away = crate::Lamp { beam: Some(crate::Beam { dir: [-1.0, 0.0, 0.0], spread: 60.0, soft: 10.0 }), ..lamp(-4.0, 3.0) };
    assert!(caster.cast(&project, &[away], 2.0).is_none(), "a beam pointed away casts none");
    let toward = crate::Lamp { beam: Some(crate::Beam { dir: [1.0, 0.0, 0.0], spread: 60.0, soft: 10.0 }), ..away };
    assert!(caster.cast(&project, &[toward], 2.0).is_some(), "pointed at it, it does");
}

/// Rasterised in bands on several threads, a skin is the same, pixel for
/// pixel, as on one -- wounds, nerves and a kept surface included.
#[test]
fn a_skin_on_many_threads_is_the_skin_on_one() {
    let p = humanoid();
    let posed = p.pose(&Pose::default());
    let cam = camera(&p);
    let project = |q| cam.project(q);
    let wounds = [("chest", 2u8), ("nsys", 3), ("back", 2), ("leftArm", 3)];
    let look = Look { yaw: 30.0, wounds: &wounds, ..Look::default() };
    let on = |threads: usize| {
        crate::skin::THREADS_FOR_TEST.with(|t| t.set(Some(threads)));
        let (skin, surface) = posed.skin_surface(&project, &look, None, crate::Effects::default());
        crate::skin::THREADS_FOR_TEST.with(|t| t.set(None));
        (skin, surface.glow().to_vec())
    };
    let (one, glow_one) = on(1);
    let (many, glow_many) = on(7);
    assert!(!one.is_empty());
    assert_eq!((one.w, one.h, one.origin), (many.w, many.h, many.origin));
    assert!(one.rgba == many.rgba, "the pixels differ");
    assert!(glow_one == glow_many, "the glow differs");
}

/// Fresh blood reaches the kept surface as wetness -- a wound's cuts, a
/// rank-2 wound's pooled blood more, a bleeding part's run -- and a scar
/// alone stays dry.
#[test]
fn wet_blood_reaches_the_surface() {
    let p = humanoid();
    let posed = p.pose(&Pose::default());
    let cam = camera(&p);
    let project = |q| cam.project(q);
    let wet = |wounds: &[(&str, u8)], effects: crate::Effects| {
        let look = Look { wounds, ..Look::default() };
        let (_, surface) = posed.skin_surface(&project, &look, None, effects);
        surface.wet().iter().filter(|w| **w > 8).count()
    };
    let look = Look { scars: &[("chest", 3)], ..Look::default() };
    let (_, scarred) = posed.skin_surface(&project, &look, None, crate::Effects::default());
    assert_eq!(scarred.wet().iter().filter(|w| **w > 8).count(), 0, "a scar is dry");
    let cut = wet(&[("chest", 1), ("abdomen", 1)], crate::Effects::default());
    let pooled = wet(&[("chest", 2), ("abdomen", 2)], crate::Effects::default());
    assert!(cut > 0 && pooled > cut, "cuts are wet, a rank-2 wound's pool more: {cut} {pooled}");
    let bleeding = wet(&[("chest", 2), ("abdomen", 2)], crate::Effects { bleeding: true, ..Default::default() });
    assert!(bleeding > pooled, "bleeding runs wet: {bleeding} vs {pooled}");
}
