// The creature field's v2 lighting pass (field_gpu.rs): local lights -- a
// torch, a mote of light -- added in linear light on top of each layer's v1
// night result, with shadows every creature casts on the backdrop, on itself
// and on the others, the lights' own flame / glow, and the tone map.
//
// A port of Niffy's "Swamp Torchlight" prototype shader
// (lights-and-weather.md), generalised from its two fixed creatures to
// MAX_LAYERS layers in an atlas and MAX_LIGHTS lights. Weather is not here
// yet. Every constant not in a uniform is the prototype's.
//
// Camera space is the prototype's: x right, y DOWN, z forward, metres; a
// pixel p of the scene looks along ((p.x - cx) / f, (p.y - cy) / f, 1).
//
// One frame (group 0) is drawn in one or more pieces (group 1, one per
// egui paint callback): everything at once (the harness), or the backdrop
// and then each creature layer in its place in the stage's own far-to-near
// order, so the egui shapes between them (occluders, cards' overlays) keep
// theirs. Shadows see every layer whichever piece is drawn, and each piece
// draws the lights' own flame and glow where its surface is behind them.

struct Layer {
    rect: vec4<f32>,   // scene px: x, y, w, h
    uv: vec4<f32>,     // its slot in the atlas: x, y, w, h (0..1)
    z: vec4<f32>,      // x = the depth of its feet, metres; y = colour multiplier, linear
};

struct Light {
    pos: vec4<f32>,    // xyz = camera space, metres; w = kind (0 torch, 1 mote)
    col: vec4<f32>,    // rgb = this frame's colour, linear; w = power (flicker in)
    fall: vec4<f32>,   // r0, range, light radius (shadow softness), bounce
    surf: vec4<f32>,   // wrap on creatures, wrap on the ground, sheen on creatures, sheen on the ground (x wetness)
    shine: vec4<f32>,  // shininess on creatures, on the ground, brightness factor (flicker / pulse), prop shown
    prop: vec4<f32>,   // torch: flame base without sway (xyz), stick length; mote: core colour (rgb)
    beam: vec4<f32>,   // the beam's axis across the floor (xyz, camera space), half its width in degrees (>= 180: all round)
};

struct Field {
    size: vec4<f32>,   // scene w, h, target is sRGB (1 = write linear), view
    cam: vec4<f32>,    // f, cx, cy, camera height (m)
    up: vec4<f32>,     // world up in camera space (xyz), time (s)
    misc: vec4<f32>,   // water, shadows on, layer count, light count
    bgmap: vec4<f32>,  // where the backdrop image lies, scene px: x, y, w, h
    layers: array<Layer, 32>,
    lights: array<Light, 8>,
};

// One piece of the frame.
struct Draw {
    ndc: vec4<f32>,    // its quad, normalised device coords: x0, y0, x1, y1
    map: vec4<f32>,    // scene px = framebuffer px * map.xy + map.zw
    mode: vec4<f32>,   // x: 0 everything, 1 the backdrop, 2 one layer (y = its index)
};

@group(0) @binding(0) var<uniform> U: Field;
@group(0) @binding(1) var s_lin: sampler;
@group(0) @binding(2) var s_near: sampler;
@group(0) @binding(3) var bg_albedo: texture_2d<f32>;
@group(0) @binding(4) var bg_lit: texture_2d<f32>;
@group(0) @binding(5) var bg_geo: texture_2d<f32>;
@group(0) @binding(6) var at_albedo: texture_2d<f32>;
@group(0) @binding(7) var at_lit: texture_2d<f32>;
@group(0) @binding(8) var at_geo: texture_2d<f32>;
@group(1) @binding(0) var<uniform> D: Draw;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
};

// The piece's quad (two triangles). Its pixels find their scene position
// from where they land in the framebuffer (D.map), so a quad that runs off
// the screen still maps every pixel it does cover correctly.
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let corner = array<vec2<f32>, 6>(vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
                                     vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0));
    let c = corner[i];
    var o: VsOut;
    o.pos = vec4<f32>(mix(D.ndc.x, D.ndc.z, c.x), mix(D.ndc.y, D.ndc.w, c.y), 0.0, 1.0);
    return o;
}

