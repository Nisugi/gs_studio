// vellum_light's relight on the GPU (relight_gpu.rs): the same per-pixel
// kernel (`light_pixel`), the same whole-image frame (`LightFrame::measure`:
// the 4th / 96th percentile of OKLab L over pixels with alpha >= 32, and the
// rows they fill), and the same byte-space cross-fade of time-of-day
// keyframes (`blend_keyframes`). Every pass writes RGBA8, so each rounds to a
// byte where the CPU does.

const LUM: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);
const BINS: u32 = 4096u;

fn s2l(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}
fn l2s(c0: vec3<f32>) -> vec3<f32> {
    let c = max(c0, vec3<f32>(0.0));
    return select(1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055, 12.92 * c, c <= vec3<f32>(0.0031308));
}
fn cbrt(x: f32) -> f32 {
    return sign(x) * pow(abs(x), 1.0 / 3.0);
}
fn lin_to_oklab(c: vec3<f32>) -> vec3<f32> {
    let l = cbrt(0.41222147 * c.r + 0.53633255 * c.g + 0.051445995 * c.b);
    let m = cbrt(0.2119035 * c.r + 0.6806995 * c.g + 0.10739696 * c.b);
    let s = cbrt(0.08830246 * c.r + 0.28171885 * c.g + 0.6299787 * c.b);
    return vec3<f32>(0.21045426 * l + 0.7936178 * m - 0.004072047 * s,
                     1.9779985 * l - 2.4285922 * m + 0.4505937 * s,
                     0.025904037 * l + 0.78277177 * m - 0.80867577 * s);
}
fn oklab_to_lin(lab: vec3<f32>) -> vec3<f32> {
    let l = pow3(lab.x + 0.39633778 * lab.y + 0.21580376 * lab.z);
    let m = pow3(lab.x - 0.105561346 * lab.y - 0.06385417 * lab.z);
    let s = pow3(lab.x - 0.08948418 * lab.y - 1.2914855 * lab.z);
    return vec3<f32>(4.0767417 * l - 3.3077116 * m + 0.23096994 * s,
                     -1.268438 * l + 2.6097574 * m - 0.34131938 * s,
                     -0.0041960863 * l - 0.7034186 * m + 1.7076147 * s);
}
fn pow3(x: f32) -> f32 { return x * x * x; }
fn in_gamut(c: vec3<f32>) -> bool {
    return all(c >= vec3<f32>(-1e-4)) && all(c <= vec3<f32>(1.0 + 1e-4));
}
// chroma only: L and hue held, 14 bisection steps
fn gamut_clip(lab: vec3<f32>) -> vec3<f32> {
    if (in_gamut(oklab_to_lin(lab))) { return lab; }
    let c = length(lab.yz);
    if (c < 1e-9) { return lab; }
    let uab = lab.yz / c;
    var lo = 0.0;
    var hi = c;
    for (var i = 0; i < 14; i = i + 1) {
        let mid = 0.5 * (lo + hi);
        if (in_gamut(oklab_to_lin(vec3<f32>(lab.x, uab * mid)))) { lo = mid; } else { hi = mid; }
    }
    return vec3<f32>(lab.x, uab * lo);
}
fn smooth01(x0: f32) -> f32 {
    let x = clamp(x0, 0.0, 1.0);
    return x * x * (3.0 - 2.0 * x);
}

// ---- the frame: a histogram of L and the content rows, then percentiles

@group(0) @binding(0) var stats_src: texture_2d<f32>;
// BINS bins, then count, top row, bottom row
@group(0) @binding(1) var<storage, read_write> hist: array<atomic<u32>>;
// l_lo, l_hi, y_top, y_bot
@group(0) @binding(2) var<storage, read_write> frame_out: array<f32>;

@compute @workgroup_size(16, 16)
fn cs_stats(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(stats_src);
    if (id.x >= size.x || id.y >= size.y) { return; }
    let q = textureLoad(stats_src, vec2<i32>(id.xy), 0);
    if (q.a * 255.0 < 31.5) { return; }
    let l = lin_to_oklab(s2l(q.rgb)).x;
    let bin = u32(clamp(l, 0.0, 0.99999994) * f32(BINS));
    atomicAdd(&hist[bin], 1u);
    atomicAdd(&hist[BINS], 1u);
    atomicMin(&hist[BINS + 1u], id.y);
    atomicMax(&hist[BINS + 2u], id.y);
}

// the value of rank `r` (0-based) in the sorted Ls: its bin, and where in
// the bin by its rank there (spread evenly across it)
fn value_at(r: u32) -> f32 {
    var cum = 0u;
    for (var b = 0u; b < BINS; b = b + 1u) {
        let n = atomicLoad(&hist[b]);
        if (cum + n > r) {
            return (f32(b) + (f32(r - cum) + 0.5) / f32(n)) / f32(BINS);
        }
        cum = cum + n;
    }
    return 1.0;
}

// numpy's "linear" percentile, as `percentile_sorted`
fn percentile(p: f32, n: u32) -> f32 {
    let idx = p / 100.0 * f32(n - 1u);
    let i = u32(floor(idx));
    let f = idx - floor(idx);
    let vi = value_at(i);
    if (i + 1u < n) { return vi + (value_at(i + 1u) - vi) * f; }
    return vi;
}

