//! Kernel `qmath` — port of qcommon/q_math.c and the q_shared.h vector macros.
//!
//! Parity rule (dec008): every expression reproduces C's promotion rules. Where
//! the C code mixes `float` with a `double` literal or a libm call, the Rust
//! code widens to f64 at exactly the same point and narrows on assignment.

pub type Vec3 = [f32; 3];

pub const PITCH: usize = 0;
pub const YAW: usize = 1;
pub const ROLL: usize = 2;
/// glibc `M_PI` (a double: math.h is included before q_shared.h's fallback).
pub const M_PI: f64 = std::f64::consts::PI;
pub const VEC3_ORIGIN: Vec3 = [0.0; 3];

#[inline]
pub fn dot(a: &Vec3, b: &Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
#[inline]
pub fn sub(a: &Vec3, b: &Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
#[inline]
pub fn add(a: &Vec3, b: &Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
#[inline]
pub fn scale(v: &Vec3, s: f32) -> Vec3 {
    [v[0] * s, v[1] * s, v[2] * s]
}
/// `VectorMA(v, s, b, o)`: o = v + b*s (note operand order: b[i]*s).
#[inline]
pub fn ma(v: &Vec3, s: f32, b: &Vec3) -> Vec3 {
    [v[0] + b[0] * s, v[1] + b[1] * s, v[2] + b[2] * s]
}
#[inline]
pub fn neg(v: &Vec3) -> Vec3 {
    [-v[0], -v[1], -v[2]]
}
#[inline]
pub fn cross(a: &Vec3, b: &Vec3) -> Vec3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
/// `VectorLength`: (vec_t)sqrt(double(float sum)).
#[inline]
pub fn length(v: &Vec3) -> f32 {
    ((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]) as f64).sqrt() as f32
}
#[inline]
pub fn length_squared(v: &Vec3) -> f32 {
    v[0] * v[0] + v[1] * v[1] + v[2] * v[2]
}
#[inline]
pub fn distance(a: &Vec3, b: &Vec3) -> f32 {
    length(&sub(b, a))
}
#[inline]
pub fn compare(a: &Vec3, b: &Vec3) -> bool {
    a[0] == b[0] && a[1] == b[1] && a[2] == b[2]
}
pub fn compare_epsilon(a: &Vec3, b: &Vec3, eps: f32) -> bool {
    let d = sub(a, b);
    let d = [d[0].abs(), d[1].abs(), d[2].abs()];
    !(d[0] > eps || d[1] > eps || d[2] > eps)
}

/// `VectorNormalize`: ilength = 1/(float)sqrt(length).
pub fn normalize(v: &mut Vec3) -> f32 {
    let mut length = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if length != 0.0 {
        let ilength = 1.0f32 / ((length as f64).sqrt() as f32);
        length *= ilength;
        v[0] *= ilength;
        v[1] *= ilength;
        v[2] *= ilength;
    }
    length
}
pub fn normalize2(v: &Vec3, out: &mut Vec3) -> f32 {
    let mut length = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if length != 0.0 {
        let ilength = 1.0f32 / ((length as f64).sqrt() as f32);
        length *= ilength;
        *out = [v[0] * ilength, v[1] * ilength, v[2] * ilength];
    } else {
        *out = [0.0; 3];
    }
    length
}
pub fn normalized(v: &Vec3) -> Vec3 {
    let mut o = *v;
    normalize(&mut o);
    o
}

/// `AngleVectors`: angle = angles[i] * (M_PI*2/360) in double, stored float;
/// sin/cos in double, stored float.
pub fn angle_vectors(angles: &Vec3) -> (Vec3, Vec3, Vec3) {
    let k = M_PI * 2.0 / 360.0;
    let a = (angles[YAW] as f64 * k) as f32;
    let sy = (a as f64).sin() as f32;
    let cy = (a as f64).cos() as f32;
    let a = (angles[PITCH] as f64 * k) as f32;
    let sp = (a as f64).sin() as f32;
    let cp = (a as f64).cos() as f32;
    let a = (angles[ROLL] as f64 * k) as f32;
    let sr = (a as f64).sin() as f32;
    let cr = (a as f64).cos() as f32;
    let forward = [cp * cy, cp * sy, -sp];
    let right = [-1.0 * sr * sp * cy + -1.0 * cr * -sy, -1.0 * sr * sp * sy + -1.0 * cr * cy, -1.0 * sr * cp];
    let up = [cr * sp * cy + -sr * -sy, cr * sp * sy + -sr * cy, cr * cp];
    (forward, right, up)
}

/// `ProjectPointOnPlane` (including its double reciprocal of the dot product).
pub fn project_point_on_plane(p: &Vec3, normal: &Vec3) -> Vec3 {
    let mut inv_denom = 1.0f32 / dot(normal, normal);
    inv_denom = 1.0f32 / inv_denom;
    let d = dot(normal, p) * inv_denom;
    let n = [normal[0] * inv_denom, normal[1] * inv_denom, normal[2] * inv_denom];
    [p[0] - d * n[0], p[1] - d * n[1], p[2] - d * n[2]]
}

pub fn perpendicular_vector(src: &Vec3) -> Vec3 {
    let mut pos = 0;
    let mut minelem = 1.0f32;
    for (i, s) in src.iter().enumerate() {
        // fabs(float) is the double fabs: exact, so comparing in f32 is identical
        if s.abs() < minelem {
            pos = i;
            minelem = s.abs();
        }
    }
    let mut temp = [0.0f32; 3];
    temp[pos] = 1.0;
    let mut dst = project_point_on_plane(&temp, src);
    normalize(&mut dst);
    dst
}

/// `RotatePointAroundVector`.
pub fn rotate_point_around_vector(dir: &Vec3, point: &Vec3, degrees: f32) -> Vec3 {
    let mut cos_ia = (degrees as f64 * M_PI / 180.0f32 as f64) as f32; // DEG2RAD
    let sin_a = (cos_ia as f64).sin() as f32;
    let cos_a = (cos_ia as f64).cos() as f32;
    cos_ia = 1.0f32 - cos_a;
    let i_i_ia = dir[0] * dir[0] * cos_ia;
    let j_j_ia = dir[1] * dir[1] * cos_ia;
    let k_k_ia = dir[2] * dir[2] * cos_ia;
    let i_j_ia = dir[0] * dir[1] * cos_ia;
    let i_k_ia = dir[0] * dir[2] * cos_ia;
    let j_k_ia = dir[1] * dir[2] * cos_ia;
    let a_sin = dir[0] * sin_a;
    let b_sin = dir[1] * sin_a;
    let c_sin = dir[2] * sin_a;
    let rot = [
        [i_i_ia + cos_a, i_j_ia - c_sin, i_k_ia + b_sin],
        [i_j_ia + c_sin, j_j_ia + cos_a, j_k_ia - a_sin],
        [i_k_ia - b_sin, j_k_ia + a_sin, k_k_ia + cos_a],
    ];
    [
        point[0] * rot[0][0] + point[1] * rot[0][1] + point[2] * rot[0][2],
        point[0] * rot[1][0] + point[1] * rot[1][1] + point[2] * rot[1][2],
        point[0] * rot[2][0] + point[1] * rot[2][1] + point[2] * rot[2][2],
    ]
}

/// `vectoangles`.
pub fn vectoangles(v: &Vec3) -> Vec3 {
    let (yaw, pitch): (f32, f32);
    if v[1] == 0.0 && v[0] == 0.0 {
        yaw = 0.0;
        pitch = if v[2] > 0.0 { 90.0 } else { 270.0 };
    } else {
        let mut y: f32 = if v[0] != 0.0 {
            ((v[1] as f64).atan2(v[0] as f64) * 180.0 / M_PI) as f32
        } else if v[1] > 0.0 {
            90.0
        } else {
            270.0
        };
        if y < 0.0 {
            y += 360.0;
        }
        yaw = y;
        let forward = ((v[0] * v[0] + v[1] * v[1]) as f64).sqrt() as f32;
        let mut p = ((v[2] as f64).atan2(forward as f64) * 180.0 / M_PI) as f32;
        if p < 0.0 {
            p += 360.0;
        }
        pitch = p;
    }
    [-pitch, yaw, 0.0]
}

/// `AxisToAngles`.
pub fn axis_to_angles(axis: &[Vec3; 3]) -> Vec3 {
    let (yaw, pitch);
    let mut roll = 0.0f32;
    if axis[0][1] == 0.0 && axis[0][0] == 0.0 {
        yaw = 0.0;
        pitch = if axis[0][2] > 0.0 { 90.0 } else { 270.0 };
    } else {
        let mut y: f32 = if axis[0][0] != 0.0 {
            ((axis[0][1] as f64).atan2(axis[0][0] as f64) * 180.0 / M_PI) as f32
        } else if axis[0][1] > 0.0 {
            90.0
        } else {
            270.0
        };
        if y < 0.0 {
            y += 360.0;
        }
        yaw = y;
        let length1 = ((axis[0][0] * axis[0][0] + axis[0][1] * axis[0][1]) as f64).sqrt() as f32;
        let mut p = ((axis[0][2] as f64).atan2(length1 as f64) * 180.0 / M_PI) as f32;
        if p < 0.0 {
            p += 360.0;
        }
        pitch = p;
        roll = ((axis[1][2] as f64).atan2(axis[2][2] as f64) * 180.0 / M_PI) as f32;
        if roll < 0.0 {
            roll += 360.0;
        }
    }
    [-pitch, yaw, roll]
}

/// `AnglesToAxis`: axis[1] = vec3_origin - right.
pub fn angles_to_axis(angles: &Vec3) -> [Vec3; 3] {
    let (f, r, u) = angle_vectors(angles);
    [f, sub(&VEC3_ORIGIN, &r), u]
}

pub fn angle_subtract(a1: f32, a2: f32) -> f32 {
    let mut a = a1 - a2;
    while a > 180.0 {
        a -= 360.0;
    }
    while a < -180.0 {
        a += 360.0;
    }
    a
}

/// `AngleNormalize360`: (360.0/65536) * ((int)(angle*(65536/360.0)) & 65535).
pub fn angle_normalize360(angle: f32) -> f32 {
    ((360.0f64 / 65536.0) * (((angle as f64 * (65536.0 / 360.0)) as i32 & 65535) as f64)) as f32
}
pub fn angle_normalize180(angle: f32) -> f32 {
    let mut a = angle_normalize360(angle);
    if a as f64 > 180.0 {
        a = (a as f64 - 360.0) as f32;
    }
    a
}

/// `ANGLE2SHORT` for a float argument: ((int)(x*65536/360) & 65535), float math.
#[inline]
pub fn angle2short_f(x: f32) -> i32 {
    ((x * 65536.0 / 360.0) as i32) & 65535
}
/// `ANGLE2SHORT` for a double argument.
#[inline]
pub fn angle2short_d(x: f64) -> i32 {
    ((x * 65536.0 / 360.0) as i32) & 65535
}
/// `SHORT2ANGLE`: x * (360.0/65536) in double.
#[inline]
pub fn short2angle(x: i32) -> f64 {
    x as f64 * (360.0 / 65536.0)
}
/// `RAD2DEG` on a double: ((a)*180.0f)/M_PI.
#[inline]
pub fn rad2deg(a: f64) -> f64 {
    (a * 180.0f32 as f64) / M_PI
}

pub fn byte_to_dir(b: i32) -> Vec3 {
    use crate::gen::bytedirs::{BYTEDIRS, BYTEDIRS_COUNT};
    if b < 0 || b as usize >= BYTEDIRS_COUNT {
        return VEC3_ORIGIN;
    }
    BYTEDIRS[b as usize].dir
}
pub fn dir_to_byte(dir: &Vec3) -> i32 {
    use crate::gen::bytedirs::BYTEDIRS;
    let mut bestd = 0.0f32;
    let mut best = 0;
    for (i, d) in BYTEDIRS.iter().enumerate() {
        let x = dot(dir, &d.dir);
        if x > bestd {
            bestd = x;
            best = i as i32;
        }
    }
    best
}

pub fn clear_bounds() -> (Vec3, Vec3) {
    ([99999.0; 3], [-99999.0; 3])
}
pub fn add_point_to_bounds(v: &Vec3, mins: &mut Vec3, maxs: &mut Vec3) {
    for i in 0..3 {
        if v[i] < mins[i] {
            mins[i] = v[i];
        }
        if v[i] > maxs[i] {
            maxs[i] = v[i];
        }
    }
}
pub fn bounds_intersect(mins: &Vec3, maxs: &Vec3, mins2: &Vec3, maxs2: &Vec3) -> bool {
    !(maxs[0] < mins2[0] || maxs[1] < mins2[1] || maxs[2] < mins2[2] || mins[0] > maxs2[0] || mins[1] > maxs2[1] || mins[2] > maxs2[2])
}
