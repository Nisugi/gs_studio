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

/// Rotation of `deg` degrees about the unit vector `axis` (Rodrigues).
pub fn axis_angle(axis: V3, deg: f32) -> M3 {
    let (s, c) = deg.to_radians().sin_cos();
    let [x, y, z] = axis;
    let t = 1.0 - c;
    [
        [t * x * x + c, t * x * y - s * z, t * x * z + s * y],
        [t * x * y + s * z, t * y * y + c, t * y * z - s * x],
        [t * x * z - s * y, t * y * z + s * x, t * z * z + c],
    ]
}

/// Unit quaternion `[w, x, y, z]`.
pub type Quat = [f32; 4];

pub const QUAT_IDENTITY: Quat = [1.0, 0.0, 0.0, 0.0];

pub fn quat_from_m3(m: &M3) -> Quat {
    let tr = m[0][0] + m[1][1] + m[2][2];
    let q = if tr > 0.0 {
        let s = (tr + 1.0).sqrt() * 2.0;
        [0.25 * s, (m[2][1] - m[1][2]) / s, (m[0][2] - m[2][0]) / s, (m[1][0] - m[0][1]) / s]
    } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
        let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
        [(m[2][1] - m[1][2]) / s, 0.25 * s, (m[0][1] + m[1][0]) / s, (m[0][2] + m[2][0]) / s]
    } else if m[1][1] > m[2][2] {
        let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
        [(m[0][2] - m[2][0]) / s, (m[0][1] + m[1][0]) / s, 0.25 * s, (m[1][2] + m[2][1]) / s]
    } else {
        let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
        [(m[1][0] - m[0][1]) / s, (m[0][2] + m[2][0]) / s, (m[1][2] + m[2][1]) / s, 0.25 * s]
    };
    quat_norm(q)
}

pub fn quat_norm(q: Quat) -> Quat {
    let l = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if l > 1e-9 && l.is_finite() {
        [q[0] / l, q[1] / l, q[2] / l, q[3] / l]
    } else {
        QUAT_IDENTITY
    }
}

pub fn quat_to_m3(q: Quat) -> M3 {
    let [w, x, y, z] = q;
    [
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - w * z), 2.0 * (x * z + w * y)],
        [2.0 * (x * y + w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - w * x)],
        [2.0 * (x * z - w * y), 2.0 * (y * z + w * x), 1.0 - 2.0 * (x * x + y * y)],
    ]
}

/// Shortest-arc spherical interpolation.
pub fn slerp(a: Quat, b: Quat, t: f32) -> Quat {
    let mut d = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    let mut b = b;
    if d < 0.0 {
        b = [-b[0], -b[1], -b[2], -b[3]];
        d = -d;
    }
    if d > 0.9995 {
        return quat_norm([0, 1, 2, 3].map(|i| a[i] + (b[i] - a[i]) * t));
    }
    let th = d.clamp(-1.0, 1.0).acos();
    let (sa, sb) = (((1.0 - t) * th).sin() / th.sin(), (t * th).sin() / th.sin());
    quat_norm([0, 1, 2, 3].map(|i| a[i] * sa + b[i] * sb))
}

/// The same rotation on the other side of the body (reflected in x = 0).
pub fn quat_mirror(q: Quat) -> Quat {
    [q[0], q[1], -q[2], -q[3]]
}
