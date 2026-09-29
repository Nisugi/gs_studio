// The puppet's smooth skin on the GPU (skin_gpu.rs): gs_puppet's CPU skin
// (`Posed::skin_surface`, then `field_gpu_stage::skin_image`), pass by pass.
//
//   raster   the mesh at SS x the skin's size: colour (paint, wound tint,
//            marks), glow, normal, the normal sheet's deviation, segment,
//            surface point and depth              -- skin.rs `Band::tri`
//   resolve  SS x SS samples to a pixel            -- `Pixels::resolve`
//   smooth   the normals, one axis a pass          -- `Pixels::smooth`
//   final    relief, cel bands, contours, and the layer the v2 pass reads:
//            albedo, lit (dimmed to a floor, glow in alpha), geo
//                                                  -- the shade loop,
//                                                     `skin_image`
//
// Colours are 0..1 of the CPU's 0..255 byte space (sRGB, as the CPU skin
// works in it).

struct U {
    size: vec4<f32>,      // w, h, ss, pad
    depth: vec4<f32>,     // near, range, blend_tol, contour_tol
    light: vec4<f32>,     // toward the light (skin frame), outline_px
    to_cam: vec4<f32>,    // toward the viewer, boss (0/1)
    line: vec4<f32>,      // contour colour 0..1, smooth radius (px)
    bands: vec4<f32>,     // light_above, dark_below, band_soft, lit_tone
    tone: vec4<f32>,      // dark_tone, lambert, relief_gain, normal_map_strength
    more: vec4<f32>,      // depth_shade, joined_blend, boss_extra_px, m_per_ft
    boss: vec4<f32>,      // boss colour 0..1, segments
    floor: vec4<f32>,     // the lit layer's floor (linear), has_normal
    flags: vec4<f32>,     // has_marks, smooth axis x, smooth axis y, marks texels per foot
    sheet: vec4<f32>,     // paint sheet w, h, marks sheet w, h
    fx: vec4<f32>,        // has_paint, frozen, webbed, half a strand (feet)
    web: vec4<f32>,       // thread half width (px), threads, smooth colours, eyes
    web_rgb: vec4<f32>,   // web colour 0..1, strand spacing (feet)
    frost_rgb: vec4<f32>, // frost colour 0..1
    eye_rgb: vec4<f32>,   // eye colour 0..1
};

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var<storage, read> paint_layout: array<f32>;
@group(0) @binding(2) var<storage, read> marks_layout: array<f32>;
// per segment: rotation columns (3 x vec4), marked in [3].x
@group(0) @binding(3) var<storage, read> segs: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read> joined: array<u32>;
@group(0) @binding(5) var paint: texture_2d<f32>;
@group(0) @binding(6) var normals: texture_2d<f32>;
@group(0) @binding(7) var marks_bk: texture_2d<f32>;
@group(0) @binding(8) var marks_glow: texture_2d<f32>;
// an unpainted skin's eyes: centre x, y (pixels), radius, depth
@group(0) @binding(9) var<storage, read> eyes: array<vec4<f32>>;
// a webbed figure's threads, two per: (point x, y, direction x, y), (steps)
@group(0) @binding(10) var<storage, read> threads: array<vec4<f32>>;

const TAU: f32 = 6.2831853;
const ARC: u32 = 48u;
// no surface here (f16 targets: under their 65504)
const FAR: f32 = 60000.0;

fn is_joined(a: u32, b: u32) -> bool {
    let n = u32(u.boss.w);
    if (a >= n || b >= n) { return false; }
    return joined[a * n + b] != 0u;
}

// ---- UvLayout::locate over a packed layout (UvLayout::packed)
fn across(l: u32, arc_at: u32, k: u32, rel: f32, which: u32) -> f32 {
    let base = arc_at + k * (ARC + 1u);
    let rr = rel - floor(rel / TAU) * TAU;
    let f = rr / TAU * f32(ARC);
    let i = min(u32(floor(f)), ARC - 1u);
    let t = f - f32(i);
    var a: f32;
    var b: f32;
    if (which == 0u) { a = paint_layout[base + i]; b = paint_layout[base + i + 1u]; }
    else { a = marks_layout[base + i]; b = marks_layout[base + i + 1u]; }
    return a + (b - a) * t;
}

