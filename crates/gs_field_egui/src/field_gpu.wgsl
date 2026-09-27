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
    z: vec4<f32>,      // x = the depth of its feet, metres; y = colour multiplier, linear; zw = its figure's nearest and farthest depth from the feet
};

struct Light {
    pos: vec4<f32>,    // xyz = camera space, metres (the moon: toward it, unit); w = kind (0 torch, 1 mote, 2 moon)
    col: vec4<f32>,    // rgb = this frame's colour, linear; w = power (flicker in)
    fall: vec4<f32>,   // r0, range, light radius (shadow softness), bounce
    surf: vec4<f32>,   // wrap on creatures, wrap on the ground, sheen on creatures, sheen on the ground (x wetness)
    shine: vec4<f32>,  // shininess on creatures, on the ground, brightness factor (flicker / pulse), prop shown
    prop: vec4<f32>,   // torch: flame base without sway (xyz), stick length; mote: core colour (rgb); moon: cloud cover, wind across, wind deeper (m/s), phase (0 new .. 0.5 full; +1: Tilaok circles it)
    beam: vec4<f32>,   // the beam's axis across the floor (xyz, camera space), half its width in degrees (>= 180: all round); moon: where its disc is drawn (xyz), its angular radius (0 = none)
};

// The weather (FieldWeather; all zero: none).
struct Weather {
    a: vec4<f32>,          // lightning now, where its cloud is (0..1 across), mist density at the ground, mist height (m)
    b: vec4<f32>,          // rain amount, rain slant (wind), snowfall, snow lying
    c: vec4<f32>,          // blowing sand, its wind (m/s), the snow's wind (m/s), mist drift (m/s)
    fog: vec4<f32>,        // the night air's own light, rgb linear
    flash_tint: vec4<f32>, // lightning's colour, rgb linear
    dust: vec4<f32>,       // blowing sand's colour, rgb linear
    rain_tint: vec4<f32>,  // the rain's colour, rgb linear (white; blood)
};

struct Field {
    size: vec4<f32>,   // scene w, h, target is sRGB (1 = write linear), view
    cam: vec4<f32>,    // f, cx, cy, camera height (m)
    up: vec4<f32>,     // world up in camera space (xyz), time (s)
    misc: vec4<f32>,   // water, shadows on, layer count, light count
    bgmap: vec4<f32>,  // where the backdrop image lies, scene px: x, y, w, h
    layers: array<Layer, 32>,
    lights: array<Light, 8>,
    weather: Weather,
    bounce: vec4<f32>, // strength (0 = none), gather radius (m), scene px per bounce texel
};

