// A modelled body's splat cloud on the GPU (skin_gpu.rs `draw_body`, its cloud): the
// cloud is uploaded once in its rest pose (gs_puppet::mesh::SplatBody) and
// every Gaussian is moved, projected and shaded here from the puppet's
// segment transforms (gs_puppet::mesh::BodyFrame). Per frame the CPU sends
// those transforms and the ORDER to draw the splats in (far to near: one
// index each, found from the transforms alone) -- a fitted cloud is many
// faint Gaussians over each other, and is only right blended in that order.
//
//   front   each splat's core: the front-most core per pixel -- its depth,
//           normal, glow and point (the surface; a splat far behind it is
//           inside)
//   blend   each splat's reach, far to near, premultiplied "over"
//   final   un-premultiplied; under a core, opaque; the layer the v2 pass
//           reads (albedo, lit, geo) and the plain colours
//
// splat_gpu.wgsl is the same three passes for a cloud the CPU projected
// (the ring puppet's own).

struct U {
    size: vec4<f32>,      // w, h, pad, m_per_ft
    frame: vec4<f32>,     // the image's origin (screen px), nearest depth, depth range
    to_cam: vec4<f32>,    // toward the viewer (turned world); w: reach (x the splat's radius)
    light: vec4<f32>,     // toward the light (turned world); w: the surface band
    floor: vec4<f32>,     // the doll's floor (linear); w: samples a pixel side (front and blend run at that)
    proj: array<vec4<f32>, 3>,     // turned world -> (x * depth, y * depth, depth)
    bones: array<vec4<f32>, 192>,  // per segment, rest -> turned world: 3 rows
    // a hit arriving: [0..3) the ages of its frames (gs_puppet::layers::
    // ARRIVAL_AGES), [3].x the hit's age now (< 0: none), [3].y the frames;
    // [3].z a running clock, seconds (glints and shimmer: skin_gpu `clock`)
    arrive: array<vec4<f32>, 4>,
};

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var t_front_n: texture_2d<f32>;
@group(0) @binding(2) var t_front_pos: texture_2d<f32>;
@group(0) @binding(3) var t_acc: texture_2d<f32>;
@group(0) @binding(4) var<storage, read> splats: array<Inst>;
@group(0) @binding(5) var<storage, read> order: array<u32>;

const FAR: f32 = 60000.0;
// gs_puppet::splat::CORE_M and REACH_M: a splat's core, and its reach
const CORE_M: f32 = 2.25;
// the facing is smoothed over this many pixels round, among the cores no
// further than this (feet) from the pixel's own (fs_final)
const SMOOTH_PX: i32 = 5;
const SMOOTH_FT: f32 = 0.08;
// added to a core's glow where it keeps its own facing (fs_front -> fs_final)
const OWN_FACING: f32 = 4.0;
// and this for each eighth of its rim (an invisible figure's skin): the
// glow channel of the front normal (Rgba16Float) carries all three
const RIM_STEP: f32 = 8.0;
const REACH_M: f32 = 9.0;
// gs_puppet::splat::RIM_FROM and RIM_TO: where a rim splat starts to show,
// and is whole, by how edge-on it is
const RIM_FROM: f32 = 0.3;
const RIM_TO: f32 = 0.8;
// an invisible figure's outline (fs_final): how far round a pixel (output
// pixels) is looked at for the figure's edge, and how far off another
// surface is (feet) to count as one (a limb in front of the trunk)
const RIM_PX: f32 = 4.0;
const RIM_GAP_FT: f32 = 0.3;
const LIGHT_ABOVE: f32 = 0.62;
const DARK_BELOW: f32 = -0.08;
// field_gpu::WET_DRY_LEN and WET_SPAN: how wet a layer's surface is rides
// in its geo normal's length (field_gpu::wet_normal)
const WET_DRY_LEN: f32 = 0.88;
const WET_SPAN: f32 = 0.3;