fn lay(which: u32, i: u32) -> f32 {
    if (which == 0u) { return paint_layout[i]; }
    return marks_layout[i];
}

// Where (segment, station, theta) lands on a layout's sheet, pixels; x < 0
// when the segment has no patch.
fn locate(which: u32, seg: u32, station: f32, theta: f32) -> vec2<f32> {
    let n = u32(lay(which, 0u));
    if (seg >= n) { return vec2<f32>(-1.0, -1.0); }
    let px_per_ft = lay(which, 1u);
    let h = 2u + 8u * seg;
    let ox = lay(which, h);
    let oy = lay(which, h + 1u);
    let cx = lay(which, h + 2u);
    let seam = lay(which, h + 3u);
    let rows_at = u32(lay(which, h + 4u));
    let n_rows = u32(lay(which, h + 5u));
    let arc_at = u32(lay(which, h + 6u));
    if (n_rows == 0u) { return vec2<f32>(-1.0, -1.0); }
    let last = f32(n_rows - 1u);
    let s = clamp(station, 0.0, last);
    let k = min(u32(floor(s)), n_rows - 1u);
    var k1 = k;
    var t = 0.0;
    if (k < n_rows - 1u) { k1 = k + 1u; t = s - f32(k); }
    let rel = theta - seam;
    let u0 = across(which, arc_at, k, rel, which);
    let u1 = across(which, arc_at, k1, rel, which);
    let uu = u0 + (u1 - u0) * t;
    let y0 = lay(which, rows_at + 2u * k);
    let c0 = lay(which, rows_at + 2u * k + 1u);
    let y1 = lay(which, rows_at + 2u * k1);
    let c1 = lay(which, rows_at + 2u * k1 + 1u);
    let y = y0 + (y1 - y0) * t;
    let circ = c0 + (c1 - c0) * t;
    return vec2<f32>(cx + (uu - 0.5) * circ * px_per_ft, oy + y);
}

// Texture::sample: alpha-weighted bilinear, None under half coverage.
fn sample_paint(at: vec2<f32>) -> vec4<f32> {
    let wh = vec2<f32>(u.sheet.x, u.sheet.y);
    let p = clamp(at - 0.5, vec2<f32>(0.0), wh - 1.0);
    let p0 = vec2<i32>(floor(p));
    let p1 = min(p0 + vec2<i32>(1, 1), vec2<i32>(wh) - 1);
    let t = p - floor(p);
    var rgb = vec3<f32>(0.0);
    var a = 0.0;
    let taps = array<vec3<f32>, 4>(
        vec3<f32>(f32(p0.x), f32(p0.y), (1.0 - t.x) * (1.0 - t.y)),
        vec3<f32>(f32(p1.x), f32(p0.y), t.x * (1.0 - t.y)),
        vec3<f32>(f32(p0.x), f32(p1.y), (1.0 - t.x) * t.y),
        vec3<f32>(f32(p1.x), f32(p1.y), t.x * t.y));
    for (var i = 0; i < 4; i = i + 1) {
        let q = textureLoad(paint, vec2<i32>(taps[i].xy), 0);
        let w = q.a * taps[i].z;
        rgb = rgb + q.rgb * w;
        a = a + w;
    }
    if (a <= 0.5) { return vec4<f32>(0.0); }
    return vec4<f32>(rgb / a, 1.0);
}

