// A puppet as a Gaussian splat cloud on the GPU (skin_gpu.rs `draw_splats`):
// gs_puppet::splat::render, pass by pass, from `splat::prepare`'s splats
// (projected, cel-shaded, sorted back to front on the CPU).
//
//   front   each splat's core: the front-most core per pixel -- its depth,
//           normal and point (the surface; a splat far behind it is inside)
//   blend   each splat's reach, back to front, premultiplied "over"
//   final   un-premultiplied; under a core, opaque; the layer the v2 pass
//           reads (albedo, lit, geo) and the plain colours

struct U {
    size: vec4<f32>,      // w, h, pad, m_per_ft
    depth: vec4<f32>,     // near, range, surface band, reach (x the splat's radius)
    to_cam: vec4<f32>,    // toward the viewer (turned world)
    floor: vec4<f32>,     // the doll's floor (linear)
};

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var t_front_n: texture_2d<f32>;
@group(0) @binding(2) var t_front_pos: texture_2d<f32>;
@group(0) @binding(3) var t_acc: texture_2d<f32>;

const FAR: f32 = 60000.0;
// gs_puppet::splat::CORE_M and REACH_M: a splat's core, and its reach
const CORE_M: f32 = 2.25;
const REACH_M: f32 = 9.0;

struct Inst {
    @location(0) c: vec2<f32>,
    @location(1) inv: vec4<f32>,
    @location(2) r: vec2<f32>,
    @location(3) z: f32,
    @location(4) rgb: vec3<f32>,
    @location(5) alpha: f32,
    @location(6) pos: vec3<f32>,
    @location(7) n: vec3<f32>,
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
};

@vertex fn vs_splat(@builtin(vertex_index) k: u32, i: Inst) -> VOut {
    let corner = array<vec2<f32>, 6>(vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
                                     vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0));
    let off = corner[k] * i.r * u.depth.w;
    let p = i.c + off;
    var o: VOut;
    o.clip = vec4<f32>(p.x / u.size.x * 2.0 - 1.0, 1.0 - p.y / u.size.y * 2.0, 0.5, 1.0);
    o.local = off;
    o.inv = i.inv;
    o.z = i.z;
    o.rgb = i.rgb;
    o.alpha = i.alpha;
    o.pos = i.pos;
    o.n = i.n;
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
    if (gauss_m(v.local, v.inv) > CORE_M) { discard; }
    var o: Front;
    o.depth = clamp((v.z - u.depth.x) / max(u.depth.y, 1e-3) * 0.9 + 0.05, 0.0, 1.0);
    o.n = vec4<f32>(v.n, 1.0);
    o.pos = vec4<f32>(v.pos, v.z);
    return o;
}

@fragment fn fs_blend(v: VOut) -> @location(0) vec4<f32> {
    let m = gauss_m(v.local, v.inv);
    if (m > REACH_M) { discard; }
    let front = textureLoad(t_front_pos, vec2<i32>(v.clip.xy), 0).w;
    if (v.z > front + u.depth.z) { discard; }
    let a = min(v.alpha * exp(-0.5 * m), 0.99);
    return vec4<f32>(v.rgb / 255.0 * a, a);
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
    let acc = textureLoad(t_acc, px, 0);
    if (acc.a <= 1e-3) { return o; }
    let front = textureLoad(t_front_pos, px, 0);
    let covered = front.w < FAR * 0.5;
    // bytes, as the CPU rounds them, then back
    let rgb = round(clamp(acc.rgb / acc.a, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0) / 255.0;
    let a = select(acc.a, 1.0, covered);
    o.albedo = vec4<f32>(rgb, a);
    o.lit = vec4<f32>(l2s(s2l(rgb) * u.floor.rgb), 0.0);
    o.rgba = vec4<f32>(rgb, a);
    let back = u.to_cam.xyz;
    let fwd = -back;
    let right = normalize(cross(fwd, vec3<f32>(0.0, 1.0, 0.0)));
    let down = cross(fwd, right);
    if (covered) {
        let n = textureLoad(t_front_n, px, 0).xyz;
        let nn = select(back, normalize(n), dot(n, n) > 1e-12);
        o.geo = vec4<f32>(dot(nn, right), dot(nn, down), dot(nn, fwd), dot(front.xyz, fwd) * u.size.w);
    } else {
        // a soft edge: the depth of the nearest covered pixel
        var best = FAR;
        var depth = 0.0;
        for (var dy = -8; dy <= 8; dy = dy + 1) {
            for (var dx = -8; dx <= 8; dx = dx + 1) {
                let q = px + vec2<i32>(dx, dy);
                if (q.x < 0 || q.y < 0 || q.x >= wh.x || q.y >= wh.y) { continue; }
                let f = textureLoad(t_front_pos, q, 0);
                if (f.w < FAR * 0.5) {
                    let d = f32(max(abs(dx), abs(dy)));
                    if (d < best) { best = d; depth = dot(f.xyz, fwd) * u.size.w; }
                }
            }
        }
        o.geo = vec4<f32>(0.0, 0.0, 0.0, depth);
    }
    return o;
}