const LUM = vec3<f32>(0.2126, 0.7152, 0.0722);

fn s2l(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}
fn l2s(c0: vec3<f32>) -> vec3<f32> {
    let c = max(c0, vec3<f32>(0.0));
    return select(1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3<f32>(0.0031308));
}
fn cbrt(x: f32) -> f32 { return sign(x) * pow(abs(x), 1.0 / 3.0); }
fn lin2lab(c: vec3<f32>) -> vec3<f32> {
    let l = cbrt(0.4122214708 * c.r + 0.5363325363 * c.g + 0.0514459929 * c.b);
    let m = cbrt(0.2119034982 * c.r + 0.6806995451 * c.g + 0.1073969566 * c.b);
    let s = cbrt(0.0883024619 * c.r + 0.2817188376 * c.g + 0.6299787005 * c.b);
    return vec3<f32>(0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
                     1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
                     0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s);
}
fn lab2lin(c: vec3<f32>) -> vec3<f32> {
    var l = c.x + 0.3963377774 * c.y + 0.2158037573 * c.z;
    var m = c.x - 0.1055613458 * c.y - 0.0638541728 * c.z;
    var s = c.x - 0.0894841775 * c.y - 1.2914855480 * c.z;
    l = l * l * l; m = m * m * m; s = s * s * s;
    return vec3<f32>( 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
                     -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
                     -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s);
}
fn in_gamut(l: vec3<f32>) -> bool {
    return all(l >= vec3<f32>(-1e-4)) && all(l <= vec3<f32>(1.0001));
}
// Chroma only: the most saturated colour of this lightness and hue in gamut.
fn gamut_clip(lab0: vec3<f32>) -> vec3<f32> {
    let lin = lab2lin(lab0);
    if (in_gamut(lin)) { return lin; }
    let L = clamp(lab0.x, 0.0, 1.0);
    var lo = 0.0;
    var hi = 1.0;
    for (var i = 0; i < 12; i = i + 1) {
        let m = 0.5 * (lo + hi);
        if (in_gamut(lab2lin(vec3<f32>(L, lab0.yz * m)))) { lo = m; } else { hi = m; }
    }
    return clamp(lab2lin(vec3<f32>(L, lab0.yz * lo)), vec3<f32>(0.0), vec3<f32>(1.0));
}
fn tonemap(lin0: vec3<f32>) -> vec3<f32> {
    var lin = max(lin0, vec3<f32>(0.0));
    let Y = dot(lin, LUM);
    let k = 0.8;
    if (Y > k) { lin = lin * ((k + (1.0 - k) * (1.0 - exp(-(Y - k) / (1.0 - k)))) / Y); }
    return gamut_clip(lin2lab(lin));
}

fn hash12(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}
fn hash11(x: f32) -> f32 { return fract(sin(x * 127.1 + 11.7) * 43758.5453); }
fn vn(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash12(i), hash12(i + vec2<f32>(1.0, 0.0)), u.x),
               mix(hash12(i + vec2<f32>(0.0, 1.0)), hash12(i + vec2<f32>(1.0, 1.0)), u.x), u.y);
}
fn fbm(p0: vec2<f32>) -> f32 {
    var p = p0;
    var s = 0.0;
    var a = 0.5;
    for (var i = 0; i < 4; i = i + 1) { s = s + a * vn(p); p = p * 2.03 + 17.1; a = a * 0.5; }
    return s;
}