// Texture::sample_normal: the rest-space deviation (xyz), w = found.
fn sample_dev(at: vec2<f32>) -> vec4<f32> {
    let wh = vec2<f32>(u.sheet.x, u.sheet.y);
    let p = clamp(at - 0.5, vec2<f32>(0.0), wh - 1.0);
    let p0 = vec2<i32>(floor(p));
    let p1 = min(p0 + vec2<i32>(1, 1), vec2<i32>(wh) - 1);
    let t = p - floor(p);
    var v = vec3<f32>(0.0);
    var a = 0.0;
    let taps = array<vec3<f32>, 4>(
        vec3<f32>(f32(p0.x), f32(p0.y), (1.0 - t.x) * (1.0 - t.y)),
        vec3<f32>(f32(p1.x), f32(p0.y), t.x * (1.0 - t.y)),
        vec3<f32>(f32(p0.x), f32(p1.y), (1.0 - t.x) * t.y),
        vec3<f32>(f32(p1.x), f32(p1.y), t.x * t.y));
    for (var i = 0; i < 4; i = i + 1) {
        let q = textureLoad(normals, vec2<i32>(taps[i].xy), 0);
        let w = q.a * taps[i].z;
        // decode_normal: b / 127.5 - 1, from the 0..1 texel
        v = v + (q.rgb * 255.0 / 127.5 - 1.0) * w;
        a = a + w;
    }
    if (a <= 0.5) { return vec4<f32>(0.0); }
    return vec4<f32>(v / a, 1.0);
}

// MarksSheet::apply: colour * k + b, and the glow, bilinear.
fn apply_marks(col: vec3<f32>, feet: vec2<f32>) -> vec4<f32> {
    let per_ft = u.flags.w;
    let wh = vec2<f32>(u.sheet.z, u.sheet.w);
    let f = clamp(feet * per_ft - 0.5, vec2<f32>(0.0), wh - 1.0);
    let p0 = vec2<i32>(floor(f));
    let p1 = min(p0 + vec2<i32>(1, 1), vec2<i32>(wh) - 1);
    let t = f - floor(f);
    let bk = mix(mix(textureLoad(marks_bk, p0, 0), textureLoad(marks_bk, vec2<i32>(p1.x, p0.y), 0), t.x),
                 mix(textureLoad(marks_bk, vec2<i32>(p0.x, p1.y), 0), textureLoad(marks_bk, p1, 0), t.x), t.y);
    let g = mix(mix(textureLoad(marks_glow, p0, 0).r, textureLoad(marks_glow, vec2<i32>(p1.x, p0.y), 0).r, t.x),
                mix(textureLoad(marks_glow, vec2<i32>(p0.x, p1.y), 0).r, textureLoad(marks_glow, p1, 0).r, t.x), t.y);
    return vec4<f32>(col * bk.a + bk.rgb, g);
}

// ---- gs_puppet's hash, and the web and frost measured on the surface

fn hash(a: i32, b: u32) -> f32 {
    var x = (bitcast<u32>(a) * 0x9E3779B9u) ^ (b * 0x85EBCA6Bu);
    x = x ^ (x >> 15u);
    x = x * 0x2C1B3C6Du;
    x = x ^ (x >> 12u);
    return f32(x & 0xFFFFu) / 65535.0;
}

fn web_alpha(at: vec2<f32>, half_width: f32) -> f32 {
    var a = 0.0;
    let fam = array<vec3<f32>, 3>(vec3<f32>(0.8, 0.6, 1.0), vec3<f32>(-0.6, 0.8, 1.13), vec3<f32>(0.0, 1.0, 1.7));
    for (var k = 0; k < 3; k = k + 1) {
        let f = fam[k];
        let sp = f.z * u.web_rgb.w;
        let across = (at.x * f.x + at.y * f.y) / sp;
        let along = (-at.x * f.y + at.y * f.x) / sp;
        let cell = floor(across);
        if (hash(i32(cell), u32(k) + 11u) < 0.22) { continue; }
        let wobble = 0.18 * sin(along * 1.9 + hash(i32(cell), u32(k)) * TAU);
        let d = abs(across - cell - 0.5 - wobble) * sp;
        a = max(a, 1.0 - smoothstep(half_width * 0.5, half_width, d));
    }
    return a;
}