// one splat at rest (144 bytes)
struct Inst {
    c: vec4<f32>,       // its centre; w: glow
    ax0: vec4<f32>,     // w: 1 a film (ice, a web): never the surface
    ax1: vec4<f32>,     // w: 0 always shown; k + 1 a hit's arrival frame k; -1 thrown, -2 rising (see `thrown`)
    ax2: vec4<f32>,     // w: 1 lit by its own facing (built on the body), 0 skin
    color: vec4<f32>,   // rgb 0..1, alpha
    n: vec4<f32>,       // its facing; w: how wet (0..1)
    bones: vec4<u32>,
    shares: vec4<f32>,
    fx: vec4<f32>,      // x: its rim (gs_puppet::splat::Gaussian::rim)
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) inv: vec4<f32>,
    @location(2) @interpolate(flat) z: f32,
    @location(3) @interpolate(flat) rgb: vec3<f32>,
    @location(4) @interpolate(flat) alpha: f32,
    @location(5) @interpolate(flat) pos: vec3<f32>,
    @location(6) @interpolate(flat) n: vec3<f32>,
    @location(7) @interpolate(flat) glow: f32,
    @location(8) @interpolate(flat) solid: f32,   // its opacity as made
    @location(9) @interpolate(flat) wet: f32,
};

// gs_puppet::layers::FRESH_SECS and FALL_FT_S2
const FRESH_SECS: f32 = 4.0;
const FALL_FT_S2: f32 = 24.0;

// A splat as it is now, at rest: as made, or what a hit threw at its age.
struct Dot {
    ok: bool,
    c: vec3<f32>,
    a0: vec3<f32>,
    a1: vec3<f32>,
    a2: vec3<f32>,
    rgb: vec3<f32>,
    alpha: f32,
    glow: f32,
    n: vec3<f32>,
    wet: f32,
    film: bool,
};

fn as_made(i: Inst) -> Dot {
    return Dot(true, i.c.xyz, i.ax0.xyz, i.ax1.xyz, i.ax2.xyz, i.color.rgb, i.color.a, i.c.w, i.n.xyz, i.n.w, i.ax0.w > 0.5);
}

fn frame_age(k: u32) -> f32 {
    return u.arrive[k / 4u][k % 4u];
}

// How much of arrival frame k shows at the hit's age: the two frames either
// side of it blended; no hit, the last frame (the wound as it stays).
fn frame_weight(k: u32) -> f32 {
    let n = u32(u.arrive[3].y + 0.5);
    let age = u.arrive[3].x;
    if (n == 0u) { return 1.0; }
    if (age < 0.0) { return select(0.0, 1.0, k + 1u == n); }
    var lo = 0u;
    for (var j = 1u; j < n; j = j + 1u) {
        if (frame_age(j) <= age) { lo = j; }
    }
    if (lo + 1u >= n) { return select(0.0, 1.0, k == lo); }
    let f = clamp((age - frame_age(lo)) / max(frame_age(lo + 1u) - frame_age(lo), 1e-4), 0.0, 1.0);
    if (k == lo) { return 1.0 - f; }
    if (k == lo + 1u) { return f; }
    return 0.0;
}

