//! Small vector, quaternion and matrix set the battlefield needs. No math crate is available
//! offline, so only what the renderer uses is written here. Right-handed, +Y up, column-major
//! matrices that upload to WebGL unchanged.

use std::ops::{Add, Mul, Neg, Sub};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);
    pub const UP: Self = Self::new(0.0, 1.0, 0.0);

    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    pub fn cross(self, other: Self) -> Self {
        Self::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    /// The unit vector, or zero when the length is too small to divide by.
    pub fn normalized(self) -> Self {
        let length = self.length();
        if length < 1e-9 {
            Self::ZERO
        } else {
            self * (1.0 / length)
        }
    }

    pub fn lerp(self, other: Self, t: f32) -> Self {
        self + (other - self) * t
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

impl Add for Vec3 {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }
}

impl Sub for Vec3 {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }
}

impl Mul<f32> for Vec3 {
    type Output = Self;
    fn mul(self, factor: f32) -> Self {
        Self::new(self.x * factor, self.y * factor, self.z * factor)
    }
}

impl Neg for Vec3 {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y, -self.z)
    }
}

pub fn lerp(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}

pub fn clamp_unit(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

/// Hermite ease with zero slope at both ends.
pub fn ease_in_out(t: f32) -> f32 {
    let t = clamp_unit(t);
    t * t * (3.0 - 2.0 * t)
}

/// Rotation as x, y, z, w. Kept normalised by the constructors that take raw splat data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Quat {
    pub const IDENTITY: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    };

    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    /// Normalises, falling back to identity for a zero or non-finite input.
    pub fn normalized(self) -> Self {
        let length = (self.x * self.x + self.y * self.y + self.z * self.z + self.w * self.w).sqrt();
        if !length.is_finite() || length < 1e-9 {
            return Self::IDENTITY;
        }
        let inverse = 1.0 / length;
        Self::new(
            self.x * inverse,
            self.y * inverse,
            self.z * inverse,
            self.w * inverse,
        )
    }

    pub fn from_axis_angle_y(radians: f32) -> Self {
        let half = radians * 0.5;
        Self::new(0.0, half.sin(), 0.0, half.cos())
    }

    /// Rotation by `other` first, then by `self`.
    pub fn compose(self, other: Self) -> Self {
        Self::new(
            self.w * other.x + self.x * other.w + self.y * other.z - self.z * other.y,
            self.w * other.y - self.x * other.z + self.y * other.w + self.z * other.x,
            self.w * other.z + self.x * other.y - self.y * other.x + self.z * other.w,
            self.w * other.w - self.x * other.x - self.y * other.y - self.z * other.z,
        )
    }

    /// Row-major 3x3 rotation matrix of the (assumed unit) quaternion.
    pub fn to_matrix3(self) -> [[f32; 3]; 3] {
        let Self { x, y, z, w } = self;
        [
            [
                1.0 - 2.0 * (y * y + z * z),
                2.0 * (x * y - w * z),
                2.0 * (x * z + w * y),
            ],
            [
                2.0 * (x * y + w * z),
                1.0 - 2.0 * (x * x + z * z),
                2.0 * (y * z - w * x),
            ],
            [
                2.0 * (x * z - w * y),
                2.0 * (y * z + w * x),
                1.0 - 2.0 * (x * x + y * y),
            ],
        ]
    }

    pub fn rotate(self, vector: Vec3) -> Vec3 {
        let m = self.to_matrix3();
        Vec3::new(
            m[0][0] * vector.x + m[0][1] * vector.y + m[0][2] * vector.z,
            m[1][0] * vector.x + m[1][1] * vector.y + m[1][2] * vector.z,
            m[2][0] * vector.x + m[2][1] * vector.y + m[2][2] * vector.z,
        )
    }
}

/// Column-major 4x4 matrix: element (row, column) lives at `column * 4 + row`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4(pub [f32; 16]);

impl Mat4 {
    pub const IDENTITY: Self = Self([
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]);

    pub fn at(&self, row: usize, column: usize) -> f32 {
        self.0[column * 4 + row]
    }

    pub fn translation(offset: Vec3) -> Self {
        let mut matrix = Self::IDENTITY;
        matrix.0[12] = offset.x;
        matrix.0[13] = offset.y;
        matrix.0[14] = offset.z;
        matrix
    }

    pub fn mul(&self, other: &Self) -> Self {
        let mut out = [0.0; 16];
        for column in 0..4 {
            for row in 0..4 {
                out[column * 4 + row] = (0..4).map(|k| self.at(row, k) * other.at(k, column)).sum();
            }
        }
        Self(out)
    }

    /// Vertical field of view in radians; depth maps to the WebGL clip range.
    pub fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Self {
        let focal = 1.0 / (fov_y * 0.5).tan();
        let mut out = [0.0; 16];
        out[0] = focal / aspect;
        out[5] = focal;
        out[10] = (far + near) / (near - far);
        out[11] = -1.0;
        out[14] = 2.0 * far * near / (near - far);
        Self(out)
    }

    pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Self {
        let forward = (target - eye).normalized();
        let side = forward.cross(up).normalized();
        let true_up = side.cross(forward);
        Self([
            side.x,
            true_up.x,
            -forward.x,
            0.0,
            side.y,
            true_up.y,
            -forward.y,
            0.0,
            side.z,
            true_up.z,
            -forward.z,
            0.0,
            -side.dot(eye),
            -true_up.dot(eye),
            forward.dot(eye),
            1.0,
        ])
    }

    pub fn transform_point(&self, point: Vec3) -> Vec3 {
        let m = &self.0;
        Vec3::new(
            m[0] * point.x + m[4] * point.y + m[8] * point.z + m[12],
            m[1] * point.x + m[5] * point.y + m[9] * point.z + m[13],
            m[2] * point.x + m[6] * point.y + m[10] * point.z + m[14],
        )
    }

    /// Projects a world point to normalised device coordinates plus the clip `w`.
    /// Returns `None` for points on or behind the camera plane.
    pub fn project(&self, point: Vec3) -> Option<(f32, f32, f32)> {
        let m = &self.0;
        let w = m[3] * point.x + m[7] * point.y + m[11] * point.z + m[15];
        if w <= 1e-6 {
            return None;
        }
        let x = m[0] * point.x + m[4] * point.y + m[8] * point.z + m[12];
        let y = m[1] * point.x + m[5] * point.y + m[9] * point.z + m[13];
        Some((x / w, y / w, w))
    }

    /// The upper-left 3x3 as row-major rows, used to rotate directions.
    pub fn upper_left(&self) -> [[f32; 3]; 3] {
        [
            [self.at(0, 0), self.at(0, 1), self.at(0, 2)],
            [self.at(1, 0), self.at(1, 1), self.at(1, 2)],
            [self.at(2, 0), self.at(2, 1), self.at(2, 2)],
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn look_at_puts_the_target_in_front_on_the_view_axis() {
        let view = Mat4::look_at(
            Vec3::new(0.0, 3.0, 10.0),
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::UP,
        );
        let target = view.transform_point(Vec3::ZERO);
        assert!(close(target.x, 0.0));
        assert!(target.z < 0.0);
        assert!(close(target.length(), (9.0f32 + 100.0).sqrt()));
    }

    #[test]
    fn perspective_maps_near_and_far_to_the_clip_range() {
        let projection = Mat4::perspective(1.0, 16.0 / 9.0, 0.5, 100.0);
        let near = projection.project(Vec3::new(0.0, 0.0, -0.5)).unwrap();
        let far = projection.project(Vec3::new(0.0, 0.0, -100.0)).unwrap();
        assert!(close(near.2, 0.5));
        assert!(close(near.1, 0.0));
        assert!(close(far.2, 100.0));
        let m = &projection.0;
        let depth = |z: f32| (m[10] * z + m[14]) / (m[11] * z);
        assert!(close(depth(-0.5), -1.0));
        assert!(close(depth(-100.0), 1.0));
    }

    #[test]
    fn matrix_multiplication_applies_the_right_operand_first() {
        let moved = Mat4::translation(Vec3::new(1.0, 2.0, 3.0));
        let combined = moved.mul(&Mat4::translation(Vec3::new(10.0, 0.0, 0.0)));
        let point = combined.transform_point(Vec3::ZERO);
        assert_eq!(point, Vec3::new(11.0, 2.0, 3.0));
    }

    #[test]
    fn project_rejects_points_behind_the_camera() {
        let projection = Mat4::perspective(1.0, 1.0, 0.1, 50.0);
        assert!(projection.project(Vec3::new(0.0, 0.0, 5.0)).is_none());
    }

    #[test]
    fn quaternion_rotation_turns_a_quarter_about_y() {
        let turn = Quat::from_axis_angle_y(std::f32::consts::FRAC_PI_2);
        let rotated = turn.rotate(Vec3::new(1.0, 0.0, 0.0));
        assert!(close(rotated.x, 0.0));
        assert!(close(rotated.z, -1.0));
    }

    #[test]
    fn quaternion_normalisation_recovers_identity_from_degenerate_input() {
        assert_eq!(Quat::new(0.0, 0.0, 0.0, 0.0).normalized(), Quat::IDENTITY);
        assert_eq!(
            Quat::new(f32::NAN, 0.0, 0.0, 1.0).normalized(),
            Quat::IDENTITY
        );
        let unit = Quat::new(0.0, 0.0, 0.0, 4.0).normalized();
        assert!(close(unit.w, 1.0));
    }

    #[test]
    fn ease_in_out_is_bounded_and_flat_at_the_ends() {
        assert_eq!(ease_in_out(-1.0), 0.0);
        assert_eq!(ease_in_out(2.0), 1.0);
        assert!(ease_in_out(0.01) < 0.001);
        assert!(close(ease_in_out(0.5), 0.5));
    }
}