fn frost_glint(at: vec2<f32>) -> bool {
    let cx = i32(floor(at.x / 0.07));
    let cy = i32(floor(at.y / 0.07));
    return hash((cx * 7919) ^ cy, 3u) > 0.965;
}

// ---- raster

struct VIn {
    @location(0) p: vec2<f32>,
    @location(1) z: f32,
    @location(2) seg: u32,
    @location(3) n: vec3<f32>,
    @location(4) addressed: f32,
    @location(5) pos: vec3<f32>,
    @location(6) col: vec3<f32>,
    @location(7) wound: vec4<f32>,
    @location(8) addr: vec2<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) n: vec3<f32>,
    @location(1) pos: vec3<f32>,
    @location(2) col: vec3<f32>,
    @location(3) wound: vec4<f32>,
    @location(4) addr: vec2<f32>,
    @location(5) z: f32,
    @location(6) @interpolate(flat) seg: u32,
    @location(7) @interpolate(flat) addressed: f32,
};

@vertex fn vs_raster(v: VIn) -> VOut {
    let ss = u.size.z;
    let full = vec2<f32>(u.size.x, u.size.y) * ss;
    var o: VOut;
    let d = clamp((v.z - u.depth.x) / max(u.depth.y, 1e-3) * 0.9 + 0.05, 0.0, 1.0);
    o.clip = vec4<f32>(v.p.x / full.x * 2.0 - 1.0, 1.0 - v.p.y / full.y * 2.0, d, 1.0);
    o.n = v.n;
    o.pos = v.pos;
    o.col = v.col / 255.0;
    o.wound = vec4<f32>(v.wound.rgb / 255.0, v.wound.a);
    o.addr = v.addr;
    o.z = v.z;
    o.seg = v.seg;
    o.addressed = v.addressed;
    return o;
}

struct RasterOut {
    @location(0) col: vec4<f32>,   // colour, glow
    @location(1) n: vec4<f32>,     // normal
    @location(2) dev: vec4<f32>,   // deviation, segment
    @location(3) pos: vec4<f32>,   // surface point, depth
};

@fragment fn fs_raster(i: VOut) -> RasterOut {
    var col = i.col;
    var dev = vec3<f32>(0.0);
    var glow = 0.0;
    if (i.addressed > 0.5 && u.fx.x > 0.5) {
        let at = locate(0u, i.seg, i.addr.x, i.addr.y);
        if (at.x >= 0.0) {
            let painted = sample_paint(at);
            if (painted.a > 0.0) {
                col = painted.rgb + (i.wound.rgb - painted.rgb) * i.wound.a;
            }
            if (u.floor.w > 0.5) {
                let d = sample_dev(at);
                if (d.w > 0.0) {
                    let base = i.seg * 4u;
                    dev = segs[base].xyz * d.x + segs[base + 1u].xyz * d.y + segs[base + 2u].xyz * d.z;
                }
            }
        }
    }
    // the surface layout, where marks and effects are measured
    var feet = vec2<f32>(-1.0, -1.0);
    if (i.addressed > 0.5 && (u.flags.x > 0.5 || u.fx.y > 0.5 || u.fx.z > 0.5)) {
        feet = locate(1u, i.seg, i.addr.x, i.addr.y);
    }
    if (feet.x >= 0.0 && u.flags.x > 0.5 && segs[i.seg * 4u + 3u].x > 0.5) {
        let m = apply_marks(col, feet);
        col = m.rgb;
        glow = m.a;
    }
    if (u.fx.y > 0.5) {
        col = col + (u.frost_rgb.rgb - col) * 0.62;
        if (feet.x >= 0.0 && frost_glint(feet)) {
            col = col + (vec3<f32>(1.0) - col) * 0.85;
        }
    }
    if (u.fx.z > 0.5 && feet.x >= 0.0) {
        let a = web_alpha(feet, u.fx.w) * 0.92;
        col = col + (u.web_rgb.rgb - col) * a;
    }
    var o: RasterOut;
    o.col = vec4<f32>(col, glow);
    o.n = vec4<f32>(i.n, 0.0);
    o.dev = vec4<f32>(dev, f32(i.seg));
    o.pos = vec4<f32>(i.pos, i.z);
    return o;
}