// What a hit threw, `age` seconds on: gs_puppet::layers::Particle::at, line
// for line. c: where it left from; ax0: how fast; ax1: size, when it leaves
// (rising), how long it lives (rising) or when it lands (thrown); ax2.x: 0
// no glow, 1 an ember, 2 a spark; ax2.y: the ground it lands on.
fn thrown(i: Inst, age: f32) -> Dot {
    var d = Dot(false, vec3<f32>(0.0), vec3<f32>(0.0), vec3<f32>(0.0), vec3<f32>(0.0), vec3<f32>(0.0), 0.0, 0.0, vec3<f32>(0.0), 0.0, false);
    if (age < 0.0) { return d; }
    let start = i.c.xyz;
    let v = i.ax0.xyz;
    let r = i.ax1.x;
    let mode = i.ax2.x;
    let glows = mode > 0.5;
    if (i.ax1.w < -1.5) {
        let life = i.ax1.z;
        let t = age - i.ax1.y;
        let fade = smoothstep(0.0, 0.1, t) * (1.0 - smoothstep(0.3 * life, life, t));
        if (fade <= 0.02) { return d; }
        let grown = r * (1.0 + t / life);
        d.ok = true;
        d.c = start + v * t;
        d.a0 = vec3<f32>(grown, 0.0, 0.0);
        d.a1 = vec3<f32>(0.0, grown, 0.0);
        d.a2 = vec3<f32>(0.0, 0.0, grown);
        d.rgb = i.color.rgb;
        d.alpha = i.color.a * fade;
        d.glow = select(0.0, 0.5 * fade, glows);
        return d;
    }
    let land = i.ax1.z;
    let stale = 1.0 - smoothstep(FRESH_SECS - 1.2, FRESH_SECS, age);
    var glow = 0.0;
    var fade = stale;
    if (mode > 1.5) {
        glow = 1.0;
        fade = 1.0 - smoothstep(0.1, 0.45, age);
    } else if (glows) {
        glow = exp(-age);
    }
    if (fade <= 0.02) { return d; }
    d.rgb = i.color.rgb;
    if (glows) { d.rgb = mix(vec3<f32>(70.0, 34.0, 24.0) / 255.0, i.color.rgb, clamp(glow, 0.0, 1.0)); }
    d.alpha = i.color.a * fade;
    if (age < land) {
        d.c = vec3<f32>(start.x + v.x * age, start.y + v.y * age - 0.5 * FALL_FT_S2 * age * age, start.z + v.z * age);
        let vel = vec3<f32>(v.x, v.y - FALL_FT_S2 * age, v.z);
        let along = select(vec3<f32>(0.0, -1.0, 0.0), normalize(vel), length(vel) > 1e-9);
        let side = cross(along, vec3<f32>(0.0, 1.0, 0.0));
        let across = select(vec3<f32>(1.0, 0.0, 0.0), normalize(side), length(side) > 1e-9);
        d.a0 = along * (r + length(vel) * 0.002);
        d.a1 = across * r;
        d.a2 = cross(along, across) * r;
        d.glow = glow;
        d.ok = true;
        return d;
    }
    if (mode > 1.5) { return d; }
    d.c = vec3<f32>(start.x + v.x * land, i.ax2.y, start.z + v.z * land);
    d.a0 = vec3<f32>(r * 1.7, 0.0, 0.0);
    d.a1 = vec3<f32>(0.0, 0.0, r * 1.7);
    d.a2 = vec3<f32>(0.0, r * 0.25, 0.0);
    d.n = vec3<f32>(0.0, 1.0, 0.0);
    d.glow = glow * 0.8;
    d.ok = true;
    return d;
}

// A glint drifting up off an invisible figure, or a wisp of shadow off a
// hidden one (ax2.x 3: dark, a film, unlit, spreading, swaying wider), on
// a loop of its own (gs_puppet::layers::Particle::at with `loops`, line for
// line): c where it leaves from; ax0.y how fast it rises in the world;
// ax1.x its size, ax1.y its place in the loop, ax1.z the loop's length
// (seconds). Where it has risen to is `mote_rise`, added in the world.
// Or a mote circling where it leaves from (ax2.x 4: stunned stars, calm
// motes): always there, aglow, twinkling, once round a loop -- ax0.x the
// circle's radius, ax0.y how high it bobs.
fn mote_t(i: Inst) -> f32 {
    let life = max(i.ax1.z, 0.1);
    let t = (u.arrive[3].z + i.ax1.y) % life;
    return select(t, t + life, t < 0.0);
}

