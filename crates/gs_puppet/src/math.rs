//! Tiny 3-vector / 3x3 helpers. Right-handed: x = creature's left (screen
//! right in a front view), y = up, z = creature's front.

pub type V3 = [f32; 3];
pub type M3 = [[f32; 3]; 3];

pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
pub fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub fn len(a: V3) -> f32 {
    dot(a, a).sqrt()
}
pub fn lerp(a: V3, b: V3, t: f32) -> V3 {
    add(scale(a, 1.0 - t), scale(b, t))
}
/// Unit vector, or `fallback` for a (near) zero vector.
pub fn norm_or(a: V3, fallback: V3) -> V3 {
    let l = len(a);
    if l > 1e-6 {
        scale(a, 1.0 / l)
    } else {
        fallback
    }
}

pub const IDENTITY: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

pub fn mul_mv(m: &M3, v: V3) -> V3 {
    [dot(m[0], v), dot(m[1], v), dot(m[2], v)]
}
pub fn mul_mm(a: &M3, b: &M3) -> M3 {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}
pub fn rot_x(deg: f32) -> M3 {
    let (s, c) = deg.to_radians().sin_cos();
    [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]]
}
pub fn rot_y(deg: f32) -> M3 {
    let (s, c) = deg.to_radians().sin_cos();
    [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]]
}
pub fn rot_z(deg: f32) -> M3 {
    let (s, c) = deg.to_radians().sin_cos();
    [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]]
}
/// Euler degrees `[x, y, z]` -> `Rx * Rz * Ry`: twist about the limb's own
/// (rest-vertical) axis first, then spread sideways, then swing fore/aft.
pub fn euler(deg: [f32; 3]) -> M3 {
    mul_mm(&rot_x(deg[0]), &mul_mm(&rot_z(deg[2]), &rot_y(deg[1])))
}

/// Rigid transform `p -> r * p + t`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Xf {
    pub r: M3,
    pub t: V3,
}

impl Xf {
    pub const IDENTITY: Xf = Xf { r: IDENTITY, t: [0.0; 3] };

    pub fn about(pivot: V3, r: M3) -> Xf {
        Xf { r, t: sub(pivot, mul_mv(&r, pivot)) }
    }
    pub fn point(&self, p: V3) -> V3 {
        add(mul_mv(&self.r, p), self.t)
    }
    pub fn dir(&self, d: V3) -> V3 {
        mul_mv(&self.r, d)
    }
    /// `self` after `inner`.
    pub fn then_inner(&self, inner: &Xf) -> Xf {
        Xf { r: mul_mm(&self.r, &inner.r), t: add(mul_mv(&self.r, inner.t), self.t) }
    }
}