// ---- full-screen passes

@vertex fn vs_full(@builtin(vertex_index) k: u32) -> @builtin(position) vec4<f32> {
    let c = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(c[k], 0.0, 1.0);
}

@group(1) @binding(0) var s_col: texture_2d<f32>;
@group(1) @binding(1) var s_n: texture_2d<f32>;
@group(1) @binding(2) var s_dev: texture_2d<f32>;
@group(1) @binding(3) var s_pos: texture_2d<f32>;
// per thread (x = its index): first and last step on the body, drawn
@group(1) @binding(4) var s_threads: texture_2d<f32>;

struct Resolved {
    @location(0) col: vec4<f32>,   // colour avg, glow avg
    @location(1) n: vec4<f32>,     // normal avg, coverage
    @location(2) dev: vec4<f32>,   // deviation avg, nearest segment (-1: none)
    @location(3) pos: vec4<f32>,   // nearest surface point, depth (FAR: none)
};

@fragment fn fs_resolve(@builtin(position) fc: vec4<f32>) -> Resolved {
    let ss = i32(u.size.z);
    let px = vec2<i32>(fc.xy);
    var hits = 0;
    var n = vec3<f32>(0.0);
    var col = vec3<f32>(0.0);
    var dev = vec3<f32>(0.0);
    var glow = 0.0;
    var best = FAR;
    var seg = -1.0;
    var pos = vec3<f32>(0.0);
    for (var sy = 0; sy < ss; sy = sy + 1) {
        for (var sx = 0; sx < ss; sx = sx + 1) {
            let q = px * ss + vec2<i32>(sx, sy);
            let p = textureLoad(s_pos, q, 0);
            if (p.w >= FAR * 0.5) { continue; }
            hits = hits + 1;
            let nn = textureLoad(s_n, q, 0).xyz;
            n = n + select(vec3<f32>(0.0, 0.0, 1.0), normalize(nn), dot(nn, nn) > 1e-12);
            let d = textureLoad(s_dev, q, 0);
            dev = dev + d.xyz;
            let c = textureLoad(s_col, q, 0);
            col = col + c.rgb;
            glow = glow + c.a;
            if (p.w < best) { best = p.w; seg = d.w; pos = p.xyz; }
        }
    }
    var o: Resolved;
    if (hits == 0) {
        o.col = vec4<f32>(0.0);
        o.n = vec4<f32>(0.0);
        o.dev = vec4<f32>(0.0, 0.0, 0.0, -1.0);
        o.pos = vec4<f32>(0.0, 0.0, 0.0, FAR);
        return o;
    }
    let k = 1.0 / f32(hits);
    o.col = vec4<f32>(col * k, glow * k);
    o.n = vec4<f32>(select(vec3<f32>(0.0, 0.0, 1.0), normalize(n), dot(n, n) > 1e-12), f32(hits) / f32(ss * ss));
    o.dev = vec4<f32>(dev * k, seg);
    o.pos = vec4<f32>(pos, best);
    return o;
}

// Pixels::smooth, one axis (u.flags.yz): normals (and an unpainted skin's
// colours) averaged over the radius, between pixels near in depth (or
// joined and a little further).
struct Smoothed {
    @location(0) n: vec4<f32>,
    @location(1) col: vec4<f32>,
};