fn mote(i: Inst) -> Dot {
    var d = Dot(false, vec3<f32>(0.0), vec3<f32>(0.0), vec3<f32>(0.0), vec3<f32>(0.0), vec3<f32>(0.0), 0.0, 0.0, vec3<f32>(0.0), 0.0, false);
    let life = max(i.ax1.z, 0.1);
    let t = mote_t(i);
    if (i.ax2.x > 3.5) {
        let r = i.ax1.x;
        d.ok = true;
        d.c = i.c.xyz;
        d.a0 = vec3<f32>(r, 0.0, 0.0);
        d.a1 = vec3<f32>(0.0, r, 0.0);
        d.a2 = vec3<f32>(0.0, 0.0, r);
        d.rgb = i.color.rgb;
        d.alpha = i.color.a * (0.7 + 0.3 * sin(u.arrive[3].z * 7.0 + i.ax1.y * 37.0));
        d.glow = 1.0;
        return d;
    }
    let shadow = i.ax2.x > 2.5;
    let fade = smoothstep(0.0, 0.15 * life, t) * (1.0 - smoothstep(0.45 * life, life, t));
    let twinkle = select(0.55 + 0.45 * sin(u.arrive[3].z * 9.0 + i.ax1.y * 37.0), 1.0, shadow);
    if (fade * twinkle <= 0.02) { return d; }
    let r = i.ax1.x * (1.0 + select(0.6, 1.2, shadow) * t / life);
    d.ok = true;
    d.c = i.c.xyz;
    d.a0 = vec3<f32>(r, 0.0, 0.0);
    d.a1 = vec3<f32>(0.0, r, 0.0);
    d.a2 = vec3<f32>(0.0, 0.0, r);
    d.rgb = i.color.rgb;
    d.alpha = i.color.a * fade * twinkle;
    d.glow = select(fade, 0.0, shadow);
    d.film = shadow;
    return d;
}

fn mote_rise(i: Inst) -> vec3<f32> {
    let t = mote_t(i);
    if (i.ax2.x > 3.5) {
        let a = 6.2831853 * t / max(i.ax1.z, 0.1);
        return vec3<f32>(i.ax0.x * cos(a), i.ax0.y * sin(2.0 * a), i.ax0.x * sin(a));
    }
    let sway = select(0.02, 0.07, i.ax2.x > 2.5) * sin(t * 2.7 + i.ax1.y * 11.0);
    return vec3<f32>(sway, i.ax0.y * t, 0.6 * sway);
}

// A core's glow channel, taken apart: its rim (0..1), and its glow with the
// OWN_FACING mark.
fn core_rim(w: f32) -> f32 {
    return floor(w / RIM_STEP) / 8.0;
}
fn core_rest(w: f32) -> f32 {
    return w - floor(w / RIM_STEP) * RIM_STEP;
}

fn moved(seg: u32, p: vec4<f32>) -> vec3<f32> {
    let k = seg * 3u;
    return vec3<f32>(dot(u.bones[k], p), dot(u.bones[k + 1u], p), dot(u.bones[k + 2u], p));
}

fn posed(i: Inst, p0: vec4<f32>, point: f32) -> vec3<f32> {
    let p = vec4<f32>(p0.xyz, point);
    var out = vec3<f32>(0.0);
    for (var k = 0; k < 4; k = k + 1) {
        if (i.shares[k] > 0.0) { out = out + moved(i.bones[k], p) * i.shares[k]; }
    }
    return out;
}

// screen pixels (in the image) and depth
fn seen(world: vec3<f32>) -> vec3<f32> {
    let w = vec4<f32>(world, 1.0);
    let depth = max(dot(u.proj[2], w), 1e-4);
    let ss = max(u.floor.w, 1.0);
    return vec3<f32>((dot(u.proj[0], w) / depth - u.frame.x) * ss, (dot(u.proj[1], w) / depth - u.frame.y) * ss, depth);
}

