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