fn ray(px: vec2<f32>) -> vec3<f32> {
    return vec3<f32>((px.x - U.cam.y) / U.cam.x, (px.y - U.cam.z) / U.cam.x, 1.0);
}
fn proj(P: vec3<f32>) -> vec2<f32> {
    return vec2<f32>(U.cam.y + U.cam.x * P.x / P.z, U.cam.z + U.cam.x * P.y / P.z);
}
// A stored normal; one missing (outside a figure) faces the camera.
fn normal_of(g: vec3<f32>) -> vec3<f32> {
    let l = length(g);
    return select(vec3<f32>(0.0, 0.0, -1.0), g / max(l, 1e-6), l > 0.5);
}

// ---- the shadow one layer casts: march toward the light inside its rectangle
fn layer_shadow(i: u32, P: vec3<f32>, LP: vec3<f32>, start_dist: f32, light_r: f32) -> f32 {
    let Ly = U.layers[i];
    let p0 = proj(P);
    let p1 = proj(LP);
    let z0 = P.z;
    let z1 = LP.z;
    let d = p1 - p0;
    let lo = Ly.rect.xy;
    let hi = Ly.rect.xy + Ly.rect.zw;
    var s_in = 0.0;
    var s_out = 1.0;
    // clip the segment to the rectangle, x then y
    if (abs(d.x) < 1e-5) {
        if (p0.x < lo.x || p0.x > hi.x) { return 1.0; }
    } else {
        let ta = (lo.x - p0.x) / d.x;
        let tb = (hi.x - p0.x) / d.x;
        s_in = max(s_in, min(ta, tb));
        s_out = min(s_out, max(ta, tb));
    }
    if (abs(d.y) < 1e-5) {
        if (p0.y < lo.y || p0.y > hi.y) { return 1.0; }
    } else {
        let ta = (lo.y - p0.y) / d.y;
        let tb = (hi.y - p0.y) / d.y;
        s_in = max(s_in, min(ta, tb));
        s_out = min(s_out, max(ta, tb));
    }
    let dist = length(LP - P);
    let t_min = clamp(start_dist / max(dist, 1e-3), 0.0, 1.0);
    s_in = max(s_in, t_min * z1 / (z0 + t_min * (z1 - z0)));
    if (s_out <= s_in) { return 1.0; }
    var occ = 0.0;
    let N = 32;
    for (var k = 0; k < N; k = k + 1) {
        let sp = mix(s_in, s_out, (f32(k) + 0.5) / f32(N));
        let t = (p0 + sp * d - Ly.rect.xy) / Ly.rect.zw;
        let auv = Ly.uv.xy + t * Ly.uv.zw;
        if (textureSampleLevel(at_albedo, s_near, auv, 0.0).a < 0.5) { continue; }
        let tt = sp * z0 / (z1 - sp * (z1 - z0));
        let zq = z0 + tt * (z1 - z0);
        let dz = zq - (Ly.z.x + textureSampleLevel(at_geo, s_near, auv, 0.0).w);
        let soft = 0.03 + light_r * tt / max(1.0 - tt, 0.05);   // the penumbra widens away from the occluder
        occ = max(occ, clamp((dz - 0.05) / soft, 0.0, 1.0) * clamp((0.7 - dz) / 0.25, 0.0, 1.0));
        if (occ > 0.999) { break; }
    }
    return 1.0 - occ;
}
fn shadow_all(P: vec3<f32>, LP: vec3<f32>, start_dist: f32, light_r: f32) -> f32 {
    if (U.misc.y < 0.5) { return 1.0; }
    var s = 1.0;
    let n = u32(U.misc.z);
    for (var i = 0u; i < n; i = i + 1u) {
        s = s * layer_shadow(i, P, LP, start_dist, light_r);
        if (s < 1e-3) { break; }
    }
    return s;
}