@compute @workgroup_size(1)
fn cs_frame() {
    let n = atomicLoad(&hist[BINS]);
    let size = textureDimensions(stats_src);
    if (n == 0u) {
        frame_out[0] = 0.0;
        frame_out[1] = 1.0;
        frame_out[2] = 0.0;
        frame_out[3] = f32(size.y) - 1.0;
        return;
    }
    frame_out[0] = percentile(4.0, n);
    frame_out[1] = percentile(96.0, n);
    frame_out[2] = f32(atomicLoad(&hist[BINS + 1u]));
    frame_out[3] = f32(atomicLoad(&hist[BINS + 2u]));
}

// ---- the light: light_pixel

struct P {
    key_tint: vec4<f32>,
    ambient_tint: vec4<f32>,
    bounce_tint: vec4<f32>,
    fog_lab: vec4<f32>,
    a: vec4<f32>,         // exposure, white, chroma, strength
    b: vec4<f32>,         // bounce_strength, night, haze, 0
};

@group(0) @binding(0) var<uniform> pp: P;
@group(0) @binding(1) var<storage, read> frame: array<f32>;
@group(0) @binding(2) var light_src: texture_2d<f32>;

fn light_pixel(lin: vec3<f32>, y: f32) -> vec3<f32> {
    let l_lo = frame[0];
    let l_hi = frame[1];
    let y_top = frame[2];
    let y_bot = frame[3];
    let l_src = lin_to_oklab(lin).x;
    // 1 litness
    let t = smooth01((l_src - l_lo) / max(l_hi - l_lo, 1e-3));
    // 2-3 light colour + bounce
    let yrel = clamp((y - y_top) / max(y_bot - y_top, 1.0), 0.0, 1.0);
    let wb = pp.b.x * yrel * yrel * (1.0 - t);
    var light = pp.ambient_tint.rgb * (1.0 - t) + pp.key_tint.rgb * t;
    light = light * (1.0 - wb) + pp.bounce_tint.rgb * wb;
    light = 1.0 + pp.a.w * (light - 1.0);
    var o = lin * light * pp.a.x;
    // 5 highlight shoulder
    let white = pp.a.y;
    if (white > 1.0) {
        let yl = max(dot(o, LUM), 1e-6);
        o = o * ((1.0 + yl / (white * white)) / (1.0 + yl));
    }
    var lab = lin_to_oklab(max(o, vec3<f32>(0.0)));
    // 6 saturation
    lab = vec3<f32>(lab.x, lab.yz * pp.a.z);
    // 7 night
    let night = pp.b.y;
    if (night > 0.0) {
        let c = length(lab.yz);
        var warm = 0.0;
        if (c > 1e-9) { warm = max((lab.y * 0.7660444 + lab.z * 0.6427876) / c, 0.0); }
        warm = warm * smooth01(c / 0.04);
        let k = 1.0 - 0.55 * night;
        lab = vec3<f32>(lab.x * (1.0 - 0.18 * night * warm), lab.yz * k);
    }
    // 8 haze
    let haze = pp.b.z;
    if (haze > 0.0) { lab = lab * (1.0 - haze) + pp.fog_lab.xyz * haze; }
    // 9 gamut clip
    return clamp(oklab_to_lin(gamut_clip(lab)), vec3<f32>(0.0), vec3<f32>(1.0));
}

@vertex fn vs_full(@builtin(vertex_index) k: u32) -> @builtin(position) vec4<f32> {
    let c = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(c[k], 0.0, 1.0);
}

@fragment fn fs_light(@builtin(position) fc: vec4<f32>) -> @location(0) vec4<f32> {
    let px = vec2<i32>(fc.xy);
    let q = textureLoad(light_src, px, 0);
    if (q.a <= 0.0) { return q; }
    // bytes in, as the LUT reads them (the texel is byte / 255)
    let o = light_pixel(s2l(q.rgb), f32(px.y));
    return vec4<f32>(l2s(o), q.a);
}

// ---- the cross-fade: keyframes' bytes weighted and summed

struct F {
    w: vec4<f32>,         // weights of k0, k1, k2; how many
};

@group(0) @binding(0) var<uniform> ff: F;
@group(0) @binding(1) var k0: texture_2d<f32>;
@group(0) @binding(2) var k1: texture_2d<f32>;
@group(0) @binding(3) var k2: texture_2d<f32>;

@fragment fn fs_fade(@builtin(position) fc: vec4<f32>) -> @location(0) vec4<f32> {
    let px = vec2<i32>(fc.xy);
    let a = textureLoad(k0, px, 0);
    var rgb = a.rgb * ff.w.x;
    if (ff.w.w > 1.5) { rgb = rgb + textureLoad(k1, px, 0).rgb * ff.w.y; }
    if (ff.w.w > 2.5) { rgb = rgb + textureLoad(k2, px, 0).rgb * ff.w.z; }
    return vec4<f32>(rgb, a.a);
}