@vertex fn vs_cloud(@builtin(vertex_index) k: u32, @builtin(instance_index) at: u32) -> VOut {
    let i = splats[order[at]];
    var o: VOut;
    // what it is now: as made; a frame of a hit's arrival, blended with the
    // next by the hit's age; or a dot the hit threw, where it is by now
    var d = as_made(i);
    var weight = 1.0;
    let tag = i.ax1.w;
    if (tag > 0.5) { weight = frame_weight(u32(tag + 0.5) - 1u); }
    if (tag < -2.5) {
        d = mote(i);
    } else if (tag < -0.5) {
        d = thrown(i, u.arrive[3].x);
    }
    if (weight <= 0.0 || !d.ok) {
        // nothing to draw: off the image
        o.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return o;
    }
    let corner = array<vec2<f32>, 6>(vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
                                     vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0));
    var world = posed(i, vec4<f32>(d.c, 0.0), 1.0);
    // (a glint rises in the world, whatever way its part is turned)
    if (tag < -2.5) { world = world + mote_rise(i); }
    let c = seen(world);
    // its three axes on the screen: the footprint's covariance
    let a0 = seen(world + posed(i, vec4<f32>(d.a0, 0.0), 0.0)).xy - c.xy;
    let a1 = seen(world + posed(i, vec4<f32>(d.a1, 0.0), 0.0)).xy - c.xy;
    let a2 = seen(world + posed(i, vec4<f32>(d.a2, 0.0), 0.0)).xy - c.xy;
    // A splat covers at least a pixel -- and one smaller than a pixel is
    // spread over it: as much fainter as it was made wider. (Left as
    // opaque as it was, every speck of a fitted cloud paints a whole pixel
    // its own colour, and a figure drawn small is a speckle of them.)
    let xx0 = a0.x * a0.x + a1.x * a1.x + a2.x * a2.x;
    let yy0 = a0.y * a0.y + a1.y * a1.y + a2.y * a2.y;
    let xy = a0.x * a0.y + a1.x * a1.y + a2.x * a2.y;
    let xx = xx0 + 0.3;
    let yy = yy0 + 0.3;
    let det = max(xx * yy - xy * xy, 1e-6);
    let spread = sqrt(clamp((xx0 * yy0 - xy * xy) / det, 0.0, 1.0));
    let r = vec2<f32>(3.0 * sqrt(xx), 3.0 * sqrt(yy));
    let off = corner[k] * r * u.to_cam.w;
    let p = c.xy + off;
    // What shows faces the viewer: a splat that took its facing from the
    // wrong side of a thin or doubled surface (a generated mesh has an
    // inner shell) is turned round, not lit from behind.
    var n = posed(i, vec4<f32>(d.n, 0.0), 0.0);
    if (dot(n, u.to_cam.xyz) < 0.0) { n = -n; }
    let glow = d.glow;
    // lit as the puppet's skin is; a splat with no facing keeps its colour
    var shade = 1.0;
    if (dot(n, n) > 1e-12) {
        let g = dot(normalize(n), u.light.xyz);
        let lit = smoothstep(LIGHT_ABOVE - 0.2, LIGHT_ABOVE + 0.2, g);
        let dark = 1.0 - smoothstep(DARK_BELOW - 0.2, DARK_BELOW + 0.2, g);
        // what glows is not shaded
        shade = mix((1.0 + 0.13 * lit - 0.2 * dark) * (1.0 + 0.08 * g), 1.0, clamp(glow, 0.0, 1.0));
    }
    let ss = max(u.floor.w, 1.0);
    o.clip = vec4<f32>(p.x / (u.size.x * ss) * 2.0 - 1.0, 1.0 - p.y / (u.size.y * ss) * 2.0, 0.5, 1.0);
    o.local = off;
    o.inv = vec4<f32>(yy / det, -xy / det, -xy / det, xx / det);
    o.z = c.z;
    o.rgb = clamp(d.rgb * shade, vec3<f32>(0.0), vec3<f32>(1.0));
    o.alpha = d.alpha * spread * weight;
    // a film is never a core: the body, and past it the backdrop, show
    // through; nor is a frame on its way out
    o.solid = select(select(d.alpha, 0.0, d.film), 0.0, weight < 0.5);
    o.pos = world;
    o.n = n;
    // (a thrown dot, and what is built on the body, keeps its own facing;
    // an invisible figure's skin is a surface seen only edge-on: fs_final)
    o.glow = glow + select(0.0, OWN_FACING, i.ax2.w > 0.5 || tag < -0.5) + RIM_STEP * round(clamp(i.fx.x, 0.0, 1.0) * 8.0);
    o.wet = clamp(d.wet, 0.0, 1.0);
    return o;
}