// One piece of the frame.
struct Draw {
    ndc: vec4<f32>,    // its quad, normalised device coords: x0, y0, x1, y1
    map: vec4<f32>,    // scene px = framebuffer px * map.xy + map.zw
    mode: vec4<f32>,   // x: 0 everything, 1 the backdrop, 2 one layer (y = its index), 3 the bounce pass
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
// the frame's direct light (what each surface reflects) and its surfaces
// (normal, depth; 0 = nothing or sky), for bounce
@group(0) @binding(9) var bn_light: texture_2d<f32>;
@group(0) @binding(10) var bn_surf: texture_2d<f32>;
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
    // The ray can be blocked only where its depth is just behind the
    // figure's surface (the band tested below) -- inside the figure's slab,
    // Ly.z.zw. The march keeps its N places along the whole crossing (moving
    // them into the slab would alias thin parts into stripes) and visits
    // only those in the slab: the same shadow, most samples skipped.
    if (Ly.z.w < Ly.z.z) { return 1.0; }
    let zlo = Ly.z.x + Ly.z.z + 0.05;
    let zhi = Ly.z.x + Ly.z.w + 0.7;
    var c_in = s_in;
    var c_out = s_out;
    if (abs(z1 - z0) < 1e-4) {
        if (z0 <= zlo || z0 >= zhi) { return 1.0; }
    } else {
        // depth runs linearly in tt, the screen in sp (projective)
        let ta = clamp((zlo - z0) / (z1 - z0), 0.0, 1.0);
        let tb = clamp((zhi - z0) / (z1 - z0), 0.0, 1.0);
        let sa = ta * z1 / (z0 + ta * (z1 - z0));
        let sb = tb * z1 / (z0 + tb * (z1 - z0));
        c_in = max(c_in, min(sa, sb));
        c_out = min(c_out, max(sa, sb));
    }
    if (c_out <= c_in) { return 1.0; }
    var occ = 0.0;
    let N = 32;
    let span = s_out - s_in;
    let k_lo = max(i32(floor((c_in - s_in) / span * f32(N) - 0.5)), 0);
    let k_hi = min(i32(ceil((c_out - s_in) / span * f32(N) - 0.5)), N - 1);
    for (var k = k_lo; k <= k_hi; k = k + 1) {
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

// The shadow (`shadow_all` from `start` metres out, light radius `light_r`)
// is marched only where it can matter: within the light's range, on a
// surface facing it -- most of a frame's pixels, for a lamp, are neither.
fn light_term(P: vec3<f32>, n: vec3<f32>, alb: vec3<f32>, LP: vec3<f32>, col: vec3<f32>, pw: f32, r0: f32,
              range: f32, wrap: f32, bounce: f32, sheen: f32, shin: f32, start: f32, light_r: f32) -> vec3<f32> {
    let Lv = LP - P;
    let d2 = dot(Lv, Lv);
    let dd = sqrt(d2);
    let l = Lv / dd;
    let win = pow(clamp(1.0 - pow(dd / range, 4.0), 0.0, 1.0), 2.0);
    if (win <= 0.0) { return vec3<f32>(0.0); }
    let fall = pw / (d2 + r0 * r0) * win;
    let ndl = dot(n, l);
    let shade = max((ndl + wrap) / (1.0 + wrap), 0.0);
    var sh = 0.0;
    if (shade > 0.0) { sh = shadow_all(P, LP, start, light_r); }
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

// ---- the moon: a light at infinity, from the sky
const CLOUD_H: f32 = 600.0;      // the cloud deck, metres up
const CLOUD_SCALE: f32 = 16.0;   // metres of it per noise unit
// How far toward the moon a shadow march runs: 60 m, or until the ray
// would pass the camera (the moon is usually behind the viewer).
fn moon_reach(P: vec3<f32>, l: vec3<f32>) -> f32 {
    var t = 60.0;
    if (l.z < -1e-4) { t = min(t, (P.z - 0.3) / -l.z); }
    return max(t, 0.0);
}
// 1 where the moon shines through, 0 under cloud: a drifting noise field on
// the cloud deck, read where the ray from P to the moon crosses it. `c` of
// the sky is covered.
fn cloud_mask(P: vec3<f32>, Lt: Light) -> f32 {
    let c = Lt.prop.x;
    if (c <= 0.001) { return 1.0; }
    if (c >= 0.999) { return 0.0; }
    let up = U.up.xyz;
    let l = Lt.pos.xyz;
    let h = U.cam.w + dot(P, up);
    let Q = P + l * ((CLOUD_H - h) / max(dot(l, up), 0.05));
    let deeper = vec3<f32>(0.0, up.z, -up.y);
    let uv = (vec2<f32>(Q.x, dot(Q, deeper)) - Lt.prop.yz * U.up.w) / CLOUD_SCALE;
    let n = fbm(uv);
    let thr = 0.47 + 0.56 * (c - 0.5);
    // far off, the patches crowd together: toward the average, which the
    // sky past the flat ground takes, so the two meet without a seam
    return mix(smoothstep(thr - 0.05, thr + 0.05, n), 1.0 - 0.9 * c, smoothstep(25.0, 60.0, P.z));
}
fn moon_term(P: vec3<f32>, n: vec3<f32>, alb: vec3<f32>, Lt: Light, wrap: f32, sheen: f32, shin: f32, start: f32) -> vec3<f32> {
    let l = Lt.pos.xyz;
    let ndl = dot(n, l);
    let shade = max((ndl + wrap) / (1.0 + wrap), 0.0);
    var lit = 0.0;
    if (shade > 0.0) {
        let t = moon_reach(P, l);
        lit = cloud_mask(P, Lt);
        if (lit > 0.0) { lit = lit * shadow_all(P, P + l * t, start, Lt.fall.z * t); }
    }
    var o = alb * Lt.col.rgb * Lt.col.w * (shade * lit + Lt.fall.w);
    if (sheen > 0.0 && lit > 0.0) {
        let v = normalize(-P);
        let h = normalize(l + v);
        let fres = 0.04 + 0.96 * pow(1.0 - max(dot(v, h), 0.0), 5.0);
        o = o + Lt.col.rgb * Lt.col.w * sheen * fres * pow(max(dot(n, h), 0.0), shin) * (shin + 8.0) / 8.0 * max(ndl, 0.0) * lit;
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
    // The halo is in the air round the flame: anything well in front of it
    // hides it (the prototype kept a quarter of it, which laid a glow over
    // whatever stood before the torch); only an occluder at the flame
    // itself -- a hand round the torch -- lets the glare spill over.
    let glare_hidden = smoothstep(0.08, 0.6, T.z - nearest_z);
    col = col + tcol * halo * (1.0 - glare_hidden);
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
    // the halo hidden by anything well in front of it, as the torch's
    let glare_hidden = smoothstep(0.05, 0.6, M.z - nearest_z);
    var acc = mcore * core * 3.0 * pulse * (1.0 - hidden) + mcol * halo * pulse * (1.0 - glare_hidden);
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
// ---- weather (the prototype's, lights-and-weather.md "Weather"; U.weather)
const SNOW_SRGB = vec3<f32>(0.93, 0.95, 0.98);
fn hash31(p0: vec3<f32>) -> f32 {
    var p = fract(p0 * 0.1031);
    p = p + dot(p, p.zyx + 31.32);
    return fract((p.x + p.y) * p.z);
}
fn vn3(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(mix(hash31(i), hash31(i + vec3<f32>(1.0, 0.0, 0.0)), u.x),
                   mix(hash31(i + vec3<f32>(0.0, 1.0, 0.0)), hash31(i + vec3<f32>(1.0, 1.0, 0.0)), u.x), u.y),
               mix(mix(hash31(i + vec3<f32>(0.0, 0.0, 1.0)), hash31(i + vec3<f32>(1.0, 0.0, 1.0)), u.x),
                   mix(hash31(i + vec3<f32>(0.0, 1.0, 1.0)), hash31(i + vec3<f32>(1.0, 1.0, 1.0)), u.x), u.y), u.z);
}
fn fbm3(p: vec3<f32>) -> f32 { return 0.62 * vn3(p) + 0.38 * vn3(p * 2.3 + 7.1); }
// along the ground, away from the camera
fn ground_fwd() -> vec3<f32> { return vec3<f32>(0.0, U.up.z, -U.up.y); }
fn fmod(x: f32, y: f32) -> f32 { return x - y * floor(x / y); }
fn sq(x: f32) -> f32 { return x * x; }
// The camera's ground plane under pixel px (splashes and streaming sand sit
// on it: a painted depth is too flat up close), and the ground one pixel
// spans there, metres.
fn ground_under(px: vec2<f32>) -> vec3<f32> {
    let r = ray(px);
    return r * (U.cam.w / max(-dot(r, U.up.xyz), 1e-3));
}
fn ground_foot(z: f32) -> f32 { return z / U.cam.x + z * z / (U.cam.x * max(U.cam.w, 0.3)); }

// Light arriving at a point in the air: the night air's own, the lamps
// near it, the moon, the lightning.
fn air_light(P: vec3<f32>) -> vec3<f32> {
    var L = U.weather.fog.rgb;
    let n_lights = u32(U.misc.w);
    for (var li = 0u; li < n_lights; li = li + 1u) {
        let Lt = U.lights[li];
        if (Lt.pos.w > 1.5) {
            L = L + Lt.col.rgb * Lt.col.w * 0.05 * (1.0 - 0.9 * Lt.prop.x);
            continue;
        }
        let d = Lt.pos.xyz - P;
        let k = select(0.05, 0.07, Lt.pos.w > 0.5);
        L = L + Lt.col.rgb * Lt.col.w / (dot(d, d) + Lt.fall.x * Lt.fall.x) * k * beam_gain(P, Lt);
    }
    return L + U.weather.flash_tint.rgb * U.weather.a.x * 0.035;
}

// gust fronts: bands of stronger wind travelling across the ground with it
fn gust_at(P: vec3<f32>, wind: f32) -> f32 {
    let t = U.up.w;
    let g = vec2<f32>(P.x - t * wind, dot(P, ground_fwd()));
    return smoothstep(0.3, 0.85, vn(vec2<f32>(g.x * 0.07, g.y * 0.05 + t * 0.04)));
}

// Mist, blowing sand, blizzard haze and rain haze between the camera and a
// surface z_to metres out along this pixel's ray (12 steps, to 30 m at
// most): xyz = the light they add, w = the share of the surface that shows.
fn medium(px: vec2<f32>, z_to: f32) -> vec4<f32> {
    let W = U.weather;
    if (W.a.z <= 0.0 && W.c.x <= 0.0 && W.b.z <= 0.0 && W.b.x <= 0.0) { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }
    let dir = ray(px);
    let z_max = min(z_to, 30.0);
    let dl = length(dir);
    let fwd = ground_fwd();
    let t = U.up.w;
    var T = 1.0;
    var S = vec3<f32>(0.0);
    var prev = 0.0;
    for (var i = 0; i < 12; i = i + 1) {
        let z1 = z_max * pow((f32(i) + 1.0) / 12.0, 1.5);
        let P = dir * (0.5 * (prev + z1));
        let h = dot(P, U.up.xyz) + U.cam.w;
        let gz = dot(P, fwd);
        var dM = 0.010 * W.b.x;
        var dS = 0.0;
        var dW = 0.0;
        if (W.a.z > 0.0) {
            let n = fbm3(vec3<f32>(P.x + t * W.c.w, h * 1.8, gz + t * W.c.w * 0.35) * 0.55);
            dM = dM + W.a.z * exp(-max(h, 0.0) / max(W.a.w, 0.05)) * (0.2 + 1.6 * smoothstep(0.32, 0.78, n));
        }
        if (W.c.x > 0.0) {
            let g = mix(gust_at(P, W.c.y), 1.0, W.c.x * W.c.x);
            let n = fbm3(vec3<f32>((P.x - t * W.c.y * 0.9) * 0.18, h * 0.7, gz * 0.35 + t * 0.1));
            dS = (0.015 + 0.30 * W.c.x) * g * exp(-max(h, 0.0) / (1.0 + 5.0 * W.c.x)) * (0.3 + 1.4 * smoothstep(0.25, 0.8, n));
        }
        if (W.b.z > 0.0) {
            let n = fbm3(vec3<f32>((P.x - t * W.c.z) * 0.25, h * 0.5 - t * 0.3, gz * 0.3));
            dW = (0.004 + 0.16 * W.b.z * W.b.z) * (0.5 + smoothstep(0.3, 0.8, n));
        }
        let dens = dM + dS + dW;
        if (dens > 0.0) {
            let tint = (vec3<f32>(dM) + W.dust.rgb * 1.8 * dS + vec3<f32>(1.25) * dW) / dens;
            let a = 1.0 - exp(-dens * (z1 - prev) * dl);
            S = S + T * a * air_light(P) * tint;
            T = T * (1.0 - a);
        }
        prev = z1;
    }
    return vec4<f32>(S, T);
}

// rain: streaks on slabs at set distances, hidden behind anything nearer
fn rain_streaks(px: vec2<f32>, scene_z: f32) -> vec3<f32> {
    let W = U.weather;
    var acc = vec3<f32>(0.0);
    var zs = array<f32, 7>(1.7, 2.5, 3.5, 4.8, 6.6, 9.2, 13.5);
    let t = U.up.w;
    for (var i = 0; i < 7; i = i + 1) {
        let zL = zs[i];
        if (zL >= scene_z) { break; }
        let fi = f32(i) * 13.1;
        let ppm = U.cam.x / zL;
        var w = (px - U.cam.yz) / ppm;                        // metres on the slab, y down
        w.x = w.x - W.b.y * w.y;                              // wind slants the fall
        let cw = 0.05 + 0.006 * zL;
        let col = floor(w.x / cw);
        if (hash12(vec2<f32>(col, fi + 1.0)) > 0.25 + 0.75 * W.b.x) { continue; }
        let xoff = (hash12(vec2<f32>(col, fi + 2.0)) - 0.5) * 0.7 * cw;
        let speed = 7.5 + 2.5 * hash12(vec2<f32>(col, fi + 3.0));
        let period = mix(3.2, 0.9, W.b.x) * (0.7 + 0.6 * hash12(vec2<f32>(col, fi + 4.0)));
        let fy = fmod(w.y - t * speed + hash12(vec2<f32>(col, fi + 5.0)) * period, period);
        let along = fy / (speed * 0.028);
        if (along > 1.0) { continue; }
        let dxp = abs(w.x - (col + 0.5) * cw - xoff) * ppm;
        let wpx = 0.0022 * ppm;
        var cov = clamp(1.0 - dxp / (wpx + 0.6), 0.0, 1.0) * min(1.0, wpx / 0.6 + 0.25);
        cov = cov * (1.0 - along) * smoothstep(0.0, 0.12, along);
        let focus = mix(0.35, 1.0, smoothstep(1.5, 3.5, zL));    // nearest drops are out of focus
        let haze = exp(-W.a.z * 0.35 * zL);
        let Lc = W.fog.rgb * 2.2 + (air_light(ray(px) * zL) - W.fog.rgb) * 6.0;
        acc = acc + Lc * cov * focus * haze * (0.6 + 0.4 * hash12(vec2<f32>(col, fi + 6.0)));
    }
    return acc * W.rain_tint.rgb;
}

// blowing sand: short fast streaks along the wind on slabs at set
// distances, thickest near the ground and in the gusts
fn sand_grains(px: vec2<f32>, scene_z: f32) -> vec3<f32> {
    let W = U.weather;
    var acc = vec3<f32>(0.0);
    var zs = array<f32, 7>(1.6, 2.4, 3.4, 4.7, 6.4, 8.8, 12.5);
    let t = U.up.w;
    let amt = W.c.x;
    let dir = select(-1.0, 1.0, W.c.y >= 0.0);
    let speed0 = abs(W.c.y);
    for (var i = 0; i < 7; i = i + 1) {
        let zL = zs[i];
        if (zL >= scene_z) { break; }
        let fi = f32(i) * 17.3;
        let ppm = U.cam.x / zL;
        let P = ray(px) * zL;
        let h = dot(P, U.up.xyz) + U.cam.w;
        if (h < -0.02) { continue; }
        let lift = exp(-max(h, 0.0) / (0.35 + 2.2 * amt));         // most sand skims low
        let g = mix(gust_at(P, W.c.y), 1.0, amt * amt);
        var w = (px - U.cam.yz) / ppm;
        w.y = w.y + 0.02 * sin(w.x * 3.0 + t * 5.0 + fi);           // a little tumble
        let ch = 0.03 + 0.003 * zL;
        let row = floor(w.y / ch);
        if (hash12(vec2<f32>(row, fi + 1.0)) > (0.15 + 0.85 * amt) * lift * (0.35 + 0.65 * g)) { continue; }
        let yoff = (hash12(vec2<f32>(row, fi + 2.0)) - 0.5) * 0.6 * ch;
        let sp = speed0 * (0.75 + 0.5 * hash12(vec2<f32>(row, fi + 3.0)));
        let period = mix(2.4, 0.5, amt) * (0.7 + 0.6 * hash12(vec2<f32>(row, fi + 4.0)));
        let fx = fmod(w.x * dir - t * sp + hash12(vec2<f32>(row, fi + 5.0)) * period, period);
        let len = sp * (0.008 + 0.014 * hash12(vec2<f32>(row, fi + 6.0)));
        let along = fx / max(len, 1e-3);
        if (along > 1.0) { continue; }
        let dyp = abs(w.y - (row + 0.5) * ch - yoff) * ppm;
        let wpx = 0.0011 * ppm;
        var cov = clamp(1.0 - dyp / (wpx + 0.6), 0.0, 1.0) * min(1.0, wpx / 0.6 + 0.2);
        cov = cov * (1.0 - along) * smoothstep(0.0, 0.15, along);
        let focus = mix(0.35, 1.0, smoothstep(1.4, 3.4, zL));
        let Lc = W.dust.rgb * (W.fog.rgb * 2.5 + (air_light(P) - W.fog.rgb) * 6.0);
        acc = acc + Lc * cov * focus * (0.35 + 0.65 * lift);
    }
    return acc;
}

// snowflakes: soft discs drifting down and across with the wind, wobbling,
// streaked in a blizzard, on slabs at set distances
fn snow_flakes(px: vec2<f32>, scene_z: f32) -> vec3<f32> {
    let W = U.weather;
    var acc = vec3<f32>(0.0);
    var zs = array<f32, 7>(1.4, 2.1, 3.0, 4.3, 6.0, 8.5, 12.5);
    let t = U.up.w;
    let vel = vec2<f32>(W.c.z, mix(0.9, 2.6, W.b.z));                // m/s, y down
    let spd = length(vel);
    let vdir = vel / spd;
    for (var i = 0; i < 7; i = i + 1) {
        let zL = zs[i];
        if (zL >= scene_z) { break; }
        let fi = f32(i) * 11.7;
        let ppm = U.cam.x / zL;
        let w = (px - U.cam.yz) / ppm;
        let q = w - vel * t;
        let cs = 0.13;
        let cell = floor(q / cs);
        if (hash12(cell + fi) > mix(0.22, 0.95, W.b.z)) { continue; }
        var c = (cell + 0.3 + 0.4 * vec2<f32>(hash12(cell + fi + 1.0), hash12(cell + fi + 2.0))) * cs;
        c.x = c.x + 0.02 * sin(t * (1.1 + hash12(cell + fi + 3.0)) + 6.28 * hash12(cell + fi + 4.0));   // wobble
        let d = q - c;
        let blur = spd * 0.006;                                          // motion blur over ~1/160 s
        let along = max(abs(dot(d, vdir)) - blur * 0.5, 0.0);
        let across = dot(d, vec2<f32>(-vdir.y, vdir.x));
        let hr = hash12(cell + fi + 5.0);
        let r = 0.002 + 0.005 * hr * hr;
        let focus = smoothstep(1.3, 3.2, zL);
        let rpx = r * ppm;
        let soft = mix(rpx * 0.9, 0.8, focus);                           // the nearest are out of focus
        let dpx = length(vec2<f32>(along, across)) * ppm;
        var cov = (1.0 - smoothstep(max(rpx - soft, 0.0), rpx + soft, dpx)) * min(1.0, r * ppm / 0.6 + 0.15);
        cov = cov * mix(0.3, 1.0, focus) / (1.0 + blur * ppm / max(rpx, 0.5) * 0.35);
        acc = acc + (W.fog.rgb * 4.0 + (air_light(ray(px) * zL) - W.fog.rgb) * 8.0) * cov;
    }
    return acc;
}

// everything falling between the camera and a surface scene_z metres out
fn weather_front(px: vec2<f32>, scene_z: f32) -> vec3<f32> {
    let W = U.weather;
    var c = vec3<f32>(0.0);
    if (W.b.x > 0.0) { c = c + rain_streaks(px, scene_z); }
    if (W.c.x > 0.0) { c = c + sand_grains(px, scene_z); }
    if (W.b.z > 0.0) { c = c + snow_flakes(px, scene_z); }
    return c;
}

// a drop landing near this ground point: a crown on stone, rings on water
fn splash(P: vec3<f32>, wet: f32, foot: f32) -> f32 {
    let rain = U.weather.b.x;
    let g = vec2<f32>(P.x, dot(P, ground_fwd()));
    let cs = 0.3;
    let cell = floor(g / cs);
    let h0 = hash12(cell + 3.7);
    let per = mix(1.8, 0.45, rain) * (0.8 + 0.4 * h0);
    let tt = U.up.w / per + h0;
    let cyc = floor(tt);
    let age = fract(tt);
    if (hash12(cell + cyc * 1.37) > 0.3 + 0.7 * rain) { return 0.0; }
    let c = (cell + 0.25 + 0.5 * vec2<f32>(hash12(cell + cyc * 2.1), hash12(cell + cyc * 3.3 + 9.0))) * cs;
    let d = length(g - c);
    let w = 0.008 + 0.004 * age;
    let we = max(w, 0.7 * foot);
    let ring = exp(-sq((d - (0.01 + age * 0.075)) / we)) * pow(1.0 - age, 1.8) * (w / we);
    let wc = max(0.012, 0.7 * foot);
    let crown = exp(-sq(d / wc)) * (1.0 - smoothstep(0.0, 0.3, age)) * (0.012 / wc);
    return mix(crown, ring, wet);
}

// Lightning lifts the night for a moment and cools it (the prototype
// raises the night pass's exposure; here its result, in linear light).
fn storm_lift(c: vec3<f32>) -> vec3<f32> {
    let f = U.weather.a.x;
    if (f <= 0.0) { return c; }
    let lifted = c * (1.0 + 2.2 * f);
    let tint = U.weather.flash_tint.rgb;
    let cool = dot(lifted, LUM) * tint / max(dot(tint, LUM), 1e-4);
    return mix(lifted, cool, min(1.0, 0.6 * f) * 0.6);
}

// Snow settled on a surface, as the pixel's own light would show it: the
// light its night result gave its colour, now on snow.
fn snowed(base: vec3<f32>, alb: vec3<f32>, cov: f32) -> vec3<f32> {
    let lit = dot(base, LUM) / max(dot(alb, LUM), 1e-3);
    return mix(base, s2l(SNOW_SRGB) * lit, cov);
}

struct Surf {
    col: vec3<f32>,    // lit colour, linear
    a: f32,            // coverage
    z: f32,            // depth, metres
    n: vec3<f32>,      // normal (debug view)
    alb: vec3<f32>,    // colour, sRGB (debug view)
};

// ---- bounce: what the lit surfaces near P reflect onto it
// One bounce, in screen space: taps on a disc round the pixel -- as wide on
// screen as U.bounce.y metres at its depth -- read the frame's direct light
// (the bounce pass, D.mode.x = 3, which gathers nothing itself); each tap is
// a patch of the surface there, its light reaching P by how the two face
// each other over the distance. Fixed taps, no jitter (it speckles).
const BOUNCE_TAPS: i32 = 12;
fn bounce_at(P: vec3<f32>, n: vec3<f32>, px: vec2<f32>) -> vec3<f32> {
    if (U.bounce.x <= 0.0 || u32(D.mode.x + 0.5) == 3u) { return vec3<f32>(0.0); }
    let f = U.cam.x;
    let r_px = clamp(U.bounce.y * f / max(P.z, 0.1), 3.0, 0.3 * U.size.y);
    let tap_px = 3.14159 * r_px * r_px / f32(BOUNCE_TAPS);
    var acc = vec3<f32>(0.0);
    for (var k = 0; k < BOUNCE_TAPS; k = k + 1) {
        let a = f32(k) * 2.3999632;
        let q = px + vec2<f32>(cos(a), sin(a)) * sqrt((f32(k) + 0.5) / f32(BOUNCE_TAPS)) * r_px;
        if (any(q < vec2<f32>(0.0)) || any(q > U.size.xy)) { continue; }
        let uv = q / U.size.xy;
        let s = textureSampleLevel(bn_surf, s_near, uv, 0.0);
        if (s.w <= 0.0) { continue; }
        let Q = ray(q) * s.w;
        let d = Q - P;
        let d2 = dot(d, d);
        if (d2 < 1e-4) { continue; }
        let w = d * inverseSqrt(d2);
        let facing = max(dot(n, w), 0.0) * max(dot(s.xyz, -w), 0.0);
        if (facing <= 0.0) { continue; }
        // the patch the tap stands for, m^2: its screen area at its depth,
        // turned the way it faces
        let area = tap_px * (s.w / f) * (s.w / f) / max(abs(dot(s.xyz, normalize(Q))), 0.2);
        let light = textureSampleLevel(bn_light, s_lin, uv, 0.0).rgb;
        acc = acc + light * facing * area / (3.14159 * d2 + area);
    }
    return acc * U.bounce.x;
}

// ---- the moons themselves, in the sky: a disc per moon light with one
// (beam.w its angular radius, beam.xyz where it is drawn -- its height
// squeezed into the frame's sky), lit to its phase (fract(prop.w), growing
// lit on the right), dim on its dark side, under the cloud crossing it,
// with a faint glow; Tilaok, small and red, circles one with prop.w >= 1.
// Drawn round on the screen, where its centre falls: the camera is a wide
// pinhole, and a true circle of sky off to the side projects as an oval.
const DISC_GLOW: f32 = 2.2;
// Lornon, "the dark moon", shows dimmer than Liabo, "a beacon"
const LORNON_DISC: f32 = 0.3;
const TILAOK_SRGB: vec3<f32> = vec3<f32>(1.0, 0.48, 0.23);
// (how much of the pixel is the disc, how lit it is there; uv: the pixel
// from the disc's centre in its radii, u right, v up; aa: a pixel, in radii)
fn disc_at(uv: vec2<f32>, aa: f32, age: f32) -> vec2<f32> {
    let inside = 1.0 - smoothstep(1.0 - aa, 1.0 + aa, length(uv));
    // the terminator: an ellipse across the disc, cos(phase angle) wide --
    // growing, lit to its right; waning, to its left (mirrored)
    let xt = cos(6.2831853 * age) * sqrt(max(1.0 - uv.y * uv.y, 0.0));
    let s = select(-xt - uv.x, uv.x - xt, fract(age) < 0.5);
    return vec2<f32>(inside, smoothstep(-0.06, 0.06, s));
}
fn moon_discs(px: vec2<f32>) -> vec3<f32> {
    var o = vec3<f32>(0.0);
    let n_lights = u32(U.misc.w);
    for (var li = 0u; li < n_lights; li = li + 1u) {
        let Lt = U.lights[li];
        if (Lt.pos.w < 1.5 || Lt.beam.w <= 0.0) { continue; }
        let c = normalize(Lt.beam.xyz);
        if (c.z < 0.2) { continue; }
        // its radius on the screen: as drawn at the middle of the view
        let rp = U.cam.x * tan(Lt.beam.w);
        let d = (px - proj(c)) * vec2<f32>(1.0, -1.0);
        if (dot(d, d) > 64.0 * rp * rp) { continue; }
        let aa = 1.0 / max(rp, 1.0);
        let age = fract(Lt.prop.w);
        let clear = mix(0.08, 1.0, cloud_mask(vec3<f32>(0.0), Lt));
        let hue = Lt.col.rgb / max(max(Lt.col.r, Lt.col.g), max(Lt.col.b, 1e-6));
        let full = 0.5 * (1.0 - cos(6.2831853 * age));
        let dl = disc_at(d / rp, aa, age);
        let glow = exp(-max(length(d) / rp - 1.0, 0.0) * 2.5) * 0.12 * (0.2 + 0.8 * full);
        // (the one without a companion is Lornon)
        let bright = select(LORNON_DISC, 1.0, Lt.prop.w >= 1.0);
        o = o + hue * bright * (DISC_GLOW * dl.x * (dl.y + 0.03 * (1.0 - dl.y)) + glow) * clear;
        if (Lt.prop.w >= 1.0) {
            // Tilaok, "in a frenzied dance" round it
            let a = U.up.w * 0.35 + 1.3;
            let dt = disc_at((d / rp - vec2<f32>(cos(a), sin(a) * 0.45) * 2.8) / 0.3, aa / 0.3, age);
            o = o + s2l(TILAOK_SRGB) * DISC_GLOW * dt.x * (dt.y + 0.05 * (1.0 - dt.y)) * clear;
        }
    }
    return o;
}

fn backdrop_at(px: vec2<f32>, light_only: bool) -> Surf {
    let W = U.weather;
    let uv = (px - U.bgmap.xy) / U.bgmap.zw;
    let bg_a = textureSampleLevel(bg_albedo, s_lin, uv, 0.0);
    let bg_l = textureSampleLevel(bg_lit, s_lin, uv, 0.0);
    let bg_g = textureSampleLevel(bg_geo, s_near, uv, 0.0);
    let bg_n = normal_of(bg_g.xyz);
    let bg_z = bg_g.w;
    let sky = bg_z > 60.0;
    let bg_p = ray(px) * bg_z;
    let up = U.up.xyz;
    let rain = W.b.x;
    // rain darkens what it soaks
    let soak = pow(1.0 - 0.2 * rain, 2.2);
    var alb_bg = s2l(bg_a.rgb) * soak;
    var base = s2l(bg_l.rgb) * soak;
    // dark painted pixels count as water, where the scene has any
    var water = (1.0 - clamp(dot(s2l(bg_a.rgb), LUM) * 3.0, 0.0, 1.0)) * U.misc.x;
    if (W.b.w > 0.0 && !sky) {
        // snow settles on ground that faces up, in drifts laid out on the
        // camera's ground plane, not on open water
        let Pg0 = ground_under(px);
        let G = vec2<f32>(Pg0.x, dot(Pg0, ground_fwd()));
        let th = 0.12 + 0.8 * W.b.w;
        let patchy = 1.0 - smoothstep(th - 0.07, th + 0.07, fbm(G * 1.7));
        let cov = smoothstep(0.35, 0.8, dot(bg_n, up)) * patchy * (1.0 - 0.8 * water);
        base = snowed(base, alb_bg, cov);
        alb_bg = mix(alb_bg, s2l(SNOW_SRGB), cov);
        water = water * (1.0 - cov);
    }
    let wet = max(0.25 + 0.75 * water, 0.8 * rain);
    var col = select(storm_lift(base), vec3<f32>(0.0), light_only);
    if (!sky) {
        let n_lights = u32(U.misc.w);
        for (var li = 0u; li < n_lights; li = li + 1u) {
            let Lt = U.lights[li];
            if (Lt.pos.w > 1.5) {
                col = col + moon_term(bg_p, bg_n, alb_bg, Lt, Lt.surf.y, wet * Lt.surf.w, Lt.shine.y, 0.08);
                continue;
            }
            let gain = beam_gain(bg_p, Lt);
            if (gain <= 0.0) { continue; }
            col = col + light_term(bg_p, bg_n, alb_bg, Lt.pos.xyz, Lt.col.rgb, Lt.col.w * gain, Lt.fall.x, Lt.fall.y,
                                   Lt.surf.y, Lt.fall.w, wet * Lt.surf.w, Lt.shine.y, 0.08, Lt.fall.z);
        }
        col = col + alb_bg * bounce_at(bg_p, bg_n, px);
    } else {
        // Sky and far scenery: the moon lights them as open ground facing
        // up, unshadowed, under the cloud on average -- no seam where the
        // flat ground gives out at the horizon.
        let n_lights = u32(U.misc.w);
        for (var li = 0u; li < n_lights; li = li + 1u) {
            let Lt = U.lights[li];
            if (Lt.pos.w < 1.5) { continue; }
            let open = max(dot(up, Lt.pos.xyz), 0.0) * (1.0 - 0.9 * Lt.prop.x);
            col = col + alb_bg * Lt.col.rgb * Lt.col.w * (open + Lt.fall.w);
        }
        if (!light_only) {
            col = col + moon_discs(px);
        }
    }
    col = mix(col, s2l(bg_a.rgb) * 2.2, bg_l.a);
    if (!light_only) {
        if (W.a.x > 0.0) {
            // distant lightning: the haze and the cloud above the strike light up most
            let sv = px / U.size.xy;
            let far = smoothstep(10.0, 50.0, bg_z);
            let cloud = exp(-sq((sv.x - W.a.y) / 0.3)) * (1.0 - smoothstep(0.05, 0.42, sv.y));
            col = col + W.flash_tint.rgb * W.a.x * (0.004 + 0.018 * far + 0.07 * cloud * far);
        }
        let Pg = ground_under(px);
        let gz = dot(Pg, ground_fwd());
        if (rain > 0.0 && !sky && dot(bg_n, up) > 0.45 && bg_z < 22.0) {
            let sp = splash(Pg, smoothstep(0.35, 0.8, water), ground_foot(gz)) * (1.0 - smoothstep(8.0, 22.0, bg_z));
            col = col + (W.fog.rgb * 1.5 + (air_light(bg_p) - W.fog.rgb) * 2.5) * sp * (0.4 + 0.6 * rain) * W.rain_tint.rgb;
        }
        if (W.c.x > 0.0 && !sky && dot(bg_n, up) > 0.3 && bg_z < 30.0) {
            // sand streaming along the ground with the wind
            let G = vec2<f32>(Pg.x, gz);
            let g = mix(gust_at(Pg, W.c.y), 1.0, W.c.x * W.c.x);
            let st = smoothstep(0.5, 0.82, fbm(vec2<f32>((G.x - U.up.w * W.c.y * 0.8) * 1.2, G.y * 7.0)));
            let fade = 1.0 - smoothstep(0.025, 0.09, ground_foot(gz));   // too fine to see far off
            col = col + W.dust.rgb * (W.fog.rgb * 3.0 + (air_light(Pg) - W.fog.rgb) * 3.0) * st * g * (0.3 + 0.7 * W.c.x) * fade;
        }
        let m = medium(px, bg_z);
        col = col * m.w + m.xyz + weather_front(px, bg_z);
    }
    var s: Surf;
    s.col = col;
    s.a = 1.0;
    s.z = bg_z;
    s.n = select(bg_n, vec3<f32>(0.0, 0.0, -1.0), sky);
    s.alb = bg_a.rgb;
    return s;
}

fn layer_at(i: u32, px: vec2<f32>, light_only: bool) -> Surf {
    let W = U.weather;
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
    var alb = s2l(a4.rgb) * Ly.z.y;
    var base = s2l(l4.rgb) * Ly.z.y;
    if (W.b.w > 0.0) {
        // snow settles on their tops too
        let nf = fbm(ray(px).xy * z * 9.0 + f32(i) * 5.0);
        let th = 0.1 + 0.7 * W.b.w;
        let cov = smoothstep(0.45, 0.85, dot(n, U.up.xyz)) * (1.0 - smoothstep(th - 0.08, th + 0.08, nf)) * 0.85 * (1.0 - l4.a);
        base = snowed(base, alb, cov);
        alb = mix(alb, s2l(SNOW_SRGB) * Ly.z.y, cov);
    }
    var c = select(storm_lift(base), vec3<f32>(0.0), light_only);
    // rain makes them shine
    let sheen_k = 1.0 + 1.4 * W.b.x;
    let n_lights = u32(U.misc.w);
    for (var li = 0u; li < n_lights; li = li + 1u) {
        let Lt = U.lights[li];
        if (Lt.pos.w > 1.5) {
            c = c + moon_term(P, n, alb, Lt, Lt.surf.x, Lt.surf.z * sheen_k, Lt.shine.x, 0.15);
            continue;
        }
        let gain = beam_gain(P, Lt);
        if (gain <= 0.0) { continue; }
        c = c + light_term(P, n, alb, Lt.pos.xyz, Lt.col.rgb, Lt.col.w * gain, Lt.fall.x, Lt.fall.y,
                           Lt.surf.x, Lt.fall.w, Lt.surf.z * sheen_k, Lt.shine.x, 0.15, Lt.fall.z);
    }
    c = c + alb * bounce_at(P, n, px);
    if (W.a.x > 0.0 && !light_only) {
        // lightning comes from above and behind: it catches their tops and backs
        let Ls = normalize(U.up.xyz * 0.75 + vec3<f32>(0.0, 0.0, 0.65));
        c = c + alb * W.flash_tint.rgb * W.a.x * 0.10 * max(dot(n, Ls), 0.0);
    }
    var col = mix(c, s2l(a4.rgb) * Ly.z.y * 2.2, l4.a);     // glowing parts: never lit, never darkened
    if (!light_only) {
        let m = medium(px, z);
        col = col * m.w + m.xyz + weather_front(px, z);
    }
    s.col = col;
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

// The frame's direct light, for bounce (D.mode.x = 3): per texel, what the
// nearest surface there reflects of the lamps and moons (its colour times
// their light, no night, no weather), and its normal and depth (0 where
// nothing is, or sky).
struct BounceOut {
    @location(0) light: vec4<f32>,
    @location(1) surf: vec4<f32>,
};

@fragment
fn fs_bounce(in: VsOut) -> BounceOut {
    let px = in.pos.xy * D.map.xy + D.map.zw;
    let bg = backdrop_at(px, true);
    var col = bg.col;
    var z = bg.z;
    var n = bg.n;
    let n_layers = u32(U.misc.z);
    for (var i = 0u; i < n_layers; i = i + 1u) {
        let s = layer_at(i, px, true);
        if (s.a <= 0.0) { continue; }
        col = mix(col, s.col, s.a);
        // far to near: the last solid one is the nearest
        if (s.a > 0.5) {
            z = s.z;
            n = s.n;
        }
    }
    var o: BounceOut;
    o.light = vec4<f32>(col, 1.0);
    o.surf = vec4<f32>(n, select(z, 0.0, z > 60.0));
    return o;
}
