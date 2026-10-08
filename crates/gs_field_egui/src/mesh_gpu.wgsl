// A puppet's modelled body on the GPU (skin_gpu.rs `draw_body`): a mesh
// uploaded once (gs_puppet::mesh::BodyMesh), posed here from the puppet's
// segment transforms -- nothing per vertex crosses from the CPU per frame
// (gs_puppet::mesh::BodyFrame).
//
//   raster  each vertex moved by its segments' shares, projected; the
//           surface's colour (its sheet, shaded as the CPU's `mesh::render`),
//           normal (its normal sheet over the surface's own) and point,
//           SS x SS samples a pixel
//   final   the samples under each pixel averaged: the layer the v2 pass
//           reads (albedo, lit, geo) and the plain colours

struct U {
    size: vec4<f32>,      // w, h, pad, m_per_ft
    frame: vec4<f32>,     // the image's origin (screen px), nearest depth, depth range
    to_cam: vec4<f32>,    // toward the viewer (turned world); w: samples a pixel side
    light: vec4<f32>,     // toward the light (turned world); w: 1 with a normal sheet
    floor: vec4<f32>,     // the doll's floor (linear)
    proj: array<vec4<f32>, 3>,     // turned world -> (x * depth, y * depth, depth)
    bones: array<vec4<f32>, 192>,  // per segment, rest -> turned world: 3 rows
};

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var t_paint: texture_2d<f32>;
@group(0) @binding(2) var t_normal: texture_2d<f32>;
@group(0) @binding(3) var s_sheet: sampler;
@group(0) @binding(4) var t_color: texture_2d<f32>;
@group(0) @binding(5) var t_n: texture_2d<f32>;
@group(0) @binding(6) var t_pos: texture_2d<f32>;

const FAR: f32 = 60000.0;
// gs_puppet::draw::LIGHT_ABOVE / DARK_BELOW, as `mesh::render` shades
const LIGHT_ABOVE: f32 = 0.62;
const DARK_BELOW: f32 = -0.08;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) nrm: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) bones: vec4<u32>,
    @location(4) shares: vec4<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) n: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) depth: f32,
};

fn moved(seg: u32, p: vec4<f32>) -> vec3<f32> {
    let k = seg * 3u;
    return vec3<f32>(dot(u.bones[k], p), dot(u.bones[k + 1u], p), dot(u.bones[k + 2u], p));
}

@vertex fn vs_body(v: VIn) -> VOut {
    let p = vec4<f32>(v.pos, 1.0);
    let d = vec4<f32>(v.nrm, 0.0);
    var world = vec3<f32>(0.0);
    var n = vec3<f32>(0.0);
    for (var k = 0; k < 4; k = k + 1) {
        let share = v.shares[k];
        if (share > 0.0) {
            world = world + moved(v.bones[k], p) * share;
            n = n + moved(v.bones[k], d) * share;
        }
    }
    let w = vec4<f32>(world, 1.0);
    let h = vec3<f32>(dot(u.proj[0], w), dot(u.proj[1], w), dot(u.proj[2], w));
    let depth = max(h.z, 1e-4);
    let screen = h.xy / depth;
    let ndc = vec2<f32>((screen.x - u.frame.x) / u.size.x * 2.0 - 1.0, 1.0 - (screen.y - u.frame.y) / u.size.y * 2.0);
    let zn = clamp((depth - u.frame.z) / max(u.frame.w, 1e-3) * 0.9 + 0.05, 0.0, 1.0);
    var o: VOut;
    o.clip = vec4<f32>(ndc * depth, zn * depth, depth);
    o.world = world;
    o.n = n;
    o.uv = v.uv;
    o.depth = depth;
    return o;
}

struct Raster {
    @location(0) color: vec4<f32>,
    @location(1) n: vec4<f32>,
    @location(2) pos: vec4<f32>,
};

@fragment fn fs_body(v: VOut) -> Raster {
    let rgb = textureSample(t_paint, s_sheet, v.uv).rgb;
    let mapped = textureSample(t_normal, s_sheet, v.uv).xyz * 2.0 - 1.0;
    // the surface's own facing, toward the viewer on either side
    var n = normalize(v.n);
    if (dot(n, u.to_cam.xyz) < 0.0) { n = -n; }
    // a tangent frame from how the sheet runs over the surface here
    let dp1 = dpdx(v.world);
    let dp2 = dpdy(v.world);
    let duv1 = dpdx(v.uv);
    let duv2 = dpdy(v.uv);
    let dp2perp = cross(dp2, n);
    let dp1perp = cross(n, dp1);
    let t = dp2perp * duv1.x + dp1perp * duv2.x;
    let b = dp2perp * duv1.y + dp1perp * duv2.y;
    let longest = max(dot(t, t), dot(b, b));
    var nn = n;
    if (u.light.w > 0.5 && longest > 1e-20) {
        let s = inverseSqrt(longest);
        // the sheet's v runs down the image; its green is up
        nn = normalize(t * s * mapped.x - b * s * mapped.y + n * mapped.z);
    }
    let g = dot(nn, u.light.xyz);
    let lit = smoothstep(LIGHT_ABOVE - 0.2, LIGHT_ABOVE + 0.2, g);
    let dark = 1.0 - smoothstep(DARK_BELOW - 0.2, DARK_BELOW + 0.2, g);
    let shade = (1.0 + 0.13 * lit - 0.2 * dark) * (1.0 + 0.08 * g);
    var o: Raster;
    o.color = vec4<f32>(clamp(rgb * shade, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
    o.n = vec4<f32>(nn, 0.0);
    o.pos = vec4<f32>(v.world, v.depth);
    return o;
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
    let ss = i32(u.to_cam.w);
    var rgb = vec3<f32>(0.0);
    var n = vec3<f32>(0.0);
    var pos = vec3<f32>(0.0);
    var hit = 0.0;
    for (var dy = 0; dy < ss; dy = dy + 1) {
        for (var dx = 0; dx < ss; dx = dx + 1) {
            let q = px * ss + vec2<i32>(dx, dy);
            let p = textureLoad(t_pos, q, 0);
            if (p.w < FAR * 0.5) {
                rgb = rgb + textureLoad(t_color, q, 0).rgb;
                n = n + textureLoad(t_n, q, 0).xyz;
                pos = pos + p.xyz;
                hit = hit + 1.0;
            }
        }
    }
    if (hit < 0.5) { return o; }
    rgb = round(rgb / hit * 255.0) / 255.0;
    let a = hit / f32(ss * ss);
    o.albedo = vec4<f32>(rgb, a);
    o.lit = vec4<f32>(l2s(s2l(rgb) * u.floor.rgb), 0.0);
    o.rgba = vec4<f32>(rgb, a);
    let back = u.to_cam.xyz;
    let fwd = -back;
    let right = normalize(cross(fwd, vec3<f32>(0.0, 1.0, 0.0)));
    let down = cross(fwd, right);
    let nn = select(back, normalize(n), dot(n, n) > 1e-12);
    o.geo = vec4<f32>(dot(nn, right), dot(nn, down), dot(nn, fwd), dot(pos / hit, fwd) * u.size.w);
    return o;
}