@fragment fn fs_smooth(@builtin(position) fc: vec4<f32>) -> Smoothed {
    let px = vec2<i32>(fc.xy);
    let here = textureLoad(s_n, px, 0);
    let here_col = textureLoad(s_col, px, 0);
    let p0 = textureLoad(s_pos, px, 0);
    var out: Smoothed;
    out.n = here;
    out.col = here_col;
    if (p0.w >= FAR * 0.5) { return out; }
    let s0 = textureLoad(s_dev, px, 0).w;
    let r = i32(u.line.w);
    let dir = vec2<i32>(i32(u.flags.y), i32(u.flags.z));
    let wh = vec2<i32>(i32(u.size.x), i32(u.size.y));
    var n = vec3<f32>(0.0);
    var col = vec3<f32>(0.0);
    var k = 0.0;
    for (var t = -r; t <= r; t = t + 1) {
        let q = px + dir * t;
        if (q.x < 0 || q.y < 0 || q.x >= wh.x || q.y >= wh.y) { continue; }
        let pq = textureLoad(s_pos, q, 0);
        let dz = abs(pq.w - p0.w);
        let sq = textureLoad(s_dev, q, 0).w;
        let tol = u.depth.z;
        if (dz > tol && !(dz <= tol * u.more.y && s0 >= 0.0 && sq >= 0.0 && is_joined(u32(s0), u32(sq)))) { continue; }
        n = n + textureLoad(s_n, q, 0).xyz;
        col = col + textureLoad(s_col, q, 0).rgb;
        k = k + 1.0;
    }
    out.n = vec4<f32>(select(here.xyz, normalize(n), dot(n, n) > 1e-12), here.w);
    if (u.web.z > 0.5) {
        out.col = vec4<f32>(col / max(k, 1.0), here_col.a);
    }
    return out;
}

// bind_threads' walk: where thread k (this fragment's x) first and last
// meets the body, in half-pixel steps, and whether it binds (a short
// bridge over a gap, not a tripwire of air)
@fragment fn fs_threads(@builtin(position) fc: vec4<f32>) -> @location(0) vec4<f32> {
    let k = u32(fc.x);
    let a = threads[2u * k];
    let steps = i32(threads[2u * k + 1u].x);
    let wh = vec2<f32>(u.size.x, u.size.y);
    var first = 1e9;
    var last = -1e9;
    var covered = 0.0;
    for (var s = -steps; s <= steps; s = s + 1) {
        let t = f32(s) * 0.5;
        let p = a.xy + a.zw * t;
        if (p.x < 0.0 || p.y < 0.0 || p.x >= wh.x || p.y >= wh.y) { continue; }
        if (textureLoad(s_n, vec2<i32>(p), 0).w >= 0.5) {
            first = min(first, t);
            last = max(last, t);
            covered = covered + 1.0;
        }
    }
    let ok = last >= first && last - first >= 4.0 && covered * 0.5 >= (last - first) * 0.6;
    return vec4<f32>(first, last, select(0.0, 1.0, ok), 0.0);
}

fn over(top: vec4<f32>, under: vec4<f32>) -> vec4<f32> {
    let a = top.a + under.a * (1.0 - top.a);
    if (a <= 1e-6) { return vec4<f32>(0.0); }
    return vec4<f32>((top.rgb * top.a + under.rgb * under.a * (1.0 - top.a)) / a, a);
}

// ---- final: shade, contours, and the v2 pass's layer

fn smoothstep1(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn s2l(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}
fn l2s(c: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - 0.055, x * 12.92, x <= vec3<f32>(0.0031308));
}

struct Layer {
    @location(0) albedo: vec4<f32>,
    @location(1) lit: vec4<f32>,
    @location(2) geo: vec4<f32>,
};