// ---- one local light on a surface, added on top of the night pass in linear light
// How much of a light's beam reaches P: 1 inside, fading to 0 over
// BEAM_SOFT_DEG at its edge, measured across the floor (a wedge standing on
// it, gs_field's PointLight::beam); 1 all round and at the light itself.
const BEAM_SOFT_DEG: f32 = 14.0;
fn beam_gain(P: vec3<f32>, Lt: Light) -> f32 {
    if (Lt.beam.w >= 180.0) { return 1.0; }
    let up = U.up.xyz;
    let v0 = P - Lt.pos.xyz;
    let v = v0 - up * dot(v0, up);
    let len = length(v);
    if (len < 1e-4) { return 1.0; }
    let off = degrees(acos(clamp(dot(v / len, Lt.beam.xyz), -1.0, 1.0)));
    let t = clamp((Lt.beam.w + BEAM_SOFT_DEG * 0.5 - off) / BEAM_SOFT_DEG, 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn light_term(P: vec3<f32>, n: vec3<f32>, alb: vec3<f32>, LP: vec3<f32>, col: vec3<f32>, pw: f32, r0: f32,
              range: f32, wrap: f32, bounce: f32, sheen: f32, shin: f32, sh: f32) -> vec3<f32> {
    let Lv = LP - P;
    let d2 = dot(Lv, Lv);
    let dd = sqrt(d2);
    let l = Lv / dd;
    let win = pow(clamp(1.0 - pow(dd / range, 4.0), 0.0, 1.0), 2.0);
    let fall = pw / (d2 + r0 * r0) * win;
    let ndl = dot(n, l);
    let shade = max((ndl + wrap) / (1.0 + wrap), 0.0);
    var o = alb * col * fall * (shade * sh + bounce);
    if (sheen > 0.0) {
        let v = normalize(-P);
        let h = normalize(l + v);
        let ndh = max(dot(n, h), 0.0);
        let vdh = max(dot(v, h), 0.0);
        let fres = 0.04 + 0.96 * pow(1.0 - vdh, 5.0);
        o = o + col * fall * sheen * fres * pow(ndh, shin) * (shin + 8.0) / 8.0 * max(ndl, 0.0) * sh;
    }
    return o;
}

// ---- a torch's flame, sparks, stick and halo
fn flame(px: vec2<f32>, t: f32, T: vec3<f32>, T0: vec3<f32>, flick: f32) -> vec4<f32> {
    let up = U.up.xyz;
    let b = proj(T - up * 0.10);
    let ppm = U.cam.x / T.z;
    let hgt = 0.42 * ppm * (0.85 + 0.25 * flick);
    let q = vec2<f32>((px.x - b.x) / (0.11 * ppm), (b.y - px.y) / hgt);
    if (q.y < -0.35 || q.y > 1.4 || abs(q.x) > 3.0) { return vec4<f32>(0.0); }
    let n = fbm(vec2<f32>(q.x * 1.6, q.y * 2.2 - t * 3.1));
    let n2 = fbm(vec2<f32>(q.x * 3.1 + 7.0, q.y * 4.0 - t * 5.3));
    let yy = clamp(q.y, 0.0, 1.0);
    let x = q.x + (n - 0.5) * 1.7 * pow(yy, 1.2) + (T.x - T0.x) * 6.0 * yy;
    let prof = pow(clamp(1.0 - q.y, 0.0, 1.0), 1.15) * sqrt(clamp(q.y + 0.32, 0.0, 1.0)) * 1.55;
    let dd = abs(x) / max(prof, 1e-3);
    let tip = q.y + (n2 - 0.5) * 0.7 * yy;
    let body = smoothstep(1.0, 0.45, dd) * smoothstep(-0.32, -0.08, q.y) * smoothstep(1.0, 0.5, tip);
    let core = smoothstep(0.5, 0.0, dd) * smoothstep(0.5, 0.0, q.y) * smoothstep(-0.25, -0.02, q.y);
    var c = mix(vec3<f32>(0.9, 0.10, 0.02), vec3<f32>(1.0, 0.42, 0.08), smoothstep(0.05, 0.7, body));
    c = mix(c, vec3<f32>(1.0, 0.80, 0.42), smoothstep(0.2, 1.0, core));
    let I = body * (1.3 + 2.6 * core) * (1.15 - 0.5 * yy) * flick;
    return vec4<f32>(c * I, body);
}
fn sparks(px: vec2<f32>, t: f32, T: vec3<f32>, flick: f32) -> vec3<f32> {
    let b = proj(T);
    let ppm = U.cam.x / T.z;
    var acc = vec3<f32>(0.0);
    for (var i = 0; i < 9; i = i + 1) {
        let fi = f32(i);
        let period = 1.4 + hash11(fi) * 1.6;
        let life = fract(t / period + hash11(fi + 3.1));
        let seed = floor(t / period + hash11(fi + 3.1));
        let off = vec2<f32>((hash11(fi + seed * 7.3) - 0.5) * 0.14 + sin(t * 2.3 + fi * 1.7) * 0.04 * life,
                            -(0.2 + life * (0.3 + 0.35 * hash11(fi + seed))));
        let fade = (1.0 - life) * smoothstep(0.0, 0.08, life);
        let r = length(px - (b + off * ppm));
        acc = acc + vec3<f32>(1.0, 0.55, 0.18) * fade * exp(-r * r / 1.6) * 2.2;
    }
    return acc * flick;
}
fn torch_prop(col0: vec3<f32>, px: vec2<f32>, Lt: Light, nearest_z: f32) -> vec3<f32> {
    var col = col0;
    let t = U.up.w;
    let T = Lt.pos.xyz;
    let T0 = Lt.prop.xyz;
    let flick = Lt.shine.z;
    let tcol = Lt.col.rgb;
    let up = U.up.xyz;
    let ppm = U.cam.x / T.z;
    let hidden = step(nearest_z, T.z - 0.08);
    let top = proj(T - up * 0.12);
    let bot = proj(T - up * Lt.prop.w);
    let ab = bot - top;
    let ap = px - top;
    let s = clamp(dot(ap, ab) / max(dot(ab, ab), 1e-3), 0.0, 1.0);
    let dv = ap - s * ab;
    let dist = length(dv);
    let w = (0.022 + 0.018 * smoothstep(0.14, 0.08, s * length(ab) / ppm)) * ppm;
    let stick = clamp(w - dist + 0.5, 0.0, 1.0) * (1.0 - hidden);
    let lit = exp(-s * length(ab) / ppm * 6.0) * flick;
    let side = clamp(0.5 - dv.x / max(w, 1.0) * 0.5, 0.0, 1.0);
    let stick_col = vec3<f32>(0.045, 0.026, 0.016) * (0.25 + tcol * (0.6 + lit * (4.0 + 6.0 * side)));
    col = mix(col, stick_col, stick);
    let r = length(px - proj(T - up * 0.05));
    let halo = (0.45 * exp(-pow(r / (0.22 * ppm), 2.0)) + 0.05 / (1.0 + pow(r / (0.6 * ppm), 2.0))) * flick;
    col = col + tcol * halo * (1.0 - 0.75 * hidden);
    col = col + sparks(px, t, T, flick) * (1.0 - hidden);
    let fl = flame(px, t, T, T0, flick) * (1.0 - hidden);
    return col * (1.0 - 0.5 * fl.a) + fl.rgb;
}

// ---- a mote: a hot core, a halo, and motes of light circling it
fn mote_prop(px: vec2<f32>, Lt: Light, nearest_z: f32) -> vec3<f32> {
    let t = U.up.w;
    let M = Lt.pos.xyz;
    let mcol = Lt.col.rgb;
    let mcore = Lt.prop.xyz;
    let pulse = Lt.shine.z;
    let hidden = step(nearest_z, M.z - 0.05);
    let c = proj(M);
    let ppm = U.cam.x / M.z;
    let r = length(px - c);
    let core = exp(-pow(r / (0.05 * ppm), 2.0));
    let halo = 0.6 * exp(-pow(r / (0.16 * ppm), 2.0)) + 0.22 * exp(-pow(r / (0.9 * ppm), 2.0)) + 0.03 / (1.0 + pow(r / (0.5 * ppm), 2.0));
    var acc = mcore * core * 3.0 * pulse * (1.0 - hidden) + mcol * halo * pulse * (1.0 - 0.7 * hidden);
    for (var i = 0; i < 10; i = i + 1) {
        let fi = f32(i);
        let ang = t * (0.5 + 0.9 * hash11(fi)) * select(-1.0, 1.0, hash11(fi + 9.0) > 0.5) + fi * 2.39996;
        let rad = (0.07 + 0.16 * hash11(fi + 2.0)) * ppm;
        let pos = c + vec2<f32>(cos(ang) * rad, sin(ang) * rad * 0.45 - (0.03 + 0.05 * hash11(fi + 5.0)) * ppm * sin(t * 0.8 + fi));
        let tw = 0.5 + 0.5 * sin(t * (3.0 + 3.0 * hash11(fi + 4.0)) + fi * 1.3);
        let rr = length(px - pos);
        acc = acc + mix(mcol, mcore, 0.5) * tw * exp(-rr * rr / 2.0) * 1.4 * (1.0 - hidden);
    }
    return acc;
}

// What one surface shows at a pixel, before the lights' own flame / glow.
struct Surf {
    col: vec3<f32>,    // lit colour, linear
    a: f32,            // coverage
    z: f32,            // depth, metres
    n: vec3<f32>,      // normal (debug view)
    alb: vec3<f32>,    // colour, sRGB (debug view)
};

fn backdrop_at(px: vec2<f32>, light_only: bool) -> Surf {
    let uv = (px - U.bgmap.xy) / U.bgmap.zw;
    let bg_a = textureSampleLevel(bg_albedo, s_lin, uv, 0.0);
    let bg_l = textureSampleLevel(bg_lit, s_lin, uv, 0.0);
    let bg_g = textureSampleLevel(bg_geo, s_near, uv, 0.0);
    let bg_n = normal_of(bg_g.xyz);
    let bg_z = bg_g.w;
    let sky = bg_z > 60.0;
    let bg_p = ray(px) * bg_z;
    let alb_bg = s2l(bg_a.rgb);
    // dark painted pixels count as water, where the scene has any
    let water = (1.0 - clamp(dot(alb_bg, LUM) * 3.0, 0.0, 1.0)) * U.misc.x;
    let wet = 0.25 + 0.75 * water;
    var col = select(s2l(bg_l.rgb), vec3<f32>(0.0), light_only);
    if (!sky) {
        let n_lights = u32(U.misc.w);
        for (var li = 0u; li < n_lights; li = li + 1u) {
            let Lt = U.lights[li];
            let gain = beam_gain(bg_p, Lt);
            if (gain <= 0.0) { continue; }
            col = col + light_term(bg_p, bg_n, alb_bg, Lt.pos.xyz, Lt.col.rgb, Lt.col.w * gain, Lt.fall.x, Lt.fall.y,
                                   Lt.surf.y, Lt.fall.w, wet * Lt.surf.w, Lt.shine.y,
                                   shadow_all(bg_p, Lt.pos.xyz, 0.08, Lt.fall.z));
        }
    }
    col = mix(col, alb_bg * 2.2, bg_l.a);
    var s: Surf;
    s.col = col;
    s.a = 1.0;
    s.z = bg_z;
    s.n = select(bg_n, vec3<f32>(0.0, 0.0, -1.0), sky);
    s.alb = bg_a.rgb;
    return s;
}

fn layer_at(i: u32, px: vec2<f32>, light_only: bool) -> Surf {
    var s: Surf;
    s.a = 0.0;
    s.z = 1e4;
    let Ly = U.layers[i];
    let t = (px - Ly.rect.xy) / Ly.rect.zw;
    if (any(t < vec2<f32>(0.0)) || any(t > vec2<f32>(1.0))) { return s; }
    let auv = Ly.uv.xy + t * Ly.uv.zw;
    let a4 = textureSampleLevel(at_albedo, s_lin, auv, 0.0);
    if (a4.a <= 0.0) { return s; }
    let g4 = textureSampleLevel(at_geo, s_near, auv, 0.0);
    let l4 = textureSampleLevel(at_lit, s_lin, auv, 0.0);
    let n = normal_of(g4.xyz);
    let z = Ly.z.x + g4.w;
    let P = ray(px) * z;
    let alb = s2l(a4.rgb) * Ly.z.y;
    var c = select(s2l(l4.rgb) * Ly.z.y, vec3<f32>(0.0), light_only);
    let n_lights = u32(U.misc.w);
    for (var li = 0u; li < n_lights; li = li + 1u) {
        let Lt = U.lights[li];
        let gain = beam_gain(P, Lt);
        if (gain <= 0.0) { continue; }
        c = c + light_term(P, n, alb, Lt.pos.xyz, Lt.col.rgb, Lt.col.w * gain, Lt.fall.x, Lt.fall.y,
                           Lt.surf.x, Lt.fall.w, Lt.surf.z, Lt.shine.x,
                           shadow_all(P, Lt.pos.xyz, 0.15, Lt.fall.z));
    }
    s.col = mix(c, alb * 2.2, l4.a);               // glowing parts: never lit, never darkened
    s.a = a4.a;
    s.z = z;
    s.n = n;
    s.alb = a4.rgb;
    return s;
}

// The lights' own flame, stick and glow, hidden where a surface nearer than
// them covers the pixel.
fn props(col0: vec3<f32>, px: vec2<f32>, nearest_z: f32) -> vec3<f32> {
    var col = col0;
    let n_lights = u32(U.misc.w);
    for (var li = 0u; li < n_lights; li = li + 1u) {
        let Lt = U.lights[li];
        if (Lt.shine.w < 0.5) { continue; }
        if (Lt.pos.w < 0.5) {
            col = torch_prop(col, px, Lt, nearest_z);
        } else {
            col = col + mote_prop(px, Lt, nearest_z);
        }
    }
    return col;
}

fn encode(col: vec3<f32>) -> vec3<f32> {
    let lin = tonemap(col);
    return select(l2s(lin), lin, U.size.z > 0.5);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let px = in.pos.xy * D.map.xy + D.map.zw;
    let mode = u32(D.mode.x + 0.5);
    let view = i32(U.size.w + 0.5);
    let light_only = view == 1;

    // one creature layer, premultiplied over what is drawn already
    if (mode == 2u) {
        let s = layer_at(u32(D.mode.y + 0.5), px, light_only);
        if (s.a <= 0.0) { discard; }
        let c = encode(props(s.col, px, select(1e4, s.z, s.a > 0.5)));
        return vec4<f32>(c * s.a, s.a);
    }

    let bg = backdrop_at(px, light_only);
    if (mode == 1u) {
        return vec4<f32>(encode(props(bg.col, px, bg.z)), 1.0);
    }

    // everything: the backdrop, then the layers far to near
    var col = bg.col;
    var nearest_z = bg.z;
    var n_view = bg.n;
    var a_view = bg.alb;
    let n_layers = u32(U.misc.z);
    for (var i = 0u; i < n_layers; i = i + 1u) {
        let s = layer_at(i, px, light_only);
        if (s.a <= 0.0) { continue; }
        col = mix(col, s.col, s.a);
        if (s.a > 0.5) { nearest_z = min(nearest_z, s.z); }
        n_view = mix(n_view, s.n, s.a);
        a_view = mix(a_view, s.alb, s.a);
    }
    if (view == 2) { return vec4<f32>(n_view * 0.5 + 0.5, 1.0); }
    if (view == 3) {
        let g = clamp(1.0 - log(nearest_z / 3.0) / log(40.0), 0.0, 1.0);
        return vec4<f32>(vec3<f32>(g), 1.0);
    }
    if (view == 4) { return vec4<f32>(a_view, 1.0); }
    return vec4<f32>(encode(props(col, px, nearest_z)), 1.0);
}