fn gauss_m(d: vec2<f32>, inv: vec4<f32>) -> f32 {
    return d.x * (inv.x * d.x + inv.y * d.y) + d.y * (inv.z * d.x + inv.w * d.y);
}

struct Front {
    @builtin(frag_depth) depth: f32,
    @location(0) n: vec4<f32>,
    @location(1) pos: vec4<f32>,
};

@fragment fn fs_front(v: VOut) -> Front {
    // (the faintest are haze, not surface; but a fitted cloud's surface IS
    // mostly faint splats over each other, and a pixel none of them cored
    // would be a hole)
    if (v.solid < 0.05 || gauss_m(v.local, v.inv) > CORE_M) { discard; }
    var o: Front;
    o.depth = clamp((v.z - u.frame.z) / max(u.frame.w, 1e-3) * 0.9 + 0.05, 0.0, 1.0);
    // the facing, unit, made longer by how wet it is (1 dry .. 2 wet)
    var n = vec3<f32>(0.0);
    if (dot(v.n, v.n) > 1e-12) { n = normalize(v.n) * (1.0 + v.wet); }
    o.n = vec4<f32>(n, v.glow);
    o.pos = vec4<f32>(v.pos, v.z);
    return o;
}

@fragment fn fs_blend(v: VOut) -> @location(0) vec4<f32> {
    let m = gauss_m(v.local, v.inv);
    if (m > REACH_M) { discard; }
    let front = textureLoad(t_front_pos, vec2<i32>(v.clip.xy), 0).w;
    if (v.z > front + u.light.w) { discard; }
    let a = min(v.alpha * exp(-0.5 * m), 0.99);
    return vec4<f32>(v.rgb * a, a);
}

@vertex fn vs_full(@builtin(vertex_index) k: u32) -> @builtin(position) vec4<f32> {
    let c = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(c[k], 0.0, 1.0);
}

fn s2l(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}
fn l2s(c0: vec3<f32>) -> vec3<f32> {
    let c = clamp(c0, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3<f32>(0.0031308));
}

struct Layer {
    @location(0) albedo: vec4<f32>,
    @location(1) lit: vec4<f32>,
    @location(2) geo: vec4<f32>,
    @location(3) rgba: vec4<f32>,
};