@fragment fn fs_final(@builtin(position) fc: vec4<f32>) -> Layer {
    var o: Layer;
    o.albedo = vec4<f32>(0.0);
    o.lit = vec4<f32>(0.0);
    o.geo = vec4<f32>(0.0);
    let pad = i32(u.size.w);
    let px = vec2<i32>(fc.xy) - vec2<i32>(pad, pad);
    let wh = vec2<i32>(i32(u.size.x), i32(u.size.y));
    if (px.x < 0 || px.y < 0 || px.x >= wh.x || px.y >= wh.y) { return o; }
    let nc = textureLoad(s_n, px, 0);           // smoothed normal, coverage
    let cov = nc.w;
    let colg = textureLoad(s_col, px, 0);
    let devs = textureLoad(s_dev, px, 0);
    let p = textureLoad(s_pos, px, 0);
    let width = u.light.w;
    let boss = u.to_cam.w > 0.5;
    // outside distance: to the nearest pixel the figure mostly covers
    let reach = i32(ceil(width + select(0.0, u.more.z, boss))) + 2;
    var outside = FAR;
    if (cov >= 0.5) {
        outside = 0.0;
    } else {
        for (var dy = -reach; dy <= reach; dy = dy + 1) {
            for (var dx = -reach; dx <= reach; dx = dx + 1) {
                let q = px + vec2<i32>(dx, dy);
                if (q.x < 0 || q.y < 0 || q.x >= wh.x || q.y >= wh.y) { continue; }
                if (textureLoad(s_n, q, 0).w >= 0.5) {
                    outside = min(outside, length(vec2<f32>(f32(dx), f32(dy))));
                }
            }
        }
    }
    // under the figure: a boss's gold, then the contour
    var under_c = vec3<f32>(0.0);
    var under_a = 0.0;
    if (boss) {
        let a = clamp(width + u.more.z - outside + 1.0, 0.0, 1.0);
        under_c = u.boss.rgb;
        under_a = a;
    }
    if (width > 0.0) {
        let a = clamp(width - outside + 1.0, 0.0, 1.0);
        let na = a + under_a * (1.0 - a);
        if (na > 1e-6) { under_c = (u.line.rgb * a + under_c * under_a * (1.0 - a)) / na; }
        under_a = na;
    }
    var top_c = vec3<f32>(0.0);
    var top_a = 0.0;
    var albedo = vec3<f32>(0.0);
    var nrm = vec3<f32>(0.0);
    if (cov > 0.0) {
        // inner contour: another segment nearer by more than the step
        var inner = 0.0;
        if (width > 0.0) {
            let r = i32(ceil(width + 1.0));
            let s0 = devs.w;
            let tol = u.depth.w;
            for (var dy = -r; dy <= r; dy = dy + 1) {
                for (var dx = -r; dx <= r; dx = dx + 1) {
                    let q = px + vec2<i32>(dx, dy);
                    if (q.x < 0 || q.y < 0 || q.x >= wh.x || q.y >= wh.y) { continue; }
                    let sq = textureLoad(s_dev, q, 0).w;
                    let zq = textureLoad(s_pos, q, 0).w;
                    if (sq < 0.0 || sq == s0 || zq >= p.w - tol || is_joined(u32(s0), u32(sq))) { continue; }
                    let step = smoothstep1(tol, 2.0 * tol, p.w - zq);
                    let near = clamp(width - length(vec2<f32>(f32(dx), f32(dy))) + 1.0, 0.0, 1.0);
                    inner = max(inner, step * near);
                }
            }
        }
        // the normal sheet's relief on top of the smoothed geometry
        let geo_n = nc.xyz;
        nrm = geo_n;
        var dg = 0.0;
        if (u.floor.w > 0.5 && dot(devs.xyz, devs.xyz) > 0.0) {
            nrm = normalize(geo_n + devs.xyz * u.tone.w);
            dg = dot(nrm, u.light.xyz) - dot(geo_n, u.light.xyz);
        }
        let gb = dot(geo_n, u.light.xyz);
        let lit = smoothstep1(u.bands.x - u.bands.z, u.bands.x + u.bands.z, gb);
        let dark = 1.0 - smoothstep1(u.bands.y - u.bands.z, u.bands.y + u.bands.z, gb);
        let tone = (1.0 + (u.bands.w - 1.0) * lit - (1.0 - u.tone.x) * dark) * (1.0 + u.tone.y * gb) * (1.0 + u.tone.z * dg);
        let dim = 1.0 - u.more.x * clamp((p.w - u.depth.x) / u.depth.y, 0.0, 1.0);
        var c = clamp(colg.rgb * tone * dim, vec3<f32>(0.0), vec3<f32>(1.0));
        if (colg.a > 0.0) {
            // a glow is its own light: the bands do not darken it
            c = c + (max(colg.rgb, c) - c) * colg.a;
        }
        c = c + (u.line.rgb - c) * inner;
        albedo = colg.rgb + (u.line.rgb - colg.rgb) * inner;
        top_c = c;
        top_a = cov;
    }
    // over(top, under)
    var ca = over(vec4<f32>(top_c, top_a), vec4<f32>(under_c, under_a));
    let q = vec2<f32>(f32(px.x) + 0.5, f32(px.y) + 0.5);
    // a webbed figure's threads, binding limb to body
    for (var k = 0; k < i32(u.web.y); k = k + 1) {
        let span = textureLoad(s_threads, vec2<i32>(k, 0), 0);
        if (span.z < 0.5) { continue; }
        let t = threads[2u * u32(k)];
        let a0 = t.xy + t.zw * span.x;
        let b0 = t.xy + t.zw * span.y;
        let ab = b0 - a0;
        let s = clamp(dot(q - a0, ab) / max(dot(ab, ab), 1e-6), 0.0, 1.0);
        let d = length(q - (a0 + ab * s));
        let wa = clamp(u.web.x + 0.5 - d, 0.0, 1.0) * 0.8;
        if (wa > 0.0) { ca = over(vec4<f32>(u.web_rgb.rgb, wa), ca); }
    }
    // an unpainted skin's eyes, where the surface they sit on shows
    for (var k = 0; k < i32(u.web.w); k = k + 1) {
        let e = eyes[u32(k)];
        let at = vec2<i32>(floor(e.xy));
        if (at.x < 0 || at.y < 0 || at.x >= wh.x || at.y >= wh.y) { continue; }
        if (textureLoad(s_pos, at, 0).w < e.w - u.depth.w) { continue; }
        let sat = textureLoad(s_dev, at, 0).w;
        if (sat >= 0.0 && segs[u32(sat) * 4u + 3u].y > 0.5) { continue; }
        let ea = clamp(e.z - length(q - e.xy) + 0.5, 0.0, 1.0);
        if (ea > 0.0) { ca = over(vec4<f32>(u.eye_rgb.rgb, ea), ca); }
    }
    let a = ca.a;
    let c = ca.rgb;
    if (a <= 0.0) { return o; }
    // the layer (field_gpu_stage::skin_image): toward the camera's frame
    let back = u.to_cam.xyz;
    let fwd = -back;
    let right = normalize(cross(fwd, vec3<f32>(0.0, 1.0, 0.0)));
    let down = cross(fwd, right);
    o.albedo = vec4<f32>(albedo, a);
    // the doll's lit base: its colours dimmed to the sky's floor
    o.lit = vec4<f32>(l2s(s2l(c) * u.floor.rgb), colg.a);
    if (cov > 0.0) {
        o.geo = vec4<f32>(dot(nrm, right), dot(nrm, down), dot(nrm, fwd), dot(p.xyz, fwd) * u.more.w);
    } else {
        // an outline pixel: the depth of the nearest surface pixel
        var best = FAR;
        var depth = 0.0;
        for (var dy = -8; dy <= 8; dy = dy + 1) {
            for (var dx = -8; dx <= 8; dx = dx + 1) {
                let q = px + vec2<i32>(dx, dy);
                if (q.x < 0 || q.y < 0 || q.x >= wh.x || q.y >= wh.y) { continue; }
                if (textureLoad(s_n, q, 0).w > 0.0) {
                    let d = f32(max(abs(dx), abs(dy)));
                    if (d < best) { best = d; depth = dot(textureLoad(s_pos, q, 0).xyz, fwd) * u.more.w; }
                }
            }
        }
        o.geo = vec4<f32>(0.0, 0.0, 0.0, depth);
    }
    return o;
}