@fragment fn fs_final(@builtin(position) fc: vec4<f32>) -> Layer {
    var o: Layer;
    o.albedo = vec4<f32>(0.0);
    o.lit = vec4<f32>(0.0);
    o.geo = vec4<f32>(0.0);
    o.rgba = vec4<f32>(0.0);
    let pad = i32(u.size.z);
    let px = vec2<i32>(fc.xy) - vec2<i32>(pad, pad);
    let wh = vec2<i32>(i32(u.size.x), i32(u.size.y));
    if (px.x < 0 || px.y < 0 || px.x >= wh.x || px.y >= wh.y) { return o; }
    // the samples under this pixel (the passes before ran at ss x ss a pixel)
    let ss = max(i32(u.floor.w), 1);
    var acc = vec4<f32>(0.0);
    var n = vec3<f32>(0.0);
    var pos = vec3<f32>(0.0);
    var glow = 0.0;
    var wet = 0.0;
    var cores = 0.0;
    var own = false;
    var rims = 0.0;
    for (var sy = 0; sy < ss; sy = sy + 1) {
        for (var sx = 0; sx < ss; sx = sx + 1) {
            let q = px * ss + vec2<i32>(sx, sy);
            let s = textureLoad(t_acc, q, 0);
            let f = textureLoad(t_front_pos, q, 0);
            // each sample as its own pixel would be: opaque under a core
            // and where the splats summed to as good as opaque
            let under = f.w < FAR * 0.5;
            let sa = select(smoothstep(0.1, 0.55, s.a), 1.0, under);
            if (s.a > 1e-3) { acc = acc + vec4<f32>(s.rgb / s.a * sa, sa); }
            if (under) {
                let fn4 = textureLoad(t_front_n, q, 0);
                let l = length(fn4.xyz);
                if (l > 1e-6) {
                    n = n + fn4.xyz / l;
                    wet = wet + clamp(l - 1.0, 0.0, 1.0);
                }
                let rest = core_rest(fn4.w);
                rims = rims + core_rim(fn4.w);
                let mine = rest >= OWN_FACING * 0.5;
                own = own || mine;
                glow = glow + rest - select(0.0, OWN_FACING, mine);
                pos = pos + f.xyz;
                cores = cores + 1.0;
            }
        }
    }
    if (acc.a <= 1e-3) { return o; }
    let count = f32(ss * ss);
    let rgb = round(clamp(acc.rgb / acc.a, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0) / 255.0;
    let a = acc.a / count;
    o.albedo = vec4<f32>(rgb, a);
    // glow in alpha, as the skin's layer: its own light, kept in the dark
    o.lit = vec4<f32>(l2s(s2l(rgb) * u.floor.rgb), clamp(glow / count, 0.0, 1.0));
    o.rgba = vec4<f32>(rgb, a);
    let back = u.to_cam.xyz;
    let fwd = -back;
    let right = normalize(cross(fwd, vec3<f32>(0.0, 1.0, 0.0)));
    let down = cross(fwd, right);
    if (cores > 0.5) {
        // the skin's facing smoothed over the surface round it (ADDITION):
        // each splat's core carries one facing, so a cloud of big splats
        // lit core by core shows as a patchwork, light and dark -- the skin
        // cores within a few pixels that lie on the same sheet of the
        // surface (a limb in front of the trunk is not) are averaged,
        // nearer counting more. What is built on the body (ax2.w: a vine,
        // a rope, a bead of blood) keeps its own facing: it is its shape.
        let here = pos / cores;
        var sm = vec3<f32>(0.0);
        let c0 = px * ss;
        for (var dy = -SMOOTH_PX; dy <= SMOOTH_PX; dy = dy + 1) {
            for (var dx = -SMOOTH_PX; dx <= SMOOTH_PX; dx = dx + 1) {
                let q = c0 + vec2<i32>(dx, dy) * ss;
                if (q.x < 0 || q.y < 0 || q.x >= wh.x * ss || q.y >= wh.y * ss) { continue; }
                let f = textureLoad(t_front_pos, q, 0);
                if (f.w >= FAR * 0.5 || length(f.xyz - here) > SMOOTH_FT) { continue; }
                let fn4 = textureLoad(t_front_n, q, 0);
                // the skin only: what is built on it keeps its shape
                if (core_rest(fn4.w) >= OWN_FACING * 0.5) { continue; }
                let l = length(fn4.xyz);
                if (l > 1e-6) {
                    sm = sm + fn4.xyz / l * exp(-f32(dx * dx + dy * dy) / f32(SMOOTH_PX * SMOOTH_PX));
                }
            }
        }
        let mine = select(back, normalize(n), dot(n, n) > 1e-12);
        let nn = select(mine, normalize(sm), dot(sm, sm) > 1e-12 && !own);
        // wet: the normal stored shorter (field_gpu::wet_normal)
        let w = wet / cores;
        let k = select(1.0, WET_DRY_LEN - WET_SPAN * min(w, 1.0), w > 1.0 / 255.0);
        o.geo = vec4<f32>(vec3<f32>(dot(nn, right), dot(nn, down), dot(nn, fwd)) * k, dot(pos / cores, fwd) * u.size.w);
        // an invisible figure's skin (gs_puppet::splat::Gaussian::rim): an
        // outline round its edges -- where, a few pixels off any way, the
        // figure ends or another part of it lies well in front or behind
        // (as one splat edge-on, a trained cloud's being round blobs, still
        // covers a limb's middle, and the smoothed facing is too rough to
        // draw a clean edge by) -- and faintly as the surface turns away,
        // shimmering, brighter bands of it wandering up the figure
        if (rims > 0.0) {
            var off = 0.0;
            var looked = 0.0;
            for (var k = 0; k < 16; k = k + 1) {
                let a = f32(k) * 0.39269908;
                for (var r = 1; r <= 2; r = r + 1) {
                    let q = c0 + vec2<i32>(round(vec2<f32>(cos(a), sin(a)) * RIM_PX * f32(ss * r) * 0.5));
                    looked = looked + 1.0;
                    if (q.x < 0 || q.y < 0 || q.x >= wh.x * ss || q.y >= wh.y * ss) {
                        off = off + 1.0;
                        continue;
                    }
                    let f = textureLoad(t_front_pos, q, 0);
                    if (f.w >= FAR * 0.5 || length(f.xyz - here) > RIM_GAP_FT) { off = off + 1.0; }
                }
            }
            // (at the very edge half the samples are off; a pinhole between
            // cores, or a stray core, inside it is a few)
            let outline = smoothstep(0.16, 0.4, off / looked);
            let turned = smoothstep(RIM_FROM, RIM_TO, 1.0 - abs(dot(nn, back)));
            let share = clamp(rims / cores, 0.0, 1.0);
            let clock = u.arrive[3].z;
            let shimmer = 0.72 + 0.28 * sin(here.y * 9.0 - clock * 2.6 + 1.7 * sin(here.x * 7.0 + here.z * 5.0 + clock * 0.9));
            let fade = (1.0 - share * (1.0 - max(outline, 0.25 * turned))) * mix(1.0, shimmer, share);
            o.albedo.a = o.albedo.a * fade;
            o.rgba.a = o.rgba.a * fade;
        }
    } else {
        // no core here (a soft edge, or a pinhole between cores): the
        // depth and the facing of the nearest covered sample -- a pixel
        // with no facing is lit by nothing, and shows as a dark speck
        var best = FAR;
        var depth = 0.0;
        var near_n = vec3<f32>(0.0);
        let reach = 8 * ss;
        let c = px * ss;
        for (var dy = -reach; dy <= reach; dy = dy + ss) {
            for (var dx = -reach; dx <= reach; dx = dx + ss) {
                let q = c + vec2<i32>(dx, dy);
                if (q.x < 0 || q.y < 0 || q.x >= wh.x * ss || q.y >= wh.y * ss) { continue; }
                let f = textureLoad(t_front_pos, q, 0);
                if (f.w < FAR * 0.5) {
                    let d = f32(max(abs(dx), abs(dy)));
                    if (d < best) { best = d; depth = dot(f.xyz, fwd) * u.size.w; near_n = textureLoad(t_front_n, q, 0).xyz; }
                    // (its length carries wetness: normalised below, dry here)
                }
            }
        }
        var g = vec3<f32>(0.0);
        if (a > 0.5 && dot(near_n, near_n) > 1e-12) {
            let nn = normalize(near_n);
            g = vec3<f32>(dot(nn, right), dot(nn, down), dot(nn, fwd));
        }
        o.geo = vec4<f32>(g, depth);
    }
    return o;
}
